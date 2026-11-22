//! Model settings: OpenAI-compatible `/chat/completions`, stored like search.
//!
//! Settings win over `ROBROZ_MODEL_*` environment variables, field by field.
//! The key never appears in logs, errors, or `GET /api/settings/model`.
//! `ROBROZ_MODEL=echo` (no key or base URL needed) is a test switch that
//! keeps the runner echoing the prompt.

use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{ModelSettings, ModelSettingsSource, ModelTestResult};
use robroz_model::{ModelConfig, OpenAiCompat, ENV_BASE_URL, ENV_KEY, ENV_KEY_FILE, ENV_MODEL};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;

use crate::settings::{as_opt_opt, env_get, last4, setting, upsert_setting};
use crate::{error_response, lock_db, AppState};

/// How long the test endpoint waits for the model service.
pub const MODEL_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Written as the task result, and shown on the delivery, when no model is set
/// and echo mode is off.
pub const NOT_CONFIGURED: &str = "还没设置模型";

const SETTING_BASE_URL: &str = "model.base_url";
const SETTING_MODEL: &str = "model.model";
const SETTING_KEY: &str = "model.key";

const ECHO: &str = "echo";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub key: Option<String>,
}

impl EffectiveConfig {
    fn is_empty(&self) -> bool {
        self.base_url.is_none() && self.model.is_none() && self.key.is_none()
    }

    fn public(&self, source: Option<ModelSettingsSource>) -> ModelSettings {
        ModelSettings {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            has_key: self.key.is_some(),
            key_last4: self.key.as_deref().and_then(last4),
            source,
        }
    }

    fn redact(&self, err: &str) -> String {
        let mut out = err.to_string();
        if let Some(key) = &self.key {
            if !key.is_empty() {
                out = out.replace(key, "***");
            }
        }
        out
    }

    /// A complete OpenAI-compatible config, or `None` if any field is missing.
    pub fn live(&self) -> Result<Option<ModelConfig>, String> {
        match (&self.base_url, &self.model, &self.key) {
            (Some(base_url), Some(model), Some(key)) => {
                Ok(Some(ModelConfig::new(base_url, model, key)?))
            }
            _ => Ok(None),
        }
    }
}

fn env_key() -> Result<Option<String>, String> {
    match (env_get(ENV_KEY), env_get(ENV_KEY_FILE)) {
        (Some(_), Some(_)) => Err(format!("{ENV_KEY} 和 {ENV_KEY_FILE} 只能设一个")),
        (Some(key), None) => {
            if key.chars().any(char::is_whitespace) {
                Err("模型密钥里不能有空白".into())
            } else {
                Ok(Some(key))
            }
        }
        (None, Some(file)) => {
            let raw = std::fs::read_to_string(&file)
                .map_err(|err| format!("{ENV_KEY_FILE} 读不出来：{err}"))?;
            let key = raw.trim().to_string();
            if key.is_empty() {
                return Err(format!("{ENV_KEY_FILE} 是空的"));
            }
            if key.chars().any(char::is_whitespace) {
                return Err("模型密钥里不能有空白".into());
            }
            Ok(Some(key))
        }
        (None, None) => Ok(None),
    }
}

fn clean_base_url(raw: &str) -> Option<String> {
    let url = raw.trim().trim_end_matches('/').to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Settings (when present) win over environment variables, field by field.
pub fn effective_config(conn: &Connection) -> Result<EffectiveConfig, String> {
    let base_url = setting(conn, SETTING_BASE_URL)
        .map_err(|err| err.to_string())?
        .or_else(|| env_get(ENV_BASE_URL))
        .and_then(|u| clean_base_url(&u));
    let model = setting(conn, SETTING_MODEL)
        .map_err(|err| err.to_string())?
        .or_else(|| env_get(ENV_MODEL));
    let key = match setting(conn, SETTING_KEY).map_err(|err| err.to_string())? {
        Some(key) => Some(key),
        None => env_key()?,
    };
    Ok(EffectiveConfig {
        base_url,
        model,
        key,
    })
}

fn settings_are_set(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(setting(conn, SETTING_BASE_URL)?.is_some()
        || setting(conn, SETTING_MODEL)?.is_some()
        || setting(conn, SETTING_KEY)?.is_some())
}

fn public_settings(conn: &Connection) -> Result<ModelSettings, String> {
    let cfg = effective_config(conn)?;
    if cfg.is_empty() {
        return Ok(ModelSettings {
            base_url: None,
            model: None,
            has_key: false,
            key_last4: None,
            source: None,
        });
    }
    let source = Some(if settings_are_set(conn).map_err(|err| err.to_string())? {
        ModelSettingsSource::Settings
    } else {
        ModelSettingsSource::Env
    });
    Ok(cfg.public(source))
}

/// A complete config the pump can call, or `None` if the model is not set up.
pub fn live_model(conn: &Connection) -> Result<Option<ModelConfig>, String> {
    effective_config(conn)?.live()
}

/// Test-only switch: `ROBROZ_MODEL=echo` with no complete model config.
/// The runner keeps echoing the prompt. Settings that form a complete model
/// take priority over this.
pub fn echo_mode(conn: &Connection) -> bool {
    match live_model(conn) {
        Ok(None) => env_get(ENV_MODEL).is_some_and(|m| m.eq_ignore_ascii_case(ECHO)),
        _ => false,
    }
}

/// Reject a conflicting or invalid model key in the environment. Incomplete
/// configs (missing URL or model name) are allowed so the settings page can
/// fill the rest.
pub fn env_key_conflict() -> Result<(), String> {
    env_key().map(|_| ())
}

pub async fn get_model(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    match public_settings(&conn) {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, &err),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PutModel {
    /// Missing: leave as-is. `null`: clear. A string: set.
    #[serde(default, deserialize_with = "as_opt_opt")]
    base_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "as_opt_opt")]
    model: Option<Option<String>>,
    #[serde(default, deserialize_with = "as_opt_opt")]
    key: Option<Option<String>>,
}

pub async fn put_model(
    State(state): State<AppState>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let Json(raw) = match body {
        Ok(body) => body,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求体不是 JSON"),
    };
    let put: PutModel = match serde_json::from_value(raw) {
        Ok(put) => put,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "请求体无效"),
    };
    if let Err(err) = validate_put(&put) {
        return error_response(StatusCode::BAD_REQUEST, &err);
    }
    let conn = lock_db(&state);
    if let Err(err) = apply_put(&conn, &put) {
        return error_response(StatusCode::BAD_REQUEST, &err);
    }
    match public_settings(&conn) {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, &err),
    }
}

fn validate_put(put: &PutModel) -> Result<(), String> {
    if let Some(Some(url)) = &put.base_url {
        let url = url.trim();
        if url.is_empty() {
            return Err("请填写地址".into());
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("地址要以 http:// 或 https:// 开头".into());
        }
    }
    if let Some(Some(model)) = &put.model {
        if model.trim().is_empty() {
            return Err("请填写模型名".into());
        }
    }
    if let Some(Some(key)) = &put.key {
        if key.trim().is_empty() {
            return Err("密钥不能为空".into());
        }
        if key.chars().any(char::is_whitespace) {
            return Err("密钥里不能有空白".into());
        }
    }
    Ok(())
}

fn apply_put(conn: &Connection, put: &PutModel) -> Result<(), String> {
    if let Some(url) = &put.base_url {
        let value = url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string());
        upsert_setting(conn, SETTING_BASE_URL, value.as_deref()).map_err(|e| e.to_string())?;
    }
    if let Some(model) = &put.model {
        upsert_setting(
            conn,
            SETTING_MODEL,
            model.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(key) = &put.key {
        upsert_setting(conn, SETTING_KEY, key.as_deref()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn test_model(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    let result = match effective_config(&conn) {
        Err(err) => {
            return error_response(StatusCode::BAD_REQUEST, &err);
        }
        Ok(cfg) => match cfg.live() {
            Err(err) => ModelTestResult {
                ok: false,
                error: Some(cfg.redact(&err)),
            },
            Ok(None) => ModelTestResult {
                ok: false,
                error: Some(NOT_CONFIGURED.into()),
            },
            Ok(Some(live)) => {
                drop(conn);
                match OpenAiCompat::test_connection(&live, MODEL_TEST_TIMEOUT) {
                    Ok(()) => ModelTestResult {
                        ok: true,
                        error: None,
                    },
                    Err(err) => ModelTestResult {
                        ok: false,
                        error: Some(format!("连不上：{}", cfg.redact(&err.to_string()))),
                    },
                }
            }
        },
    };
    (StatusCode::OK, Json(result)).into_response()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::mpsc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::*;
    use crate::settings::test_env;

    fn temp_path(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn app() -> (PathBuf, Router, crate::Host) {
        let db = std::env::temp_dir().join(format!("robroz-model-{}.db", uuid::Uuid::new_v4()));
        let volumes = temp_path("robroz-model-vol");
        let host = crate::Host::open(&db, &volumes, "robroz-runner:local").unwrap();
        (db, host.router(), host)
    }

    async fn http(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(req).await.expect("oneshot");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                panic!("response was not json: {}", String::from_utf8_lossy(&bytes))
            })
        };
        (status, value)
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    fn json_req(method: &str, uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn post(uri: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    /// One-shot HTTP server that records the request and returns a chat reply.
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

    #[tokio::test]
    async fn env_only_reports_source_env() {
        let (_db, app, _host) = app();
        let (status, body) = http(&app, get("/api/settings/model")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["baseUrl"].is_null(), "{body}");
        assert!(body["model"].is_null(), "{body}");
        assert_eq!(body["hasKey"], false, "{body}");
        assert!(body["keyLast4"].is_null(), "{body}");
        assert!(
            body.get("source").is_none(),
            "unconfigured omits source: {body}"
        );

        let _guard = test_env::set(&[
            (ENV_BASE_URL, "https://api.example.com/v1/"),
            (ENV_MODEL, "gpt-test"),
            (ENV_KEY, "sk-envXXXX"),
        ]);
        let (status, body) = http(&app, get("/api/settings/model")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["baseUrl"], "https://api.example.com/v1");
        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["hasKey"], true);
        assert_eq!(body["keyLast4"], "XXXX");
        assert_eq!(body["source"], "env");
        assert!(!body.to_string().contains("sk-envXXXX"), "{body}");
    }

    #[tokio::test]
    async fn put_saves_and_reports_source_settings() {
        let (_db, app, _host) = app();
        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/model",
                json!({
                    "baseUrl": "https://api.example.com/v1/",
                    "model": "gpt-saved",
                    "key": "sk-setABCD",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["baseUrl"], "https://api.example.com/v1");
        assert_eq!(body["model"], "gpt-saved");
        assert_eq!(body["hasKey"], true);
        assert_eq!(body["keyLast4"], "ABCD");
        assert_eq!(body["source"], "settings");
        assert!(!body.to_string().contains("sk-setABCD"), "{body}");

        let (status, body) = http(&app, get("/api/settings/model")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["source"], "settings");
        assert_eq!(body["keyLast4"], "ABCD");
    }

    #[tokio::test]
    async fn put_omits_key_leaves_it_and_null_clears() {
        let (_db, app, _host) = app();
        let (status, _) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/model",
                json!({
                    "baseUrl": "https://api.example.com/v1",
                    "model": "m1",
                    "key": "sk-keepZZZZ",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = http(
            &app,
            json_req("PUT", "/api/settings/model", json!({ "model": "m2" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["model"], "m2");
        assert_eq!(body["hasKey"], true, "omitted key must be kept: {body}");
        assert_eq!(body["keyLast4"], "ZZZZ");

        let (status, body) = http(
            &app,
            json_req("PUT", "/api/settings/model", json!({ "key": null })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["hasKey"], false, "{body}");
        assert!(body["keyLast4"].is_null(), "{body}");
        assert_eq!(body["model"], "m2");
        assert_eq!(body["source"], "settings");
    }

    #[tokio::test]
    async fn invalid_base_url_is_400() {
        let (_db, app, _host) = app();
        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/model",
                json!({ "baseUrl": "ftp://x" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "地址要以 http:// 或 https:// 开头");
        let (status, body) = http(
            &app,
            json_req("PUT", "/api/settings/model", json!({ "baseUrl": "  " })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "请填写地址");
    }

    #[tokio::test]
    async fn unconfigured_delivery_carries_needs_setup_model() {
        let (_db, app, host) = app();
        let volumes = host.volumes_root.clone();
        let (status, group) = http(
            &app,
            json_req("POST", "/api/groups", json!({ "name": "模型" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group}");
        let group_id = group["id"].as_str().unwrap().to_string();
        let (status, body) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/members"),
                json!({ "botId": "bot-ada", "name": "Ada" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, task) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/tasks"),
                json!({ "assigneeBotId": "bot-ada", "prompt": "写一句欢迎语" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        let task_id = task["id"].as_str().unwrap().to_string();

        let mut sandbox = crate::sandbox::FakeSandbox::new();
        host.pump_once(&mut sandbox).unwrap();
        sandbox.finish(0);
        host.pump_once(&mut sandbox).unwrap();

        let (status, events) = http(&app, get(&format!("/api/groups/{group_id}/events"))).await;
        assert_eq!(status, StatusCode::OK, "{events}");
        let delivery = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "delivery")
            .expect("delivery");
        assert_eq!(delivery["needsSetup"], "model", "{delivery}");
        let result = std::fs::read_to_string(
            volumes
                .join(&group_id)
                .join(format!("tasks/{task_id}/result.txt")),
        )
        .unwrap();
        assert_eq!(result, NOT_CONFIGURED, "{result}");
        assert_ne!(result, "写一句欢迎语", "must not echo the prompt");
    }

    #[tokio::test]
    async fn saved_settings_are_used_by_the_next_task() {
        let (url, rx) = fake_server(
            200,
            r#"{"choices":[{"message":{"content":"新配置已生效"}}]}"#,
        );
        let (_db, app, host) = app();
        let volumes = host.volumes_root.clone();
        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/model",
                json!({
                    "baseUrl": url,
                    "model": "saved-model",
                    "key": "sk-liveWXYZ",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["source"], "settings");

        let (status, group) = http(
            &app,
            json_req("POST", "/api/groups", json!({ "name": "模型" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group}");
        let group_id = group["id"].as_str().unwrap().to_string();
        let (status, body) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/members"),
                json!({ "botId": "bot-ada", "name": "Ada" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, task) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/tasks"),
                json!({ "assigneeBotId": "bot-ada", "prompt": "你好" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        let task_id = task["id"].as_str().unwrap().to_string();

        let mut sandbox = crate::sandbox::FakeSandbox::new();
        host.pump_once(&mut sandbox).unwrap();
        sandbox.finish(0);
        host.pump_once(&mut sandbox).unwrap();

        let request = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("model was called");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-livewxyz"),
            "{request}"
        );
        assert!(
            request.contains("\"model\":\"saved-model\"")
                || request.contains("\"model\": \"saved-model\""),
            "{request}"
        );
        let result = std::fs::read_to_string(
            volumes
                .join(&group_id)
                .join(format!("tasks/{task_id}/result.txt")),
        )
        .unwrap();
        assert_eq!(result, "新配置已生效", "{result}");
        let (status, events) = http(&app, get(&format!("/api/groups/{group_id}/events"))).await;
        assert_eq!(status, StatusCode::OK, "{events}");
        let delivery = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "delivery")
            .expect("delivery");
        assert!(
            delivery.get("needsSetup").is_none() || delivery["needsSetup"].is_null(),
            "{delivery}"
        );
    }

    #[tokio::test]
    async fn test_uses_a_local_fake_server() {
        let (url, rx) = fake_server(200, r#"{"choices":[{"message":{"content":"pong"}}]}"#);
        let (_db, app, _host) = app();
        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/model",
                json!({
                    "baseUrl": url,
                    "model": "m-test",
                    "key": "sk-test1111",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = http(&app, post("/api/settings/model/test")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["ok"], true, "{body}");
        let request = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("test called the server");
        assert!(request.contains("/chat/completions"), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-test1111"),
            "{request}"
        );
        assert!(
            !body.to_string().contains("sk-test1111"),
            "test response must not echo the key: {body}"
        );
    }

    #[tokio::test]
    async fn echo_mode_still_echoes_the_prompt() {
        let (_db, app, host) = app();
        let volumes = host.volumes_root.clone();
        let _guard = test_env::set(&[(ENV_MODEL, "echo")]);
        let (status, body) = http(&app, get("/api/settings/model")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["model"], "echo");
        assert_eq!(body["hasKey"], false);
        assert!(body["baseUrl"].is_null());
        assert_eq!(body["source"], "env");

        let (status, group) = http(
            &app,
            json_req("POST", "/api/groups", json!({ "name": "回显" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group}");
        let group_id = group["id"].as_str().unwrap().to_string();
        let (status, body) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/members"),
                json!({ "botId": "bot-ada", "name": "Ada" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, task) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/tasks"),
                json!({ "assigneeBotId": "bot-ada", "prompt": "原样回去" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        let task_id = task["id"].as_str().unwrap().to_string();

        let mut sandbox = crate::sandbox::FakeSandbox::new();
        host.pump_once(&mut sandbox).unwrap();
        sandbox.finish(0);
        host.pump_once(&mut sandbox).unwrap();

        let result = std::fs::read_to_string(
            volumes
                .join(&group_id)
                .join(format!("tasks/{task_id}/result.txt")),
        )
        .unwrap();
        assert_eq!(result, "原样回去", "{result}");
        let (status, events) = http(&app, get(&format!("/api/groups/{group_id}/events"))).await;
        assert_eq!(status, StatusCode::OK, "{events}");
        let delivery = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "delivery")
            .expect("delivery");
        assert!(
            delivery.get("needsSetup").is_none() || delivery["needsSetup"].is_null(),
            "{delivery}"
        );
    }
}
