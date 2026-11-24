//! The model a bro thinks with. Any OpenAI-compatible `/chat/completions`.
//!
//! Configured from the environment:
//! - `ROBROZ_MODEL_BASE_URL`, e.g. `https://api.openai.com/v1`
//! - `ROBROZ_MODEL`, e.g. `gpt-4o-mini`
//! - `ROBROZ_MODEL_KEY` or `ROBROZ_MODEL_KEY_FILE` (not both; optional, e.g. local Ollama)
//!
//! No complete config (base URL and model name) means no model. The key is
//! optional, for example when talking to a local Ollama. The host then either
//! asks the user to set one up, or, when `ROBROZ_MODEL=echo`, keeps echoing
//! the prompt for CI and e2e. The key never appears in `Debug`, errors or
//! logs.

mod chat;

pub use chat::{chat_body, parse_turn, AssistantTurn, ChatMessage, ToolCall, ToolDefinition};

use std::fmt;
use std::time::Duration;

pub const ENV_BASE_URL: &str = "ROBROZ_MODEL_BASE_URL";
pub const ENV_MODEL: &str = "ROBROZ_MODEL";
pub const ENV_KEY: &str = "ROBROZ_MODEL_KEY";
pub const ENV_KEY_FILE: &str = "ROBROZ_MODEL_KEY_FILE";

/// Told to every bro before the task prompt.
pub const SYSTEM_PROMPT: &str =
    "You are a bro in robroz. Do the task the user gave you and give the result directly, briefly. \
     Reply in the language the user wrote in.";

/// A model call. `Send + Sync` so the pump thread can own one.
pub trait Model: Send + Sync {
    /// `memory_section` is appended to [`SYSTEM_PROMPT`] when present.
    fn reply(&self, prompt: &str, memory_section: Option<&str>) -> Result<String, ModelError>;

    /// One turn of a tool-calling conversation. `tools` is omitted from the
    /// request when empty. Providers that cannot do function calling return a
    /// clear "not supported" error; [`OpenAiCompat`] implements this.
    fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<AssistantTurn, ModelError> {
        let _ = (messages, tools);
        Err(ModelError("此模型不支持工具调用".into()))
    }
}

/// Why a call failed. Never carries the key or the request headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError(pub String);

impl ModelError {
    fn from_http_status(status: u16) -> Self {
        Self(format!("模型服务返回 {status}"))
    }

    /// Status code when the provider answered with a non-success HTTP status.
    pub fn http_status(&self) -> Option<u16> {
        self.0.strip_prefix("模型服务返回 ")?.parse().ok()
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A key that prints as `***`.
#[derive(Clone, PartialEq, Eq)]
pub struct ModelKey(String);

impl ModelKey {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ModelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    /// Without the trailing slash.
    pub base_url: String,
    pub model: String,
    pub key: Option<ModelKey>,
}

impl ModelConfig {
    /// Trim a trailing slash on `base_url`. An empty key is allowed (no
    /// `Authorization` header). A non-empty key must not contain whitespace.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, String> {
        let key = key.into();
        let key = if key.is_empty() {
            None
        } else if key.chars().any(char::is_whitespace) {
            return Err("模型密钥里不能有空白".into());
        } else {
            Some(ModelKey(key))
        };
        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err("地址要以 http:// 或 https:// 开头".into());
        }
        let model = model.into().trim().to_string();
        if model.is_empty() {
            return Err("请填写模型名".into());
        }
        Ok(Self {
            base_url,
            model,
            key,
        })
    }

    pub fn last4(&self) -> Option<String> {
        let key = self.key.as_ref()?.expose();
        let n = key.chars().count();
        Some(key.chars().skip(n.saturating_sub(4)).collect())
    }
}

/// Read the config with `get` (env lookup) and `read_file` (for the key file).
/// `Ok(None)`: no complete config (base URL and model name). A key is optional.
/// `ROBROZ_MODEL=echo` with no base URL is not a model, so the host can keep
/// echoing for CI and e2e. Errors name variables, never values.
pub fn config_from(
    get: impl Fn(&str) -> Option<String>,
    read_file: impl Fn(&str) -> std::io::Result<String>,
) -> Result<Option<ModelConfig>, String> {
    let set = |name: &str| {
        get(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let key = match (set(ENV_KEY), set(ENV_KEY_FILE)) {
        (Some(_), Some(_)) => return Err(format!("{ENV_KEY} 和 {ENV_KEY_FILE} 只能设一个")),
        (Some(key), None) => Some(key),
        (None, Some(file)) => {
            let key = read_file(&file)
                .map_err(|err| format!("{ENV_KEY_FILE} 读不出来：{err}"))?
                .trim()
                .to_string();
            if key.is_empty() {
                return Err(format!("{ENV_KEY_FILE} 是空的"));
            }
            Some(key)
        }
        (None, None) => None,
    };
    if let Some(key) = &key {
        if key.chars().any(char::is_whitespace) {
            return Err("模型密钥里不能有空白".into());
        }
    }
    let base_url = set(ENV_BASE_URL);
    let model = set(ENV_MODEL);
    match (base_url, model, key) {
        (Some(base_url), Some(model), key) => {
            if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
                return Err(format!("{ENV_BASE_URL} 要以 http:// 或 https:// 开头"));
            }
            Ok(Some(ModelConfig {
                base_url: base_url.trim_end_matches('/').to_string(),
                model,
                key: key.map(ModelKey),
            }))
        }
        (None, _, Some(_)) => Err(format!("设了密钥，还要设 {ENV_BASE_URL}")),
        (Some(_), None, Some(_)) => Err(format!("设了密钥，还要设 {ENV_MODEL}")),
        _ => Ok(None),
    }
}

/// [`config_from`] on the process environment.
pub fn config_from_env() -> Result<Option<ModelConfig>, String> {
    config_from(
        |name| std::env::var(name).ok(),
        |path: &str| std::fs::read_to_string(path),
    )
}

/// The system message: the fixed opening, plus the memory paragraph when there is one.
pub fn system_content(memory_section: Option<&str>) -> String {
    match memory_section {
        Some(section) if !section.is_empty() => format!("{SYSTEM_PROMPT}\n\n{section}"),
        _ => SYSTEM_PROMPT.to_string(),
    }
}

/// The `/chat/completions` body for a single prompt, with no tools.
pub fn request_body(model: &str, prompt: &str, memory_section: Option<&str>) -> serde_json::Value {
    chat_body(
        model,
        &[
            ChatMessage::System(system_content(memory_section)),
            ChatMessage::User(prompt.to_string()),
        ],
        &[],
    )
}

/// `choices[0].message.content`, trimmed. Empty is an error. A tool call in a
/// reply that was not asked for is also an error.
pub fn parse_reply(body: &str) -> Result<String, ModelError> {
    match parse_turn(body)? {
        AssistantTurn::Text(text) => Ok(text),
        AssistantTurn::ToolCalls(_) => Err(ModelError("模型在没有工具时返回了工具调用".into())),
    }
}

/// Calls an OpenAI-compatible service over HTTP(S).
pub struct OpenAiCompat {
    config: ModelConfig,
    agent: ureq::Agent,
}

impl OpenAiCompat {
    pub fn new(config: ModelConfig) -> Self {
        Self::with_timeout(config, Duration::from_secs(120))
    }

    pub fn with_timeout(config: ModelConfig, timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { config, agent }
    }

    /// Shortest `/chat/completions` request, for `POST /api/settings/model/test`.
    pub fn test_connection(config: &ModelConfig, timeout: Duration) -> Result<(), ModelError> {
        let client = Self::with_timeout(config.clone(), timeout);
        let body = serde_json::json!({
            "model": config.model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1
        });
        let _ = client.post(body)?;
        Ok(())
    }

    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    fn post(&self, body: serde_json::Value) -> Result<String, ModelError> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let mut request = self
            .agent
            .post(&url)
            .header("Content-Type", "application/json");
        if let Some(key) = &self.config.key {
            request = request.header("Authorization", &format!("Bearer {}", key.expose()));
        }
        let mut response = request
            .send(body.to_string().as_bytes())
            // ureq transport errors carry no headers, so no key.
            .map_err(|err| ModelError(format!("连不上模型服务：{err}")))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|err| ModelError(format!("读模型返回失败：{err}")))?;
        if !(200..300).contains(&status) {
            return Err(ModelError::from_http_status(status));
        }
        Ok(text)
    }
}

impl Model for OpenAiCompat {
    fn reply(&self, prompt: &str, memory_section: Option<&str>) -> Result<String, ModelError> {
        parse_reply(&self.post(request_body(&self.config.model, prompt, memory_section))?)
    }

    fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<AssistantTurn, ModelError> {
        parse_turn(&self.post(chat_body(&self.config.model, messages, tools))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn no_file(_: &str) -> std::io::Result<String> {
        Err(std::io::Error::other("no file"))
    }

    const SECRET: &str = "sk-test-SECRET123";

    #[test]
    fn no_key_means_no_model() {
        assert_eq!(config_from(env(&[]), no_file), Ok(None));
        assert_eq!(
            config_from(env(&[(ENV_MODEL, "m"), (ENV_KEY, "  ")]), no_file),
            Ok(None)
        );
        // Echo mode in CI and e2e: model name only, no base URL, no key.
        assert_eq!(config_from(env(&[(ENV_MODEL, "echo")]), no_file), Ok(None));
    }

    #[test]
    fn new_accepts_an_empty_key_and_rejects_whitespace() {
        let cfg = ModelConfig::new("https://api.example.com/v1/", "m1", "").unwrap();
        assert_eq!(cfg.base_url, "https://api.example.com/v1");
        assert_eq!(cfg.model, "m1");
        assert!(cfg.key.is_none());
        assert_eq!(cfg.last4(), None);
        assert_eq!(
            ModelConfig::new("https://api.example.com/v1", "m1", "sk a").unwrap_err(),
            "模型密钥里不能有空白"
        );
        let with_key = ModelConfig::new("https://api.example.com/v1", "m1", SECRET).unwrap();
        assert_eq!(with_key.last4().as_deref(), Some("T123"));
        assert!(!format!("{with_key:?}").contains(SECRET));
    }

    #[test]
    fn base_and_model_without_key_is_a_model() {
        let cfg = config_from(
            env(&[
                (ENV_BASE_URL, "https://api.example.com/v1/"),
                (ENV_MODEL, "m1"),
            ]),
            no_file,
        )
        .unwrap()
        .unwrap();
        assert_eq!(cfg.base_url, "https://api.example.com/v1");
        assert_eq!(cfg.model, "m1");
        assert!(cfg.key.is_none());
        assert_eq!(cfg.last4(), None);
    }

    #[test]
    fn config_reads_key_or_file_and_never_shows_the_key() {
        let base = [
            (ENV_BASE_URL, "https://api.example.com/v1/"),
            (ENV_MODEL, "m1"),
        ];
        let mut with_key = base.to_vec();
        with_key.push((ENV_KEY, SECRET));
        let cfg = config_from(env(&with_key), no_file).unwrap().unwrap();
        assert_eq!(cfg.base_url, "https://api.example.com/v1");
        assert_eq!(cfg.model, "m1");
        assert!(!format!("{cfg:?}").contains(SECRET), "Debug hides the key");

        let mut with_file = base.to_vec();
        with_file.push((ENV_KEY_FILE, "/run/secrets/model"));
        let cfg = config_from(env(&with_file), |p| {
            assert_eq!(p, "/run/secrets/model");
            Ok(format!("{SECRET}\n"))
        })
        .unwrap()
        .unwrap();
        assert_eq!(cfg.key.as_ref().unwrap().expose(), SECRET);

        let mut both = with_key.clone();
        both.push((ENV_KEY_FILE, "/x"));
        let errors = [
            config_from(env(&both), no_file).unwrap_err(),
            config_from(env(&[(ENV_KEY, SECRET), (ENV_MODEL, "m")]), no_file).unwrap_err(),
            config_from(
                env(&[(ENV_KEY, SECRET), (ENV_BASE_URL, "https://x")]),
                no_file,
            )
            .unwrap_err(),
            config_from(
                env(&[
                    (ENV_KEY, SECRET),
                    (ENV_BASE_URL, "ftp://x"),
                    (ENV_MODEL, "m"),
                ]),
                no_file,
            )
            .unwrap_err(),
            config_from(
                env(&[
                    (ENV_KEY, "sk a"),
                    (ENV_BASE_URL, "https://x"),
                    (ENV_MODEL, "m"),
                ]),
                no_file,
            )
            .unwrap_err(),
            config_from(env(&with_file), |_| Ok("\n".into())).unwrap_err(),
        ];
        for err in errors {
            assert!(!err.contains(SECRET), "{err}");
        }
    }

    #[test]
    fn body_and_reply_shapes() {
        let body = request_body("m1", "写周报", None);
        assert_eq!(body["model"], "m1");
        assert_eq!(body["messages"][0]["content"], SYSTEM_PROMPT);
        assert_eq!(
            body["messages"][1],
            serde_json::json!({ "role": "user", "content": "写周报" })
        );
        let with = request_body("m1", "写周报", Some("You remember:\n[habit] 周五交周报"));
        assert_eq!(
            with["messages"][0]["content"],
            format!("{SYSTEM_PROMPT}\n\nYou remember:\n[habit] 周五交周报")
        );
        assert_eq!(
            request_body("m1", "写周报", Some(""))["messages"][0]["content"],
            SYSTEM_PROMPT
        );
        assert_eq!(
            parse_reply(r#"{"choices":[{"message":{"content":"  好的\n"}}]}"#),
            Ok("好的".into())
        );
        assert!(parse_reply("not json").is_err());
        assert!(parse_reply(r#"{"choices":[]}"#).is_err());
        assert!(parse_reply(r#"{"choices":[{"message":{"content":"  "}}]}"#).is_err());
        assert!(body.get("tools").is_none());
        assert!(parse_reply(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"echo","arguments":"{}"}}]}}]}"#
        )
        .is_err());
    }

    /// One-shot HTTP server: returns `(status, body)` and hands back the request.
    fn fake_server(status: u16, body: &'static str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut buf = vec![0; len];
            reader.read_exact(&mut buf).unwrap();
            head.push_str(&String::from_utf8(buf).unwrap());
            tx.send(head).unwrap();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (url, rx)
    }

    fn model_at(url: &str) -> OpenAiCompat {
        OpenAiCompat::new(ModelConfig {
            base_url: url.into(),
            model: "m1".into(),
            key: Some(ModelKey(SECRET.into())),
        })
    }

    fn model_at_keyless(url: &str) -> OpenAiCompat {
        OpenAiCompat::new(ModelConfig::new(url, "m1", "").unwrap())
    }

    #[test]
    fn calls_chat_completions_with_bearer_key() {
        let (url, rx) = fake_server(200, r#"{"choices":[{"message":{"content":"周报写好了"}}]}"#);
        assert_eq!(
            model_at(&url).reply("写周报", None),
            Ok("周报写好了".into())
        );
        let request = rx.recv().unwrap();
        assert!(
            request.starts_with("POST /v1/chat/completions "),
            "{request}"
        );
        assert!(request.contains(&format!("Bearer {SECRET}")));
        assert!(request.contains("写周报"));
    }

    #[test]
    fn no_key_omits_the_authorization_header() {
        let (url, rx) = fake_server(200, r#"{"choices":[{"message":{"content":"好"}}]}"#);
        assert_eq!(
            model_at_keyless(&url).reply("写周报", None),
            Ok("好".into())
        );
        let request = rx.recv().unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(!lower.contains("authorization"), "{request}");
        assert!(!lower.contains("bearer"), "{request}");
        assert!(
            request.starts_with("POST /v1/chat/completions "),
            "{request}"
        );
    }

    #[test]
    fn http_errors_do_not_leak_the_key() {
        let (url, _rx) = fake_server(401, r#"{"error":"bad key sk-test-SECRET123"}"#);
        let err = model_at(&url).reply("hi", None).unwrap_err();
        assert_eq!(err.0, "模型服务返回 401");
        let dead = model_at("http://127.0.0.1:9")
            .reply("hi", None)
            .unwrap_err();
        assert!(!dead.0.contains(SECRET), "{dead}");
    }

    #[test]
    fn chat_sends_tools_and_parses_tool_calls() {
        let (url, rx) = fake_server(
            200,
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"echo","arguments":"{\"text\":\"hi\"}"}}]}}]}"#,
        );
        let tools = [ToolDefinition {
            name: "echo".into(),
            description: "原样返回".into(),
            parameters: serde_json::json!({ "type": "object" }),
        }];
        let turn = model_at(&url)
            .chat(&[ChatMessage::User("叫一下 echo".into())], &tools)
            .unwrap();
        match turn {
            AssistantTurn::ToolCalls(calls) => {
                assert_eq!(calls[0].id, "c1");
                assert_eq!(calls[0].name, "echo");
            }
            other => panic!("{other:?}"),
        }
        let request = rx.recv().unwrap();
        assert!(request.contains(r#""tools""#), "{request}");
        assert!(
            request.contains(r#""name":"echo""#) || request.contains(r#""name": "echo""#),
            "{request}"
        );
        assert!(request.contains(&format!("Bearer {SECRET}")));
    }

    #[test]
    fn reply_request_does_not_include_tools() {
        let (url, rx) = fake_server(200, r#"{"choices":[{"message":{"content":"好"}}]}"#);
        assert_eq!(model_at(&url).reply("写周报", None), Ok("好".into()));
        let request = rx.recv().unwrap();
        assert!(!request.contains(r#""tools""#), "{request}");
    }

    #[test]
    fn default_chat_is_clearly_unsupported() {
        struct OnlyReply;
        impl Model for OnlyReply {
            fn reply(&self, prompt: &str, _: Option<&str>) -> Result<String, ModelError> {
                Ok(prompt.into())
            }
        }
        let err = OnlyReply.chat(&[], &[]).unwrap_err();
        assert_eq!(err.0, "此模型不支持工具调用");
    }
}
