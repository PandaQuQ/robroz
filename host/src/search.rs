//! Web search on the host: Brave and SearXNG, plus `web_search` / `web_fetch`.
//!
//! Search and fetch run here, not in the sandbox. Both tools are Metered and
//! share the task's outbound GET quota. The key never appears in logs, errors,
//! or `GET /api/settings/search`.

use std::net::{IpAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::extract::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use robroz_contract::{SearchSettings, SearchSettingsSource, SearchTestResult};
use robroz_policy::SideEffect;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::settings::{as_opt_opt, env_get, setting, upsert_setting};
use crate::tools::{Tool, ToolCtx, ToolRegistry};
use crate::{db_error, error_response, lock_db, AppState};

/// How long a search or fetch (and the test endpoint) will wait.
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Default number of hits `web_search` returns, and the cap.
pub const DEFAULT_SEARCH_COUNT: u32 = 5;
pub const MAX_SEARCH_COUNT: u32 = 10;

/// Longest `web_fetch` body, in characters, including the truncation marker.
pub const MAX_FETCH_CHARS: usize = 16_000;
const TRUNCATED_MARK: &str = "\n…(truncated)";

/// Returned to the model (and shown on the delivery) when search is not set up.
pub const NOT_CONFIGURED: &str = robroz_contract::host_text::SEARCH_NOT_CONFIGURED;

pub const ENV_PROVIDER: &str = "ROBROZ_SEARCH_PROVIDER";
pub const ENV_BRAVE_KEY: &str = "ROBROZ_BRAVE_KEY";
pub const ENV_SEARXNG_URL: &str = "ROBROZ_SEARXNG_URL";

const SETTING_PROVIDER: &str = "search.provider";
const SETTING_URL: &str = "search.url";
const SETTING_KEY: &str = "search.key";

const BRAVE_DEFAULT_URL: &str = "https://api.search.brave.com/res/v1/web/search";

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    crate::settings::migrate(conn)
}

/// One search hit, already cleaned for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A search backend. Implementations must not put the key in error text.
pub trait SearchProvider: Send + Sync {
    fn search(&self, query: &str, count: u32) -> Result<Vec<SearchHit>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub provider: String,
    pub url: String,
    pub key: Option<String>,
}

impl EffectiveConfig {
    fn last4(&self) -> Option<String> {
        self.key.as_ref().map(|key| {
            let n = key.chars().count();
            key.chars().skip(n.saturating_sub(4)).collect()
        })
    }

    fn public(&self) -> SearchSettings {
        SearchSettings {
            provider: Some(self.provider.clone()),
            url: if self.provider == "brave" && self.url == BRAVE_DEFAULT_URL {
                None
            } else {
                Some(self.url.clone())
            },
            has_key: self.key.is_some(),
            key_last4: self.last4(),
            source: None,
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
}

/// Settings (when present) win over environment variables, field by field.
pub fn effective_config(conn: &Connection) -> rusqlite::Result<Option<EffectiveConfig>> {
    let provider = setting(conn, SETTING_PROVIDER)?.or_else(|| env_get(ENV_PROVIDER));
    let Some(provider) = provider else {
        return Ok(None);
    };
    let provider = provider.to_ascii_lowercase();
    match provider.as_str() {
        "brave" => {
            let url = setting(conn, SETTING_URL)?
                .or_else(|| env_get("ROBROZ_BRAVE_URL"))
                .unwrap_or_else(|| BRAVE_DEFAULT_URL.to_string());
            let key = setting(conn, SETTING_KEY)?.or_else(|| env_get(ENV_BRAVE_KEY));
            Ok(Some(EffectiveConfig { provider, url, key }))
        }
        "searxng" => {
            let Some(url) = setting(conn, SETTING_URL)?.or_else(|| env_get(ENV_SEARXNG_URL)) else {
                return Ok(None);
            };
            let key = setting(conn, SETTING_KEY)?.or_else(|| env_get(ENV_BRAVE_KEY));
            Ok(Some(EffectiveConfig { provider, url, key }))
        }
        _ => Ok(None),
    }
}

fn settings_are_set(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(setting(conn, SETTING_PROVIDER)?.is_some()
        || setting(conn, SETTING_URL)?.is_some()
        || setting(conn, SETTING_KEY)?.is_some())
}

fn public_settings(conn: &Connection) -> rusqlite::Result<SearchSettings> {
    match effective_config(conn)? {
        Some(cfg) => {
            let mut body = cfg.public();
            body.source = Some(if settings_are_set(conn)? {
                SearchSettingsSource::Settings
            } else {
                SearchSettingsSource::Env
            });
            Ok(body)
        }
        None => Ok(SearchSettings {
            provider: None,
            url: None,
            has_key: false,
            key_last4: None,
            source: None,
        }),
    }
}

/// Brave Search API (`/res/v1/web/search`).
pub struct BraveProvider {
    url: String,
    key: String,
    timeout: Duration,
}

impl BraveProvider {
    pub fn new(url: String, key: String) -> Self {
        Self {
            url,
            key,
            timeout: SEARCH_TIMEOUT,
        }
    }
}

impl SearchProvider for BraveProvider {
    fn search(&self, query: &str, count: u32) -> Result<Vec<SearchHit>, String> {
        let mut url = Url::parse(&self.url)
            .map_err(|err| format!("{}{err}", robroz_contract::host_text::URL_INVALID_PREFIX))?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("count", &count.to_string());
        let agent = http_agent(self.timeout);
        let mut response = agent
            .get(url.as_str())
            .header("Accept", "application/json")
            .header("X-Subscription-Token", &self.key)
            .call()
            .map_err(|err| redact_err(&err.to_string(), Some(&self.key)))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|err| redact_err(&err.to_string(), Some(&self.key)))?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "{}{status}",
                robroz_contract::host_text::SEARCH_SERVICE_STATUS_PREFIX
            ));
        }
        parse_brave(&body).map_err(|err| redact_err(&err, Some(&self.key)))
    }
}

pub fn parse_brave(body: &str) -> Result<Vec<SearchHit>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| robroz_contract::host_text::SEARCH_NOT_JSON.to_string())?;
    let results = value
        .pointer("/web/results")
        .and_then(Value::as_array)
        .ok_or_else(|| robroz_contract::host_text::SEARCH_NO_WEB_RESULTS.to_string())?;
    Ok(results.iter().filter_map(hit_from_brave).collect())
}

fn hit_from_brave(item: &Value) -> Option<SearchHit> {
    let url = item.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    Some(SearchHit {
        title: item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        url: url.to_string(),
        snippet: item
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
    })
}

/// A SearXNG instance with `format=json`.
pub struct SearxProvider {
    url: String,
    timeout: Duration,
}

impl SearxProvider {
    pub fn new(url: String) -> Self {
        Self {
            url,
            timeout: SEARCH_TIMEOUT,
        }
    }

    fn search_url(&self) -> Result<Url, String> {
        let trimmed = self.url.trim().trim_end_matches('/');
        let base = if trimmed.ends_with("/search") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/search")
        };
        Url::parse(&base)
            .map_err(|err| format!("{}{err}", robroz_contract::host_text::URL_INVALID_PREFIX))
    }
}

impl SearchProvider for SearxProvider {
    fn search(&self, query: &str, count: u32) -> Result<Vec<SearchHit>, String> {
        let mut url = self.search_url()?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("format", "json")
            .append_pair("categories", "general");
        let agent = http_agent(self.timeout);
        let mut response = agent
            .get(url.as_str())
            .header("Accept", "application/json")
            .call()
            .map_err(|err| err.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|err| err.to_string())?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "{}{status}",
                robroz_contract::host_text::SEARCH_SERVICE_STATUS_PREFIX
            ));
        }
        let mut hits = parse_searx(&body)?;
        hits.truncate(count as usize);
        Ok(hits)
    }
}

pub fn parse_searx(body: &str) -> Result<Vec<SearchHit>, String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| robroz_contract::host_text::SEARCH_NOT_JSON.to_string())?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| robroz_contract::host_text::SEARCH_NO_RESULTS.to_string())?;
    Ok(results.iter().filter_map(hit_from_searx).collect())
}

fn hit_from_searx(item: &Value) -> Option<SearchHit> {
    let url = item.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    Some(SearchHit {
        title: item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        url: url.to_string(),
        snippet: item
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
    })
}

fn http_agent(timeout: Duration) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false);
    // Tests talk to a one-shot localhost server; a process-wide proxy would
    // intercept those connections and break them.
    #[cfg(test)]
    let builder = builder.proxy(None);
    builder.build().new_agent()
}

fn redact_err(err: &str, key: Option<&str>) -> String {
    match key {
        Some(key) if !key.is_empty() && err.contains(key) => err.replace(key, "***"),
        _ => err.to_string(),
    }
}

fn provider_from(cfg: &EffectiveConfig) -> Result<Box<dyn SearchProvider>, String> {
    match cfg.provider.as_str() {
        "brave" => {
            let key = cfg.key.clone().ok_or_else(|| NOT_CONFIGURED.to_string())?;
            Ok(Box::new(BraveProvider::new(cfg.url.clone(), key)))
        }
        "searxng" => Ok(Box::new(SearxProvider::new(cfg.url.clone()))),
        other => Err(format!(
            "{}{other}",
            robroz_contract::host_text::SEARCH_PROVIDER_UNSUPPORTED_PREFIX
        )),
    }
}

fn format_hits(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "No results found.".into();
    }
    hits.iter()
        .enumerate()
        .map(|(i, hit)| {
            let mut line = format!("{}. {}", i + 1, hit.title);
            if !hit.url.is_empty() {
                line.push('\n');
                line.push_str(&hit.url);
            }
            if !hit.snippet.is_empty() {
                line.push('\n');
                line.push_str(&hit.snippet);
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn do_search(conn: &Connection, query: &str, count: u32) -> Result<String, String> {
    let Some(cfg) = effective_config(conn).map_err(|err| err.to_string())? else {
        return Err(NOT_CONFIGURED.to_string());
    };
    if cfg.provider == "brave" && cfg.key.is_none() {
        return Err(NOT_CONFIGURED.to_string());
    }
    let provider = provider_from(&cfg)?;
    let hits = provider
        .search(query, count)
        .map_err(|err| cfg.redact(&err))?;
    Ok(format_hits(&hits))
}

// ---- settings HTTP ----

pub async fn get_search(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    match public_settings(&conn) {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

#[derive(Debug, Deserialize)]
struct PutSearch {
    /// Missing: leave as-is. `null`: clear. A string: set.
    #[serde(default, deserialize_with = "as_opt_opt")]
    provider: Option<Option<String>>,
    #[serde(default, deserialize_with = "as_opt_opt")]
    url: Option<Option<String>>,
    #[serde(default, deserialize_with = "as_opt_opt")]
    key: Option<Option<String>>,
}

pub async fn put_search(
    State(state): State<AppState>,
    body: Result<axum::Json<Value>, JsonRejection>,
) -> Response {
    let axum::Json(raw) = match body {
        Ok(body) => body,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                robroz_contract::host_text::BODY_NOT_JSON,
            )
        }
    };
    let put: PutSearch = match serde_json::from_value(raw) {
        Ok(put) => put,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                robroz_contract::host_text::BODY_INVALID,
            )
        }
    };
    if let Err(err) = validate_put(&put) {
        return error_response(StatusCode::BAD_REQUEST, &err);
    }
    let conn = lock_db(&state);
    if let Err(err) = apply_put_checked(&conn, &put) {
        return error_response(StatusCode::BAD_REQUEST, &err);
    }
    match public_settings(&conn) {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

fn validate_put(put: &PutSearch) -> Result<(), String> {
    if let Some(Some(provider)) = &put.provider {
        let p = provider.trim().to_ascii_lowercase();
        if p != "brave" && p != "searxng" {
            return Err(robroz_contract::host_text::BAD_SEARCH_PROVIDER.into());
        }
        if p == "searxng" {
            // Need a URL either in this request or already stored — checked in apply
            // after we know the merged url. Format-check any url supplied here.
        }
    }
    if let Some(Some(url)) = &put.url {
        let url = url.trim();
        if url.is_empty() {
            return Err(robroz_contract::host_text::NEED_URL.into());
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(robroz_contract::host_text::URL_NEEDS_HTTP.into());
        }
    }
    if let Some(Some(key)) = &put.key {
        if key.trim().is_empty() {
            return Err(robroz_contract::host_text::KEY_EMPTY.into());
        }
        if key.chars().any(char::is_whitespace) {
            return Err(robroz_contract::host_text::KEY_HAS_WHITESPACE.into());
        }
    }
    Ok(())
}

fn apply_put_checked(conn: &Connection, put: &PutSearch) -> Result<(), String> {
    // Preview the merged provider/url before writing.
    let next_provider = match &put.provider {
        Some(None) => env_get(ENV_PROVIDER),
        Some(Some(p)) => Some(p.trim().to_ascii_lowercase()),
        None => setting(conn, SETTING_PROVIDER)
            .map_err(|e| e.to_string())?
            .or_else(|| env_get(ENV_PROVIDER)),
    };
    let next_url = match &put.url {
        Some(None) => env_get(ENV_SEARXNG_URL),
        Some(Some(u)) => Some(u.trim().to_string()),
        None => setting(conn, SETTING_URL)
            .map_err(|e| e.to_string())?
            .or_else(|| env_get(ENV_SEARXNG_URL)),
    };
    if next_provider.as_deref() == Some("searxng") && next_url.is_none() {
        return Err(robroz_contract::host_text::NEED_SEARXNG_URL.into());
    }
    if let Some(Some(p)) = &put.provider {
        upsert_setting(conn, SETTING_PROVIDER, Some(&p.trim().to_ascii_lowercase()))
            .map_err(|e| e.to_string())?;
    } else if matches!(&put.provider, Some(None)) {
        upsert_setting(conn, SETTING_PROVIDER, None).map_err(|e| e.to_string())?;
    }
    if let Some(url) = &put.url {
        upsert_setting(
            conn,
            SETTING_URL,
            url.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(key) = &put.key {
        upsert_setting(conn, SETTING_KEY, key.as_deref()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn test_search(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    let result = match effective_config(&conn) {
        Err(err) => return db_error(err),
        Ok(None) => SearchTestResult {
            ok: false,
            error: Some(NOT_CONFIGURED.into()),
        },
        Ok(Some(cfg)) => {
            if cfg.provider == "brave" && cfg.key.is_none() {
                SearchTestResult {
                    ok: false,
                    error: Some(NOT_CONFIGURED.into()),
                }
            } else {
                match provider_from(&cfg).and_then(|p| p.search("robroz", 1)) {
                    Ok(_) => SearchTestResult {
                        ok: true,
                        error: None,
                    },
                    Err(err) => SearchTestResult {
                        ok: false,
                        error: Some(format!(
                            "{}{}",
                            robroz_contract::host_text::CANT_CONNECT_PREFIX,
                            cfg.redact(&err)
                        )),
                    },
                }
            }
        }
    };
    (StatusCode::OK, Json(result)).into_response()
}

// ---- tools ----

#[derive(Clone)]
pub struct SearchStore {
    db: Arc<Mutex<Connection>>,
}

impl SearchStore {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    fn with<R>(&self, f: impl FnOnce(&Connection) -> R) -> R {
        let conn = self.db.lock().expect("sqlite connection lock poisoned");
        f(&conn)
    }
}

pub struct WebSearch {
    store: SearchStore,
}

pub struct WebFetch;

pub fn register_tools(registry: &mut ToolRegistry, db: Arc<Mutex<Connection>>) {
    let store = SearchStore::new(db);
    registry.register(WebSearch { store });
    registry.register(WebFetch);
}

impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the public web. Returns titles, links and snippets."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search terms" },
                "count": { "type": "integer", "description": "Number of results, default 5, at most 10", "minimum": 1, "maximum": MAX_SEARCH_COUNT }
            },
            "required": ["query"]
        })
    }
    fn effect(&self) -> SideEffect {
        SideEffect::Outbound
    }
    fn call(&self, args: &Value, ctx: &mut ToolCtx) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "query is required".to_string())?;
        let count = args
            .get("count")
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .unwrap_or(DEFAULT_SEARCH_COUNT)
            .clamp(1, MAX_SEARCH_COUNT);
        let configured = self.store.with(|conn| {
            effective_config(conn)
                .map_err(|err| err.to_string())
                .map(|cfg| {
                    cfg.filter(|c| c.provider != "brave" || c.key.is_some())
                        .is_some()
                })
        });
        let configured = configured?;
        if !configured {
            ctx.mark_needs_setup("search");
            return Ok(NOT_CONFIGURED.to_string());
        }
        if let Err(over) = ctx.charge_get() {
            return Err(over.failure());
        }
        self.store.with(|conn| do_search(conn, query, count))
    }
}

impl Tool for WebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "Read an http or https page with GET. Returns plain text."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The http or https address to read" }
            },
            "required": ["url"]
        })
    }
    fn effect(&self) -> SideEffect {
        SideEffect::Outbound
    }
    fn call(&self, args: &Value, ctx: &mut ToolCtx) -> Result<String, String> {
        let raw = args
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "url is required".to_string())?;
        let url = parse_public_http_url(raw)?;
        if let Err(over) = ctx.charge_get() {
            return Err(over.failure());
        }
        fetch_text(&url)
    }
}

/// Reject anything that is not a public http(s) URL (SSRF).
pub fn parse_public_http_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "地址无效".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("只允许 http 或 https".into());
    }
    if url.host().is_none() {
        return Err("地址无效".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("地址不能带用户名或密码".into());
    }
    let host = url.host_str().ok_or_else(|| "地址无效".to_string())?;
    if is_blocked_host(host) {
        return Err("不能访问这个地址".into());
    }
    // Resolve and reject private / loopback / link-local answers.
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|_| "无法解析这个地址".to_string())?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if is_blocked_ip(addr.ip()) {
            return Err("不能访问这个地址".into());
        }
    }
    if !any {
        return Err("无法解析这个地址".into());
    }
    Ok(url)
}

pub fn is_blocked_host(host: &str) -> bool {
    let host = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host == "localhost.localdomain"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host == "0.0.0.0"
        || host == "::"
        || host == "::1"
    {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_blocked_ip(ip);
    }
    false
}

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 64)
                || v4.octets()[0] == 169 && v4.octets()[1] == 254
        }
        IpAddr::V6(v6) => {
            let v6 = if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(v4));
            } else {
                v6
            };
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

fn fetch_text(url: &Url) -> Result<String, String> {
    let agent = http_agent(SEARCH_TIMEOUT);
    let mut response = agent
        .get(url.as_str())
        .call()
        .map_err(|err| format!("连不上：{err}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("页面返回 {status}"));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let body = response
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_string()
        .map_err(|err| format!("读页面失败：{err}"))?;
    let text = if content_type.contains("html") || looks_like_html(&body) {
        html_to_text(&body)
    } else {
        collapse_ws(&body)
    };
    Ok(clip_chars(text, MAX_FETCH_CHARS))
}

fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html")
}

fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut skip = false;
    let mut tag = String::new();
    for ch in html.chars() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let name = tag
                    .trim()
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if tag.trim_start().starts_with('/') && (name == "script" || name == "style") {
                    skip = false;
                }
                if matches!(
                    name.as_str(),
                    "p" | "div" | "br" | "tr" | "li" | "h1" | "h2" | "h3"
                ) {
                    out.push('\n');
                }
                tag.clear();
            } else if tag.len() < 32 {
                tag.push(ch);
                if tag.len() >= 6 {
                    let start = tag.trim_start().to_ascii_lowercase();
                    if start.starts_with("script") || start.starts_with("style") {
                        skip = true;
                    }
                }
            }
            continue;
        }
        if ch == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        if !skip {
            out.push(ch);
        }
    }
    decode_entities(&collapse_ws(&out))
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn clip_chars(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let keep = max.saturating_sub(TRUNCATED_MARK.chars().count());
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(TRUNCATED_MARK);
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::settings::test_env;
    use crate::tools::ToolCtx;
    use serde_json::json;

    const BRAVE_JSON: &str = r#"{"web":{"results":[{"title":"Rust","url":"https://www.rust-lang.org/","description":"A language."},{"title":"","url":"https://doc.rust-lang.org/","description":"The book."}]}}"#;
    const SEARX_JSON: &str = r#"{"results":[{"title":"Ada","url":"https://ada.example/","content":"About Ada."},{"url":""}]}"#;

    #[test]
    fn parse_brave_and_searx_hits() {
        let brave = parse_brave(BRAVE_JSON).unwrap();
        assert_eq!(brave.len(), 2);
        assert_eq!(brave[0].title, "Rust");
        assert_eq!(brave[0].url, "https://www.rust-lang.org/");
        assert_eq!(brave[0].snippet, "A language.");
        let searx = parse_searx(SEARX_JSON).unwrap();
        assert_eq!(searx.len(), 1, "empty url dropped");
        assert_eq!(searx[0].title, "Ada");
        assert_eq!(searx[0].snippet, "About Ada.");
        assert!(parse_brave("not json").is_err());
        assert!(parse_searx(r#"{"hits":[]}"#).is_err());
    }

    #[test]
    fn format_hits_is_readable() {
        let text = format_hits(&[SearchHit {
            title: "Rust".into(),
            url: "https://www.rust-lang.org/".into(),
            snippet: "A language.".into(),
        }]);
        assert!(text.contains("1. Rust"));
        assert!(text.contains("https://www.rust-lang.org/"));
        assert_eq!(format_hits(&[]), "No results found.");
    }

    #[test]
    fn ssrf_rejects_private_and_loopback() {
        for raw in [
            "http://127.0.0.1/",
            "http://localhost/",
            "https://[::1]/",
            "http://10.0.0.1/",
            "http://192.168.1.1/x",
            "http://172.16.0.5/",
            "http://169.254.169.254/latest",
            "http://0.0.0.0/",
            "file:///etc/passwd",
            "ftp://example.com/",
            "http://user:pass@example.com/",
            "http://127.0.0.1:8080/",
            "http://[fe80::1]/",
        ] {
            assert!(parse_public_http_url(raw).is_err(), "{raw}");
        }
        assert!(is_blocked_host("localhost"));
        assert!(is_blocked_host("foo.localhost"));
        assert!(is_blocked_ip("100.64.0.1".parse().unwrap()));
        // A public hostname should pass the host-name checks; we don't resolve
        // example.com here beyond parse. 203.0.113.1 is documentation (TEST-NET-3),
        // treated as public by the std checks.
        assert!(!is_blocked_ip("203.0.113.1".parse().unwrap()));
        assert!(!is_blocked_host("example.com"));
    }

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn public_settings_source_is_env_settings_or_absent() {
        let conn = mem_db();
        let body = public_settings(&conn).unwrap();
        assert!(body.source.is_none());
        assert!(body.provider.is_none());

        test_env::with(
            &[(ENV_PROVIDER, "brave"), (ENV_BRAVE_KEY, "envkeyXXXX")],
            || {
                let body = public_settings(&conn).unwrap();
                assert_eq!(body.source, Some(SearchSettingsSource::Env));
                assert_eq!(body.provider.as_deref(), Some("brave"));
                assert_eq!(body.key_last4.as_deref(), Some("XXXX"));

                upsert_setting(&conn, SETTING_PROVIDER, Some("brave")).unwrap();
                upsert_setting(&conn, SETTING_KEY, Some("setkeyABCD")).unwrap();
                let body = public_settings(&conn).unwrap();
                assert_eq!(body.source, Some(SearchSettingsSource::Settings));
                assert_eq!(body.key_last4.as_deref(), Some("ABCD"));

                upsert_setting(&conn, SETTING_PROVIDER, None).unwrap();
                upsert_setting(&conn, SETTING_KEY, None).unwrap();
                let body = public_settings(&conn).unwrap();
                assert_eq!(body.source, Some(SearchSettingsSource::Env));
                assert_eq!(body.key_last4.as_deref(), Some("XXXX"));
            },
        );
    }

    #[test]
    fn settings_override_env_and_never_echo_the_key() {
        let conn = mem_db();
        test_env::with(
            &[(ENV_PROVIDER, "brave"), (ENV_BRAVE_KEY, "envkeyXXXX")],
            || {
                let cfg = effective_config(&conn).unwrap().unwrap();
                assert_eq!(cfg.provider, "brave");
                assert_eq!(cfg.key.as_deref(), Some("envkeyXXXX"));
                let public = cfg.public();
                assert!(public.has_key);
                assert_eq!(public.key_last4.as_deref(), Some("XXXX"));
                assert!(serde_json::to_string(&public).unwrap().contains("XXXX"));
                assert!(!serde_json::to_string(&public).unwrap().contains("envkey"));

                upsert_setting(&conn, SETTING_PROVIDER, Some("searxng")).unwrap();
                upsert_setting(&conn, SETTING_URL, Some("https://searx.example")).unwrap();
                upsert_setting(&conn, SETTING_KEY, None).unwrap();
                let cfg = effective_config(&conn).unwrap().unwrap();
                assert_eq!(cfg.provider, "searxng");
                assert_eq!(cfg.url, "https://searx.example");
                // settings cleared the key, so env key may still apply — we deleted
                // only settings key, env is still there. That's field-level override
                // of the key field: absence of settings key falls back to env.
                assert_eq!(cfg.key.as_deref(), Some("envkeyXXXX"));
            },
        );
    }

    #[test]
    fn clearing_settings_key_falls_back_or_goes_empty() {
        let conn = mem_db();
        upsert_setting(&conn, SETTING_PROVIDER, Some("brave")).unwrap();
        upsert_setting(&conn, SETTING_KEY, Some("setkeyABCD")).unwrap();
        assert_eq!(
            effective_config(&conn).unwrap().unwrap().last4().as_deref(),
            Some("ABCD")
        );
        upsert_setting(&conn, SETTING_KEY, None).unwrap();
        assert!(effective_config(&conn).unwrap().unwrap().key.is_none());
    }

    fn fake_http(status: u16, body: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/search");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0;
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).unwrap();
                if n == 0 {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            if len > 0 {
                let mut buf = vec![0; len];
                let _ = reader.read_exact(&mut buf);
                head.push_str(&String::from_utf8_lossy(&buf));
            }
            let _ = tx.send(head);
            let mut stream = stream;
            let payload = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(payload.as_bytes());
            let _ = stream.flush();
        });
        (url, rx)
    }

    #[test]
    fn brave_and_searx_providers_parse_a_real_http_response() {
        let (url, rx) = fake_http(200, BRAVE_JSON);
        let hits = BraveProvider::new(url, "secret-key-9999".into())
            .search("rust", 5)
            .unwrap();
        assert_eq!(hits[0].title, "Rust");
        let req = rx.recv().unwrap();
        assert!(
            req.contains("X-Subscription-Token: secret-key-9999")
                || req.contains("x-subscription-token: secret-key-9999"),
            "{req}"
        );
        assert!(
            !format!("{:?}", hits).contains("secret-key"),
            "hits must not carry the key"
        );

        let (url, _) = fake_http(200, SEARX_JSON);
        let hits = SearxProvider::new(url).search("ada", 5).unwrap();
        assert_eq!(hits[0].title, "Ada");
    }

    #[test]
    fn provider_errors_do_not_include_the_key() {
        let (url, _) = fake_http(401, r#"{"error":"bad key secret-key-9999"}"#);
        let err = BraveProvider::new(url, "secret-key-9999".into())
            .search("q", 1)
            .unwrap_err();
        assert!(!err.contains("secret-key-9999"), "{err}");
        assert!(err.contains("401"), "{err}");
    }

    #[test]
    fn web_search_unconfigured_marks_needs_setup() {
        let conn = mem_db();
        let db = Arc::new(Mutex::new(conn));
        let tool = WebSearch {
            store: SearchStore::new(db),
        };
        let mut ctx = ToolCtx::new(8);
        let out = tool.call(&json!({ "query": "rust" }), &mut ctx).unwrap();
        assert_eq!(out, NOT_CONFIGURED);
        assert_eq!(ctx.needs_setup.as_deref(), Some("search"));
        assert_eq!(ctx.used, 0, "an unconfigured call is not a GET");
    }

    #[test]
    fn web_search_and_fetch_are_metered() {
        let (url, _) = fake_http(200, BRAVE_JSON);
        let conn = mem_db();
        upsert_setting(&conn, SETTING_PROVIDER, Some("brave")).unwrap();
        upsert_setting(&conn, SETTING_URL, Some(&url)).unwrap();
        upsert_setting(&conn, SETTING_KEY, Some("k123")).unwrap();
        let db = Arc::new(Mutex::new(conn));
        let tool = WebSearch {
            store: SearchStore::new(db),
        };
        let mut ctx = ToolCtx::new(1);
        let out = tool.call(&json!({ "query": "rust" }), &mut ctx).unwrap();
        assert!(out.contains("Rust"), "{out}");
        assert_eq!(ctx.used, 1);
        let err = tool
            .call(&json!({ "query": "again" }), &mut ctx)
            .unwrap_err();
        assert_eq!(err, "出网超额：已用 2 次，上限 1");
        assert!(ctx.over_quota.is_some());
    }

    #[test]
    fn web_fetch_rejects_ssrf_before_charging() {
        let mut ctx = ToolCtx::new(8);
        let err = WebFetch
            .call(&json!({ "url": "http://127.0.0.1/" }), &mut ctx)
            .unwrap_err();
        assert_eq!(err, "不能访问这个地址");
        assert_eq!(ctx.used, 0);
    }

    #[test]
    fn html_is_stripped_and_long_text_is_cut() {
        let text = html_to_text("<html><script>secret()</script><p>Hello&nbsp;<b>Ada</b></p>");
        assert_eq!(text, "Hello Ada");
        assert!(!text.contains("secret"));
        let cut = clip_chars("字".repeat(MAX_FETCH_CHARS + 10), MAX_FETCH_CHARS);
        assert_eq!(cut.chars().count(), MAX_FETCH_CHARS);
        assert!(cut.ends_with(TRUNCATED_MARK));
    }

    fn temp_path(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn search_app() -> (PathBuf, axum::Router) {
        let db = std::env::temp_dir().join(format!("robroz-search-{}.db", uuid::Uuid::new_v4()));
        let volumes = temp_path("robroz-search-vol");
        let host = crate::Host::open(&db, &volumes, "robroz-runner:local").unwrap();
        (db, host.router())
    }

    async fn http(
        app: &axum::Router,
        req: axum::http::Request<axum::body::Body>,
    ) -> (axum::http::StatusCode, Value) {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
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

    fn get_req(uri: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("GET")
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn json_req(method: &str, uri: &str, body: Value) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn settings_api_never_echoes_the_key_and_supports_clear() {
        let (_db, app) = search_app();
        let (status, body) = http(&app, get_req("/api/settings/search")).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["provider"], Value::Null);
        assert_eq!(body["url"], Value::Null);
        assert_eq!(body["hasKey"], false);
        assert_eq!(body["keyLast4"], Value::Null);
        assert!(body.get("key").is_none(), "{body}");
        assert!(body.get("source").is_none(), "{body}");

        let key = "secret-key-ABCD";
        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/search",
                json!({ "provider": "brave", "key": key }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["provider"], "brave");
        assert_eq!(body["hasKey"], true);
        assert_eq!(body["keyLast4"], "ABCD");
        assert_eq!(body["source"], "settings");
        assert!(body.get("key").is_none(), "{body}");
        let raw = body.to_string();
        assert!(!raw.contains(key), "GET/PUT must not echo the key: {raw}");

        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/search",
                json!({ "provider": "brave" }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["hasKey"], true);
        assert_eq!(body["keyLast4"], "ABCD");

        let (status, body) = http(
            &app,
            json_req("PUT", "/api/settings/search", json!({ "key": null })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["hasKey"], false);
        assert_eq!(body["keyLast4"], Value::Null);

        let (status, body) = http(
            &app,
            json_req("PUT", "/api/settings/search", json!({ "provider": "nope" })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");

        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/search",
                json!({ "provider": "searxng" }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{body}");

        let (status, body) = http(
            &app,
            json_req("POST", "/api/settings/search/test", json!({})),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], NOT_CONFIGURED);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn settings_source_is_env_settings_or_absent() {
        let (_db, app) = search_app();

        let (status, body) = http(&app, get_req("/api/settings/search")).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert!(
            body.get("source").is_none(),
            "unconfigured omits source: {body}"
        );

        let env = [(ENV_PROVIDER, "brave"), (ENV_BRAVE_KEY, "envkeyXXXX")];
        let _guard = test_env::set(&env);
        let (status, body) = http(&app, get_req("/api/settings/search")).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["source"], "env");
        assert_eq!(body["provider"], "brave");
        assert_eq!(body["keyLast4"], "XXXX");
        assert!(body.get("key").is_none(), "{body}");

        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/search",
                json!({ "provider": "brave", "key": "setkeyABCD" }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["source"], "settings", "PUT body carries source");
        assert_eq!(body["keyLast4"], "ABCD");

        let (status, body) = http(&app, get_req("/api/settings/search")).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["source"], "settings");

        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/search",
                json!({ "provider": null, "url": null, "key": null }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["source"], "env", "cleared settings fall back to env");
        assert_eq!(body["keyLast4"], "XXXX");
    }

    #[tokio::test]
    async fn test_endpoint_queries_the_configured_provider() {
        let (url, _rx) = fake_http(200, SEARX_JSON);
        let (_db, app) = search_app();
        let (status, body) = http(
            &app,
            json_req(
                "PUT",
                "/api/settings/search",
                json!({ "provider": "searxng", "url": url }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["provider"], "searxng");
        assert_eq!(body["url"], url);
        assert_eq!(body["hasKey"], false);

        let (status, body) = http(
            &app,
            json_req("POST", "/api/settings/search/test", json!({})),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["ok"], true, "{body}");
        assert!(
            body.get("error").is_none() || body["error"].is_null(),
            "{body}"
        );
    }

    struct ScriptedModel {
        turns: std::sync::Mutex<std::collections::VecDeque<robroz_model::AssistantTurn>>,
    }

    impl robroz_model::Model for ScriptedModel {
        fn reply(&self, _: &str, _: Option<&str>) -> Result<String, robroz_model::ModelError> {
            Err(robroz_model::ModelError::new("scripted model uses chat"))
        }

        fn chat(
            &self,
            _: &[robroz_model::ChatMessage],
            _: &[robroz_model::ToolDefinition],
        ) -> Result<robroz_model::AssistantTurn, robroz_model::ModelError> {
            self.turns
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| robroz_model::ModelError::new("没有更多回合"))
        }
    }

    #[tokio::test]
    async fn unconfigured_web_search_delivery_carries_needs_setup() {
        use std::sync::Arc;
        let db = std::env::temp_dir().join(format!("robroz-search-{}.db", uuid::Uuid::new_v4()));
        let volumes = temp_path("robroz-search-vol");
        let control = temp_path("robroz-search-ctl");
        let model = ScriptedModel {
            turns: std::sync::Mutex::new(
                [
                    robroz_model::AssistantTurn::ToolCalls(vec![robroz_model::ToolCall {
                        id: "call-1".into(),
                        name: "web_search".into(),
                        arguments: r#"{"query":"rust"}"#.into(),
                    }]),
                    robroz_model::AssistantTurn::Text("搜过了".into()),
                ]
                .into(),
            ),
        };
        let host = crate::Host::open(&db, &volumes, "robroz-runner:local")
            .unwrap()
            .with_control_root(&control)
            .with_model(Some(Arc::new(model)));
        let app = host.router();

        let (status, group) = http(
            &app,
            json_req("POST", "/api/groups", json!({ "name": "搜索" })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{group}");
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
        assert_eq!(status, axum::http::StatusCode::CREATED, "{body}");
        let (status, task) = http(
            &app,
            json_req(
                "POST",
                &format!("/api/groups/{group_id}/tasks"),
                json!({ "assigneeBotId": "bot-ada", "prompt": "搜一下 rust" }),
            ),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{task}");

        let mut sandbox = crate::sandbox::FakeSandbox::new();
        host.pump_once(&mut sandbox).unwrap();
        sandbox.finish(0);
        host.pump_once(&mut sandbox).unwrap();

        let (status, events) = http(&app, get_req(&format!("/api/groups/{group_id}/events"))).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{events}");
        let delivery = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "delivery")
            .expect("delivery");
        assert_eq!(delivery["needsSetup"], "search", "{delivery}");
        let result = std::fs::read_to_string(
            volumes
                .join(&group_id)
                .join(format!("tasks/{}/result.txt", task["id"].as_str().unwrap())),
        )
        .unwrap();
        assert!(result.contains(NOT_CONFIGURED), "{result}");
        assert!(result.contains("搜过了"), "{result}");
    }
}
