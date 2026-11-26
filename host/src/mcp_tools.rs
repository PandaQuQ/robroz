//! Register MCP tools into a task's [`ToolRegistry`] and persist a paused
//! tool-calling loop while a person answers an approval card.
//!
//! Tool names sent to the model are `{safe_server}__{safe_tool}` because
//! common function-calling APIs only allow `[A-Za-z0-9_-]` (max 64 chars).
//! Progress lines and approval cards use the original `{server}.{tool}`.

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use robroz_mcp::{ConnectConfig, ListedTool, McpPool, PoolConfig};
use robroz_model::{ChatMessage, ToolCall};
use robroz_policy::{action_hash, SideEffect};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::mcp::ReadyServer;
use crate::tools::{McpApproval, PendingApproval, Tool, ToolCtx, ToolRegistry};

/// Longest tool result the model sees, including the truncation marker.
/// Same cap as [`crate::tools::MAX_TOOL_RESULT_CHARS`].
#[allow(dead_code)]
pub const MAX_MCP_RESULT_CHARS: usize = crate::tools::MAX_TOOL_RESULT_CHARS;

const PENDING_FILE: &str = "mcp-pending.json";

/// Sync façade over the MCP client, for the host pump and step threads.
///
/// `server_id` is the `mcp_servers.id` the config belongs to, and
/// `group_id` is the group (or bro DM) the task is running in. The pooled
/// runtime keeps one process per (server, group) ([`robroz_mcp::StdioPool`]);
/// other runtimes ignore both ids.
pub trait McpRuntime: Send + Sync {
    fn list_tools(
        &self,
        server_id: &str,
        group_id: &str,
        cfg: &ConnectConfig,
    ) -> Result<Vec<ListedTool>, String>;
    fn call_tool(
        &self,
        server_id: &str,
        group_id: &str,
        cfg: &ConnectConfig,
        name: &str,
        args: &Value,
    ) -> Result<String, String>;
    /// The server was edited, disabled or deleted: stop every pooled
    /// process for this server, in every group.
    fn evict(&self, _server_id: &str) {}
    /// The group was deleted: stop every pooled process started for it.
    /// No route calls this yet.
    fn evict_group(&self, _group_id: &str) {}
    /// The host is exiting: stop every pooled process. Blocks up to a few seconds.
    fn shutdown(&self) {}
}

/// The production runtime: stdio servers stay running between tasks.
pub struct PooledMcp(pub McpPool);

/// How long [`McpRuntime::shutdown`] waits for pooled processes to exit.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

impl McpRuntime for PooledMcp {
    fn list_tools(
        &self,
        server_id: &str,
        group_id: &str,
        cfg: &ConnectConfig,
    ) -> Result<Vec<ListedTool>, String> {
        block_on(self.0.list_tools(server_id, group_id, cfg))
    }

    fn call_tool(
        &self,
        server_id: &str,
        group_id: &str,
        cfg: &ConnectConfig,
        name: &str,
        args: &Value,
    ) -> Result<String, String> {
        block_on(self.0.call_tool(server_id, group_id, cfg, name, args))
    }

    fn evict(&self, server_id: &str) {
        self.0.evict(server_id);
    }

    fn evict_group(&self, group_id: &str) {
        self.0.evict_group(group_id);
    }

    fn shutdown(&self) {
        block_on(self.0.shutdown(SHUTDOWN_GRACE));
    }
}

/// Pool settings from the environment (`ROBROZ_MCP_IDLE_SECS`,
/// `ROBROZ_MCP_MAX_IN_FLIGHT`). A missing or invalid value keeps the default.
pub fn pool_config_from_env() -> PoolConfig {
    pool_config_from(
        std::env::var("ROBROZ_MCP_IDLE_SECS").ok().as_deref(),
        std::env::var("ROBROZ_MCP_MAX_IN_FLIGHT").ok().as_deref(),
    )
}

fn pool_config_from(idle_secs: Option<&str>, max_in_flight: Option<&str>) -> PoolConfig {
    let mut cfg = PoolConfig::default();
    if let Some(secs) = idle_secs.and_then(|v| v.trim().parse::<u64>().ok()) {
        cfg.idle_timeout = Duration::from_secs(secs);
        if secs > 0 {
            // Look often enough that a short idle limit is honoured.
            cfg.reap_interval = cfg.reap_interval.min(Duration::from_secs(secs.div_ceil(2)));
        }
    }
    if let Some(n) = max_in_flight
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
    {
        cfg.max_in_flight = n;
    }
    cfg
}

pub fn real_mcp() -> Arc<dyn McpRuntime> {
    Arc::new(PooledMcp(McpPool::new(pool_config_from_env())))
}

impl McpRuntime for robroz_mcp::FakeConnector {
    fn list_tools(
        &self,
        _server_id: &str,
        _group_id: &str,
        cfg: &ConnectConfig,
    ) -> Result<Vec<ListedTool>, String> {
        self.list_tools_sync(cfg)
    }

    fn call_tool(
        &self,
        _server_id: &str,
        _group_id: &str,
        cfg: &ConnectConfig,
        name: &str,
        args: &Value,
    ) -> Result<String, String> {
        self.call_tool_sync(cfg, name, args)
    }
}

fn block_on<T>(fut: impl Future<Output = T>) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
            .block_on(fut),
    }
}

/// Keep letters, digits, `_` and `-`. Empty names become `x`.
pub fn sanitize_ident(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "x".into()
    } else {
        out
    }
}

/// The name registered in the model tool list for `{server}.{tool}`.
pub fn registered_name(server_name: &str, tool_name: &str) -> String {
    fit_name(format!(
        "{}__{}",
        sanitize_ident(server_name),
        sanitize_ident(tool_name)
    ))
}

fn fit_name(name: String) -> String {
    if name.len() <= 64 {
        return name;
    }
    let hash = &action_hash(&name)[..8];
    let mut out: String = name.chars().take(55).collect();
    out.push('_');
    out.push_str(hash);
    out
}

fn unique_registered(base: String, used: &mut HashSet<String>, hint: &str) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let suffix = &action_hash(hint)[..6];
    let mut name = fit_name(format!("{base}_{suffix}"));
    let mut n = 0u32;
    while !used.insert(name.clone()) {
        n += 1;
        name = fit_name(format!("{base}_{suffix}{n}"));
    }
    name
}

pub fn display_name(server_name: &str, tool_name: &str) -> String {
    format!("{server_name}.{tool_name}")
}

/// Connect to each ready server and register its tools. A server that fails
/// to connect is skipped; the task continues.
pub fn register_tools(tools: &mut ToolRegistry, servers: &[ReadyServer], mcp: Arc<dyn McpRuntime>) {
    let mut used = tools
        .definitions()
        .into_iter()
        .map(|d| d.name)
        .collect::<HashSet<_>>();
    for server in servers {
        let listed = match mcp.list_tools(&server.id, &server.group_id, &server.config) {
            Ok(tools) => tools,
            Err(err) => {
                eprintln!("mcp: skip server {} ({}): {err}", server.name, server.id);
                continue;
            }
        };
        for tool in listed {
            let base = registered_name(&server.name, &tool.name);
            let hint = format!("{}:{}:{}", server.id, server.name, tool.name);
            let name = unique_registered(base, &mut used, &hint);
            tools.register(McpHostTool {
                name,
                display: display_name(&server.name, &tool.name),
                description: tool.description.clone().unwrap_or_else(|| {
                    format!("MCP tool {}", display_name(&server.name, &tool.name))
                }),
                schema: if tool.input_schema.is_null() {
                    json!({ "type": "object", "properties": {} })
                } else {
                    tool.input_schema
                },
                effect: if tool.read_only {
                    SideEffect::Outbound
                } else {
                    SideEffect::ExternalWrite
                },
                approval: McpApproval {
                    server_id: server.id.clone(),
                    server_name: server.name.clone(),
                    tool_name: tool.name,
                },
                group_id: server.group_id.clone(),
                config: server.config.clone(),
                mcp: mcp.clone(),
            });
        }
    }
}

struct McpHostTool {
    name: String,
    display: String,
    description: String,
    schema: Value,
    effect: SideEffect,
    approval: McpApproval,
    group_id: String,
    config: ConnectConfig,
    mcp: Arc<dyn McpRuntime>,
}

impl Tool for McpHostTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    fn effect(&self) -> SideEffect {
        self.effect
    }

    fn display_name(&self) -> &str {
        &self.display
    }

    fn mcp_approval(&self) -> Option<McpApproval> {
        Some(self.approval.clone())
    }

    fn call(&self, args: &Value, ctx: &mut ToolCtx) -> Result<String, String> {
        if let Err(over) = ctx.charge_get() {
            return Err(over.failure());
        }
        self.mcp.call_tool(
            &self.approval.server_id,
            &self.group_id,
            &self.config,
            &self.approval.tool_name,
            args,
        )
    }
}

/// One-line card summary: `调用 {server}.{tool}` plus a compact argument list.
pub fn approval_summary(server: &str, tool: &str, args: &Value) -> String {
    let mut s = format!(
        "{}{server}.{tool}",
        robroz_contract::host_text::MCP_CALL_SUMMARY_PREFIX
    );
    let compact = match args {
        Value::Null => None,
        Value::Object(map) if map.is_empty() => None,
        other => Some(other.to_string()),
    };
    if let Some(args) = compact {
        s.push_str(robroz_contract::host_text::MCP_CALL_ARGS_SEPARATOR);
        s.push_str(&args);
    }
    let s = s.replace(['\n', '\r'], " ");
    if s.chars().count() <= robroz_contract::APPROVAL_SUMMARY_MAX {
        s
    } else {
        let keep = robroz_contract::APPROVAL_SUMMARY_MAX.saturating_sub(1);
        let mut out: String = s.chars().take(keep).collect();
        out.push('…');
        out
    }
}

pub fn intent_hash(task_id: &str, pending: &PendingApproval) -> String {
    let call = pending
        .remaining
        .first()
        .map(|c| c.id.as_str())
        .unwrap_or_default();
    let mcp = pending.mcp.as_ref();
    action_hash(&format!(
        "mcp:{task_id}:{}:{}:{call}:{}",
        mcp.map(|m| m.server_id.as_str()).unwrap_or_default(),
        mcp.map(|m| m.tool_name.as_str()).unwrap_or_default(),
        pending.args
    ))
}

fn pending_path(control_dir: &Path) -> std::path::PathBuf {
    control_dir.join(PENDING_FILE)
}

/// Write the paused loop to `{control}/{task}/mcp-pending.json`.
pub fn save_pending(control_dir: &Path, pending: &PendingApproval) -> Result<(), String> {
    let value = json!({
        "decision": Value::Null,
        "display_name": pending.display_name,
        "args": pending.args,
        "round": pending.round,
        "used": pending.used,
        "needs_setup": pending.needs_setup,
        "mcp": pending.mcp.as_ref().map(|m| json!({
            "server_id": m.server_id,
            "server_name": m.server_name,
            "tool_name": m.tool_name,
        })),
        "remaining": pending.remaining.iter().map(call_to_json).collect::<Vec<_>>(),
        "messages": pending.messages.iter().map(message_to_json).collect::<Vec<_>>(),
    });
    let path = pending_path(control_dir);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| {
        format!(
            "{}{e}",
            robroz_contract::host_text::PENDING_WRITE_FAILED_PREFIX
        )
    })
}

pub fn mark_approved(control_dir: &Path) -> Result<(), String> {
    let path = pending_path(control_dir);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}{e}",
            robroz_contract::host_text::PENDING_NOT_FOUND_PREFIX
        )
    })?;
    let mut value: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{}{e}", robroz_contract::host_text::PENDING_CORRUPT_PREFIX))?;
    value["decision"] = json!("approved");
    std::fs::write(
        &path,
        serde_json::to_vec(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("无法写入批准标记：{e}"))
}

pub fn pending_is_approved(control_dir: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(pending_path(control_dir)) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value.get("decision").and_then(Value::as_str) == Some("approved")
}

pub fn load_approved(control_dir: &Path) -> Result<PendingApproval, String> {
    let path = pending_path(control_dir);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}{e}",
            robroz_contract::host_text::PENDING_NOT_FOUND_PREFIX
        )
    })?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{}{e}", robroz_contract::host_text::PENDING_CORRUPT_PREFIX))?;
    if value.get("decision").and_then(Value::as_str) != Some("approved") {
        return Err(robroz_contract::host_text::PENDING_NOT_APPROVED.into());
    }
    parse_pending(&value)
}

pub fn remove_pending(control_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(pending_path(control_dir)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

fn parse_pending(value: &Value) -> Result<PendingApproval, String> {
    let mcp = value.get("mcp").and_then(|v| {
        if v.is_null() {
            None
        } else {
            Some(McpApproval {
                server_id: v["server_id"].as_str().unwrap_or_default().to_string(),
                server_name: v["server_name"].as_str().unwrap_or_default().to_string(),
                tool_name: v["tool_name"].as_str().unwrap_or_default().to_string(),
            })
        }
    });
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or(robroz_contract::host_text::PENDING_MISSING_MESSAGES)?
        .iter()
        .map(message_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let remaining = value
        .get("remaining")
        .and_then(Value::as_array)
        .ok_or(robroz_contract::host_text::PENDING_MISSING_REMAINING)?
        .iter()
        .map(call_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PendingApproval {
        messages,
        remaining,
        round: value.get("round").and_then(Value::as_u64).unwrap_or(1) as usize,
        display_name: value
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        args: value.get("args").cloned().unwrap_or(Value::Null),
        mcp,
        used: value.get("used").and_then(Value::as_u64).unwrap_or(0),
        needs_setup: value
            .get("needs_setup")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn call_to_json(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "name": call.name,
        "arguments": call.arguments,
    })
}

fn call_from_json(value: &Value) -> Result<ToolCall, String> {
    Ok(ToolCall {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .ok_or(robroz_contract::host_text::PENDING_CALL_MISSING_ID)?
            .to_string(),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .ok_or(robroz_contract::host_text::PENDING_CALL_MISSING_NAME)?
            .to_string(),
        arguments: value
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("{}")
            .to_string(),
    })
}

fn message_to_json(message: &ChatMessage) -> Value {
    match message {
        ChatMessage::System(content) => json!({ "role": "system", "content": content }),
        ChatMessage::User(content) => json!({ "role": "user", "content": content }),
        ChatMessage::Assistant(robroz_model::AssistantTurn::Text(content)) => {
            json!({ "role": "assistant", "content": content })
        }
        ChatMessage::Assistant(robroz_model::AssistantTurn::ToolCalls(calls)) => json!({
            "role": "assistant",
            "tool_calls": calls.iter().map(call_to_json).collect::<Vec<_>>(),
        }),
        ChatMessage::Tool {
            tool_call_id,
            content,
        } => json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "content": content,
        }),
    }
}

fn message_from_json(value: &Value) -> Result<ChatMessage, String> {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .ok_or(robroz_contract::host_text::PENDING_MESSAGE_MISSING_ROLE)?;
    match role {
        "system" => Ok(ChatMessage::System(
            value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        "user" => Ok(ChatMessage::User(
            value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        "assistant" => {
            if let Some(calls) = value.get("tool_calls").and_then(Value::as_array) {
                let calls = calls
                    .iter()
                    .map(call_from_json)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ChatMessage::Assistant(
                    robroz_model::AssistantTurn::ToolCalls(calls),
                ))
            } else {
                Ok(ChatMessage::Assistant(robroz_model::AssistantTurn::Text(
                    value
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                )))
            }
        }
        "tool" => Ok(ChatMessage::Tool {
            tool_call_id: value
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            content: value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        other => Err(format!(
            "{}{other}",
            robroz_contract::host_text::PENDING_UNKNOWN_ROLE_PREFIX
        )),
    }
}

/// Insert a failed delivery that shows why the person denied an MCP call.
pub fn finish_denied(
    conn: &Connection,
    task_id: &str,
    hash: &str,
    failure: &str,
    now: &str,
) -> rusqlite::Result<()> {
    use rusqlite::params;
    // Caller holds the approval-answer transaction.
    let changed = conn.execute(
        "UPDATE tasks SET state = 'done', container_up = 0, updated_at = ?1, failure = ?2, progress = NULL,
                          intent_kind = NULL, intent_line = NULL, intent_hash = NULL
         WHERE id = ?3 AND state = 'blocked' AND intent_hash = ?4",
        params![now, failure, task_id, hash],
    )?;
    if changed > 0 {
        let (group_id, briefing): (String, i64) = conn.query_row(
            "SELECT group_id, briefing FROM tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        conn.execute(
            "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at, failure)
             VALUES (?1, ?2, ?3, ?8, ?4, ?5, ?6, ?7)",
            params![
                crate::new_id(),
                group_id,
                task_id,
                crate::sandbox::result_rel(task_id),
                briefing,
                now,
                failure,
                robroz_contract::host_text::DELIVERY_NOT_RUN
            ],
        )?;
    }
    Ok(())
}

pub fn queue_approved(
    conn: &Connection,
    task_id: &str,
    hash: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET state = 'queued', container_up = 0, updated_at = ?1, progress = NULL
         WHERE id = ?2 AND state = 'blocked' AND intent_hash = ?3",
        rusqlite::params![now, task_id, hash],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_config_reads_idle_and_in_flight() {
        let default = PoolConfig::default();
        assert_eq!(pool_config_from(None, None), default);
        assert_eq!(pool_config_from(Some("x"), Some("0")), default);
        let cfg = pool_config_from(Some(" 0 "), Some("1"));
        assert!(!cfg.pooling());
        assert_eq!(cfg.max_in_flight, 1);
        let cfg = pool_config_from(Some("20"), None);
        assert_eq!(cfg.idle_timeout, Duration::from_secs(20));
        assert_eq!(cfg.reap_interval, Duration::from_secs(10));
    }

    #[test]
    fn registered_name_is_safe_and_maps_from_display() {
        assert_eq!(registered_name("notes", "write_note"), "notes__write_note");
        assert_eq!(
            registered_name("Git Tools", "list files"),
            "Git_Tools__list_files"
        );
        assert_eq!(sanitize_ident("???"), "x");
        let long_server = "s".repeat(80);
        let name = registered_name(&long_server, "tool");
        assert!(name.len() <= 64, "{name}");
        assert!(name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn approval_summary_fits_one_line() {
        let s = approval_summary("notes", "write_note", &json!({"title":"hi"}));
        assert!(s.starts_with("调用 notes.write_note："), "{s}");
        assert!(!s.contains('\n'));
        let long = approval_summary("notes", "write_note", &json!({"x": "很".repeat(200)}));
        assert!(long.chars().count() <= 200, "{}", long.chars().count());
        assert!(long.ends_with('…'), "{long}");
    }

    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use robroz_mcp::{FakeConnector, ListedTool};
    use robroz_model::{AssistantTurn, ToolCall};
    use tower::ServiceExt;

    fn temp_path(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn http(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, body)
    }

    fn get_req(uri: &str) -> Request<Body> {
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
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn listed(name: &str, read_only: bool) -> ListedTool {
        ListedTool {
            name: name.into(),
            description: Some(name.into()),
            read_only,
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    struct ScriptedModel {
        turns: std::sync::Mutex<std::collections::VecDeque<AssistantTurn>>,
        seen: std::sync::Mutex<Vec<Vec<String>>>,
    }

    impl ScriptedModel {
        fn new(turns: Vec<AssistantTurn>) -> Arc<Self> {
            Arc::new(Self {
                turns: std::sync::Mutex::new(turns.into()),
                seen: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn seen_tools(&self) -> Vec<Vec<String>> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl robroz_model::Model for ScriptedModel {
        fn reply(&self, _: &str, _: Option<&str>) -> Result<String, robroz_model::ModelError> {
            Err(robroz_model::ModelError("scripted model uses chat".into()))
        }

        fn chat(
            &self,
            _: &[robroz_model::ChatMessage],
            tools: &[robroz_model::ToolDefinition],
        ) -> Result<AssistantTurn, robroz_model::ModelError> {
            self.seen
                .lock()
                .unwrap()
                .push(tools.iter().map(|t| t.name.clone()).collect());
            self.turns
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| robroz_model::ModelError("没有更多回合".into()))
        }
    }

    struct Harness {
        host: crate::Host,
        app: Router,
        volumes: PathBuf,
        fake_mcp: FakeConnector,
        group_id: String,
        model: Arc<ScriptedModel>,
    }

    async fn setup(turns: Vec<AssistantTurn>, enable: bool, offline: bool) -> Harness {
        let db = std::env::temp_dir().join(format!("robroz-mcp-calls-{}.db", uuid::Uuid::new_v4()));
        let volumes = temp_path("robroz-mcp-calls-vol");
        let control = temp_path("robroz-mcp-calls-ctl");
        let fake_mcp = FakeConnector::new();
        if offline {
            fake_mcp.set_offline("http://down.local", "down");
        }
        fake_mcp.set_tools(
            "http://notes.local",
            vec![listed("ping", true), listed("write_note", false)],
        );
        fake_mcp.set_result("http://notes.local", "ping", Ok("pong".into()));
        fake_mcp.set_result("http://notes.local", "write_note", Ok("written".into()));
        let model = ScriptedModel::new(turns);
        let host = crate::Host::open(&db, &volumes, "robroz-runner:local")
            .unwrap()
            .with_control_root(&control)
            .with_model(Some(model.clone()))
            .with_mcp(Arc::new(fake_mcp.clone()));
        let app = host.router();
        let (status, group) = http(
            &app,
            json_req("POST", "/api/groups", json!({ "name": "MCP" })),
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
        let (status, server) = http(
            &app,
            json_req(
                "POST",
                "/api/mcp/servers",
                json!({ "name": "notes", "transport": "http", "url": "http://notes.local" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{server}");
        let server_id = server["id"].as_str().unwrap().to_string();
        if enable {
            let (status, body) = http(
                &app,
                json_req(
                    "PUT",
                    &format!("/api/groups/{group_id}/mcp/{server_id}"),
                    json!({}),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        }
        if offline {
            let (status, down) = http(
                &app,
                json_req(
                    "POST",
                    "/api/mcp/servers",
                    json!({ "name": "down", "transport": "http", "url": "http://down.local" }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{down}");
            let down_id = down["id"].as_str().unwrap().to_string();
            let (status, body) = http(
                &app,
                json_req(
                    "PUT",
                    &format!("/api/groups/{group_id}/mcp/{down_id}"),
                    json!({}),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        }
        Harness {
            host,
            app,
            volumes,
            fake_mcp,
            group_id,
            model,
        }
    }

    async fn post_task(h: &Harness, prompt: &str) -> String {
        let (status, task) = http(
            &h.app,
            json_req(
                "POST",
                &format!("/api/groups/{}/tasks", h.group_id),
                json!({ "assigneeBotId": "bot-ada", "prompt": prompt }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        task["id"].as_str().unwrap().to_string()
    }

    async fn events(h: &Harness) -> Value {
        let (status, body) = http(
            &h.app,
            get_req(&format!("/api/groups/{}/events", h.group_id)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// Events where the task's approval cards are: its own group (v0.2;
    /// the owner's DM only gets a reminder line).
    async fn dm_events(h: &Harness) -> Value {
        let (status, body) = http(
            &h.app,
            get_req(&format!("/api/groups/{}/events", h.group_id)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn event_of<'a>(body: &'a Value, typ: &str) -> &'a Value {
        body["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == typ)
            .unwrap_or_else(|| panic!("missing {typ} in {body}"))
    }

    fn status_of(body: &Value) -> &Value {
        body["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "status")
            .expect("status")
    }

    fn call(name: &str, id: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args.into(),
        }
    }

    fn finish(h: &Harness, sandbox: &mut crate::sandbox::FakeSandbox) {
        sandbox.finish(0);
        h.host.pump_once(sandbox).unwrap();
    }

    #[tokio::test]
    async fn read_only_mcp_tool_runs_without_a_card() {
        let h = setup(
            vec![
                AssistantTurn::ToolCalls(vec![call("notes__ping", "c1", "{}")]),
                AssistantTurn::Text("pong 了".into()),
            ],
            true,
            false,
        )
        .await;
        let task_id = post_task(&h, "ping").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        assert_eq!(h.fake_mcp.calls().len(), 1);
        assert_eq!(h.fake_mcp.calls()[0].tool, "ping");
        let ev = events(&h).await;
        assert!(
            ev["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["type"] != "approval"),
            "{ev}"
        );
        finish(&h, &mut sandbox);
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "done", "{ev}");
        let delivery = event_of(&ev, "delivery");
        assert!(
            delivery.get("failure").is_none() || delivery["failure"].is_null(),
            "{delivery}"
        );
        let result = std::fs::read_to_string(
            h.volumes
                .join(&h.group_id)
                .join(format!("tasks/{task_id}/result.txt")),
        )
        .unwrap();
        assert!(result.contains("pong 了"), "{result}");
        let offered = h.model.seen_tools();
        assert!(
            offered[0].contains(&"notes__ping".to_string()),
            "{offered:?}"
        );
        assert!(
            offered[0].contains(&"notes__write_note".to_string()),
            "{offered:?}"
        );
    }

    #[tokio::test]
    async fn write_tool_raises_a_card_then_approve_executes() {
        let h = setup(
            vec![
                AssistantTurn::ToolCalls(vec![call(
                    "notes__write_note",
                    "c1",
                    r#"{"title":"hi"}"#,
                )]),
                AssistantTurn::Text("写好了".into()),
            ],
            true,
            false,
        )
        .await;
        let task_id = post_task(&h, "写笔记").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        assert!(
            h.fake_mcp.calls().is_empty(),
            "must not run before approval"
        );
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "blocked", "{ev}");
        assert_eq!(status_of(&ev)["waitingFor"], "approval", "{ev}");
        let card = event_of(&dm_events(&h).await, "approval").clone();
        assert_eq!(card["action"], "call", "{card}");
        assert_eq!(card["mcpServer"], "notes", "{card}");
        assert_eq!(card["mcpTool"], "write_note", "{card}");
        assert_eq!(card["taskId"], task_id, "{card}");
        assert!(
            card["summary"]
                .as_str()
                .unwrap()
                .contains("notes.write_note"),
            "{card}"
        );
        let progress = status_of(&ev)
            .get("progress")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_ne!(
            progress,
            robroz_contract::APPROVAL_SENT_TO_OWNER_DM_NOTICE,
            "v0.2: the card is in this group, no notice: {ev}"
        );
        assert!(
            ev["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["type"] == "approval"),
            "v0.2: the card stays in the group"
        );

        let id = card["id"].as_str().unwrap();
        let (status, body) = http(
            &h.app,
            json_req("POST", &format!("/api/approvals/{id}/approve"), json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["decision"], "approved", "{body}");

        h.host.pump_once(&mut sandbox).unwrap();
        assert_eq!(h.fake_mcp.calls().len(), 1);
        assert_eq!(h.fake_mcp.calls()[0].tool, "write_note");
        finish(&h, &mut sandbox);
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "done", "{ev}");
        let delivery = event_of(&ev, "delivery");
        assert!(
            delivery.get("failure").is_none() || delivery["failure"].is_null(),
            "{delivery}"
        );
        let result = std::fs::read_to_string(
            h.volumes
                .join(&h.group_id)
                .join(format!("tasks/{task_id}/result.txt")),
        )
        .unwrap();
        assert!(result.contains("写好了"), "{result}");
    }

    #[tokio::test]
    async fn deny_does_not_execute_and_second_answer_is_403_decided() {
        let h = setup(
            vec![AssistantTurn::ToolCalls(vec![call(
                "notes__write_note",
                "c1",
                r#"{"title":"hi"}"#,
            )])],
            true,
            false,
        )
        .await;
        let task_id = post_task(&h, "写笔记").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        let card = event_of(&dm_events(&h).await, "approval").clone();
        let id = card["id"].as_str().unwrap().to_string();

        let (status, body) = http(
            &h.app,
            json_req("POST", &format!("/api/approvals/{id}/deny"), json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["decision"], "denied", "{body}");
        assert!(h.fake_mcp.calls().is_empty(), "deny must not run the tool");

        let (status, body) = http(
            &h.app,
            json_req("POST", &format!("/api/approvals/{id}/deny"), json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["reason"], "decided", "{body}");
        let (status, body) = http(
            &h.app,
            json_req("POST", &format!("/api/approvals/{id}/approve"), json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        h.host.pump_once(&mut sandbox).unwrap();
        assert!(h.fake_mcp.calls().is_empty());
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "done", "{ev}");
        let delivery = event_of(&ev, "delivery");
        let failure = delivery["failure"].as_str().unwrap_or("");
        assert!(failure.contains("你拒绝了"), "{delivery}");
        assert!(failure.contains("notes.write_note"), "{delivery}");
        assert_eq!(delivery["failureCode"], "denied", "{delivery}");
        assert_eq!(delivery["failureDetail"], "notes.write_note", "{delivery}");
        assert_eq!(delivery["summary"], "未执行", "{delivery}");
        assert_eq!(delivery["taskId"], task_id, "{delivery}");
        assert_eq!(delivery["botId"], "bot-ada", "{delivery}");
        assert_eq!(delivery["botName"], "Ada", "{delivery}");
    }

    #[tokio::test]
    async fn resume_is_rejected_while_waiting_for_an_mcp_approval() {
        let h = setup(
            vec![AssistantTurn::ToolCalls(vec![call(
                "notes__write_note",
                "c1",
                r#"{"title":"hi"}"#,
            )])],
            true,
            false,
        )
        .await;
        let task_id = post_task(&h, "写笔记").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        let card = event_of(&dm_events(&h).await, "approval").clone();
        let id = card["id"].as_str().unwrap().to_string();

        let (status, body) = http(
            &h.app,
            Request::builder()
                .method("POST")
                .uri(format!("/api/tasks/{task_id}/resume"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"], "请先回答审批卡", "{body}");
        assert!(h.fake_mcp.calls().is_empty());

        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "blocked", "{ev}");
        let card = event_of(&dm_events(&h).await, "approval").clone();
        assert!(card["decision"].is_null(), "{card}");
        assert_eq!(card["id"], id, "{card}");
    }

    #[tokio::test]
    async fn tools_are_not_registered_when_the_group_has_not_enabled_the_server() {
        let h = setup(vec![AssistantTurn::Text("没有工具".into())], false, false).await;
        let _ = post_task(&h, "你好").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        let offered = h.model.seen_tools();
        assert!(!offered.is_empty(), "model should have been called");
        assert!(
            !offered[0].iter().any(|n| n.starts_with("notes__")),
            "must not register MCP tools: {offered:?}"
        );
        finish(&h, &mut sandbox);
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "done", "{ev}");
        assert!(
            ev["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["type"] != "approval"),
            "{ev}"
        );
    }

    #[tokio::test]
    async fn offline_server_is_skipped_and_the_task_continues() {
        let h = setup(
            vec![
                AssistantTurn::ToolCalls(vec![call("notes__ping", "c1", "{}")]),
                AssistantTurn::Text("还在".into()),
            ],
            true,
            true,
        )
        .await;
        let _ = post_task(&h, "ping").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        let offered = h.model.seen_tools();
        assert!(
            offered[0].contains(&"notes__ping".to_string()),
            "{offered:?}"
        );
        assert!(
            !offered[0].iter().any(|n| n.starts_with("down__")),
            "offline server must be skipped: {offered:?}"
        );
        assert_eq!(h.fake_mcp.calls().len(), 1);
        finish(&h, &mut sandbox);
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "done", "{ev}");
        let delivery = event_of(&ev, "delivery");
        assert!(
            delivery.get("failure").is_none() || delivery["failure"].is_null(),
            "{delivery}"
        );
    }

    #[tokio::test]
    async fn unapproved_stdio_server_is_skipped_in_the_model_loop() {
        let h = setup(vec![AssistantTurn::Text("好".into())], false, false).await;
        h.fake_mcp.set_tools(
            "/tmp/unapproved-mcp",
            vec![listed("ping", true), listed("write_note", false)],
        );
        let (status, server) = http(
            &h.app,
            json_req(
                "POST",
                "/api/mcp/servers",
                json!({
                    "name": "shell",
                    "transport": "stdio",
                    "command": "/tmp/unapproved-mcp",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{server}");
        assert_eq!(server["commandApproved"], false, "{server}");
        let server_id = server["id"].as_str().unwrap().to_string();
        let (status, body) = http(
            &h.app,
            json_req(
                "PUT",
                &format!("/api/groups/{}/mcp/{server_id}", h.group_id),
                json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

        let _ = post_task(&h, "ping").await;
        let mut sandbox = crate::sandbox::FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        let offered = h.model.seen_tools();
        assert_eq!(offered.len(), 1, "{offered:?}");
        assert!(
            !offered[0].iter().any(|n| n.starts_with("shell__")),
            "unapproved stdio server must not be registered: {offered:?}"
        );
        assert_eq!(h.fake_mcp.calls().len(), 0);
        finish(&h, &mut sandbox);
        let ev = events(&h).await;
        assert_eq!(status_of(&ev)["state"], "done", "{ev}");
    }
}
