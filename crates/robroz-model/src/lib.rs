//! The model a bro thinks with. Any OpenAI-compatible `/chat/completions`.
//!
//! Configured from the environment:
//! - `ROBROZ_MODEL_BASE_URL`, e.g. `https://api.openai.com/v1`
//! - `ROBROZ_MODEL`, e.g. `gpt-4o-mini`
//! - `ROBROZ_MODEL_KEY` or `ROBROZ_MODEL_KEY_FILE` (not both)
//!
//! No key means no model: bros keep echoing the prompt, which is what CI and
//! e2e use. The key never appears in `Debug`, errors or logs.

use std::fmt;
use std::time::Duration;

pub const ENV_BASE_URL: &str = "ROBROZ_MODEL_BASE_URL";
pub const ENV_MODEL: &str = "ROBROZ_MODEL";
pub const ENV_KEY: &str = "ROBROZ_MODEL_KEY";
pub const ENV_KEY_FILE: &str = "ROBROZ_MODEL_KEY_FILE";

/// Told to every bro before the task prompt.
pub const SYSTEM_PROMPT: &str = "你是 robroz 里的一个 bro，替用户完成交给你的任务。直接给出结果，简洁，用用户的语言。";

/// A model call. `Send + Sync` so the pump thread can own one.
pub trait Model: Send + Sync {
    /// `memory_section` is appended to [`SYSTEM_PROMPT`] when present.
    fn reply(&self, prompt: &str, memory_section: Option<&str>) -> Result<String, ModelError>;
}

/// Why a call failed. Never carries the key or the request headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError(pub String);

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
    pub key: ModelKey,
}

/// Read the config with `get` (env lookup) and `read_file` (for the key file).
/// `Ok(None)`: no key, so no model. Errors name variables, never values.
pub fn config_from(
    get: impl Fn(&str) -> Option<String>,
    read_file: impl Fn(&str) -> std::io::Result<String>,
) -> Result<Option<ModelConfig>, String> {
    let set = |name: &str| get(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let key = match (set(ENV_KEY), set(ENV_KEY_FILE)) {
        (Some(_), Some(_)) => return Err(format!("{ENV_KEY} 和 {ENV_KEY_FILE} 只能设一个")),
        (Some(key), None) => key,
        (None, Some(file)) => read_file(&file)
            .map_err(|err| format!("{ENV_KEY_FILE} 读不出来：{err}"))?
            .trim()
            .to_string(),
        (None, None) => return Ok(None),
    };
    if key.is_empty() {
        return Err(format!("{ENV_KEY_FILE} 是空的"));
    }
    if key.chars().any(char::is_whitespace) {
        return Err("模型密钥里不能有空白".into());
    }
    let base_url = set(ENV_BASE_URL).ok_or_else(|| format!("设了密钥，还要设 {ENV_BASE_URL}"))?;
    if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
        return Err(format!("{ENV_BASE_URL} 要以 http:// 或 https:// 开头"));
    }
    let model = set(ENV_MODEL).ok_or_else(|| format!("设了密钥，还要设 {ENV_MODEL}"))?;
    Ok(Some(ModelConfig {
        base_url: base_url.trim_end_matches('/').to_string(),
        model,
        key: ModelKey(key),
    }))
}

/// [`config_from`] on the process environment.
pub fn config_from_env() -> Result<Option<ModelConfig>, String> {
    config_from(|name| std::env::var(name).ok(), |path: &str| std::fs::read_to_string(path))
}

/// The system message: the fixed opening, plus the memory paragraph when there is one.
pub fn system_content(memory_section: Option<&str>) -> String {
    match memory_section {
        Some(section) if !section.is_empty() => format!("{SYSTEM_PROMPT}\n\n{section}"),
        _ => SYSTEM_PROMPT.to_string(),
    }
}

/// The `/chat/completions` body.
pub fn request_body(model: &str, prompt: &str, memory_section: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_content(memory_section) },
            { "role": "user", "content": prompt },
        ],
    })
}

/// `choices[0].message.content`, trimmed. Empty is an error.
pub fn parse_reply(body: &str) -> Result<String, ModelError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| ModelError("模型返回的不是 JSON".into()))?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .ok_or_else(|| ModelError("模型返回里没有 choices[0].message.content".into()))?;
    if content.is_empty() {
        return Err(ModelError("模型返回了空内容".into()));
    }
    Ok(content.to_string())
}

/// Calls an OpenAI-compatible service over HTTP(S).
pub struct OpenAiCompat {
    config: ModelConfig,
    agent: ureq::Agent,
}

impl OpenAiCompat {
    pub fn new(config: ModelConfig) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { config, agent }
    }

    pub fn config(&self) -> &ModelConfig {
        &self.config
    }
}

impl Model for OpenAiCompat {
    fn reply(&self, prompt: &str, memory_section: Option<&str>) -> Result<String, ModelError> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let body = request_body(&self.config.model, prompt, memory_section).to_string();
        let mut response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.config.key.expose()))
            .header("Content-Type", "application/json")
            .send(body.as_bytes())
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
            return Err(ModelError(format!("模型服务返回 {status}")));
        }
        parse_reply(&text)
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
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| map.get(name).cloned()
    }

    fn no_file(_: &str) -> std::io::Result<String> {
        Err(std::io::Error::other("no file"))
    }

    const SECRET: &str = "sk-test-SECRET123";

    #[test]
    fn no_key_means_no_model() {
        assert_eq!(config_from(env(&[]), no_file), Ok(None));
        assert_eq!(config_from(env(&[(ENV_MODEL, "m"), (ENV_KEY, "  ")]), no_file), Ok(None));
    }

    #[test]
    fn config_reads_key_or_file_and_never_shows_the_key() {
        let base = [(ENV_BASE_URL, "https://api.example.com/v1/"), (ENV_MODEL, "m1")];
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
        assert_eq!(cfg.key.expose(), SECRET);

        let mut both = with_key.clone();
        both.push((ENV_KEY_FILE, "/x"));
        let errors = [
            config_from(env(&both), no_file).unwrap_err(),
            config_from(env(&[(ENV_KEY, SECRET), (ENV_MODEL, "m")]), no_file).unwrap_err(),
            config_from(env(&[(ENV_KEY, SECRET), (ENV_BASE_URL, "https://x")]), no_file).unwrap_err(),
            config_from(env(&[(ENV_KEY, SECRET), (ENV_BASE_URL, "ftp://x"), (ENV_MODEL, "m")]), no_file).unwrap_err(),
            config_from(env(&[(ENV_KEY, "sk a"), (ENV_BASE_URL, "https://x"), (ENV_MODEL, "m")]), no_file).unwrap_err(),
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
        assert_eq!(body["messages"][1], serde_json::json!({ "role": "user", "content": "写周报" }));
        let with = request_body("m1", "写周报", Some("你记得这些：\n[habit] 周五交周报"));
        assert_eq!(
            with["messages"][0]["content"],
            format!("{SYSTEM_PROMPT}\n\n你记得这些：\n[habit] 周五交周报")
        );
        assert_eq!(request_body("m1", "写周报", Some(""))["messages"][0]["content"], SYSTEM_PROMPT);
        assert_eq!(parse_reply(r#"{"choices":[{"message":{"content":"  好的\n"}}]}"#), Ok("好的".into()));
        assert!(parse_reply("not json").is_err());
        assert!(parse_reply(r#"{"choices":[]}"#).is_err());
        assert!(parse_reply(r#"{"choices":[{"message":{"content":"  "}}]}"#).is_err());
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
        OpenAiCompat::new(ModelConfig { base_url: url.into(), model: "m1".into(), key: ModelKey(SECRET.into()) })
    }

    #[test]
    fn calls_chat_completions_with_bearer_key() {
        let (url, rx) = fake_server(200, r#"{"choices":[{"message":{"content":"周报写好了"}}]}"#);
        assert_eq!(model_at(&url).reply("写周报", None), Ok("周报写好了".into()));
        let request = rx.recv().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions "), "{request}");
        assert!(request.contains(&format!("Bearer {SECRET}")));
        assert!(request.contains("写周报"));
    }

    #[test]
    fn http_errors_do_not_leak_the_key() {
        let (url, _rx) = fake_server(401, r#"{"error":"bad key sk-test-SECRET123"}"#);
        let err = model_at(&url).reply("hi", None).unwrap_err();
        assert_eq!(err.0, "模型服务返回 401");
        let dead = model_at("http://127.0.0.1:9").reply("hi", None).unwrap_err();
        assert!(!dead.0.contains(SECRET), "{dead}");
    }
}
