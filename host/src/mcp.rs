//! MCP server configuration. The host is the MCP client; this module only
//! stores servers and lists their tools. Registering tools into the model
//! loop is a later PR.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{
    CreateMcpServer, McpSecretIn, McpSecretInput, McpSecretPublic, McpServer, McpServers,
    McpTestResult, McpTool, McpTools, UpdateMcpServer,
};
use robroz_mcp::{ConnectConfig, McpConnector, RmcpConnector, TransportKind};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::{db_error, error_response, group_exists, lock_db, new_id, now_rfc3339, AppState};

const NAME_TAKEN: &str = "这个名字已经有了";
const NOT_FOUND: &str = "MCP 服务器不存在";
const GROUP_NOT_FOUND: &str = "群不存在";
const NAME_MAX: usize = 80;
const SECRET_NAME_MAX: usize = 64;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        transport TEXT NOT NULL,
        command TEXT,
        args TEXT,
        url TEXT,
        secrets TEXT NOT NULL DEFAULT '[]',
        enabled INTEGER NOT NULL DEFAULT 1,
        tool_count INTEGER,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS group_mcp (
        group_id TEXT NOT NULL,
        server_id TEXT NOT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY (group_id, server_id)
    );
";

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

#[derive(Debug, Clone)]
struct Secret {
    name: String,
    placement: McpSecretIn,
    value: String,
}

impl Secret {
    fn public(&self) -> McpSecretPublic {
        McpSecretPublic {
            name: self.name.clone(),
            placement: self.placement,
            has_key: !self.value.is_empty(),
            key_last4: last4(&self.value),
        }
    }
}

fn last4(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let n = value.chars().count();
    Some(value.chars().skip(n.saturating_sub(4)).collect())
}

#[derive(Debug, Clone)]
struct Row {
    id: String,
    name: String,
    transport: String,
    command: Option<String>,
    args: Vec<String>,
    url: Option<String>,
    secrets: Vec<Secret>,
    enabled: bool,
    tool_count: Option<u32>,
    created_at: String,
    updated_at: String,
}

impl Row {
    fn public(&self) -> McpServer {
        McpServer {
            id: self.id.clone(),
            name: self.name.clone(),
            transport: self.transport.clone(),
            command: self.command.clone(),
            args: if self.transport == "stdio" {
                Some(self.args.clone())
            } else {
                None
            },
            url: self.url.clone(),
            secrets: self.secrets.iter().map(Secret::public).collect(),
            enabled: self.enabled,
            tool_count: self.tool_count,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }

    fn connect_config(&self) -> Result<ConnectConfig, String> {
        let kind = parse_transport(&self.transport)
            .ok_or_else(|| "transport 只能是 stdio 或 http".to_string())?;
        let mut env = Vec::new();
        let mut headers = Vec::new();
        for secret in &self.secrets {
            if secret.value.is_empty() {
                continue;
            }
            match secret.placement {
                McpSecretIn::Env => env.push((secret.name.clone(), secret.value.clone())),
                McpSecretIn::Header => headers.push((secret.name.clone(), secret.value.clone())),
            }
        }
        Ok(ConnectConfig {
            transport: kind,
            command: self.command.clone(),
            args: self.args.clone(),
            url: self.url.clone(),
            env,
            headers,
        })
    }

    fn secret_values(&self) -> Vec<String> {
        self.secrets
            .iter()
            .map(|s| s.value.clone())
            .filter(|v| !v.is_empty())
            .collect()
    }
}

fn parse_transport(raw: &str) -> Option<TransportKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "stdio" => Some(TransportKind::Stdio),
        "http" => Some(TransportKind::Http),
        _ => None,
    }
}

fn transport_str(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Stdio => "stdio",
        TransportKind::Http => "http",
    }
}

fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    )
}

fn body_error(_rejection: JsonRejection) -> Response {
    error_response(StatusCode::BAD_REQUEST, "请求格式不正确")
}

fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("请填写名称");
    }
    if name.chars().count() > NAME_MAX {
        return Err("名称最多 80 个字");
    }
    if name.contains('\n') || name.contains('\r') {
        return Err("名称只能一行");
    }
    Ok(name.to_string())
}

fn clean_command(raw: &str) -> Result<String, &'static str> {
    let command = raw.trim();
    if command.is_empty() {
        return Err("请填写启动命令");
    }
    if command.contains('\0') {
        return Err("启动命令无效");
    }
    Ok(command.to_string())
}

fn clean_url(raw: &str) -> Result<String, &'static str> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("请填写地址");
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("地址要以 http:// 或 https:// 开头");
    }
    Ok(url.to_string())
}

fn clean_secret_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("请填写密钥名称");
    }
    if name.chars().count() > SECRET_NAME_MAX {
        return Err("密钥名称最多 64 个字");
    }
    if name.chars().any(char::is_whitespace) {
        return Err("密钥名称里不能有空白");
    }
    Ok(name.to_string())
}

fn default_placement(kind: TransportKind, given: Option<McpSecretIn>) -> McpSecretIn {
    given.unwrap_or(match kind {
        TransportKind::Stdio => McpSecretIn::Env,
        TransportKind::Http => McpSecretIn::Header,
    })
}

fn parse_args(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

fn parse_secrets(raw: &str) -> Vec<Secret> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_string();
            let placement = item
                .get("in")
                .and_then(Value::as_str)
                .and_then(McpSecretIn::parse)
                .unwrap_or(McpSecretIn::Env);
            let value = item
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(Secret {
                name,
                placement,
                value,
            })
        })
        .collect()
}

fn secrets_json(secrets: &[Secret]) -> String {
    let items: Vec<Value> = secrets
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "in": s.placement.as_str(),
                "value": s.value,
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let args_raw: Option<String> = row.get(4)?;
    let secrets_raw: String = row.get(6)?;
    let tool_count: Option<i64> = row.get(8)?;
    let enabled: i64 = row.get(7)?;
    Ok(Row {
        id: row.get(0)?,
        name: row.get(1)?,
        transport: row.get(2)?,
        command: row.get(3)?,
        args: parse_args(args_raw.as_deref()),
        url: row.get(5)?,
        secrets: parse_secrets(&secrets_raw),
        enabled: enabled != 0,
        tool_count: tool_count.map(|n| n as u32),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const SELECT: &str = "SELECT id, name, transport, command, args, url, secrets, enabled, tool_count, created_at, updated_at
     FROM mcp_servers";

fn load_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<Row>> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
        .optional()
}

fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY updated_at DESC, id DESC"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect()
}

fn list_enabled_for_group(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT mcp_servers.id, mcp_servers.name, mcp_servers.transport, mcp_servers.command, mcp_servers.args,
                mcp_servers.url, mcp_servers.secrets, mcp_servers.enabled, mcp_servers.tool_count,
                mcp_servers.created_at, mcp_servers.updated_at
         FROM mcp_servers
         JOIN group_mcp g ON g.server_id = mcp_servers.id
         WHERE g.group_id = ?1
         ORDER BY mcp_servers.updated_at DESC, mcp_servers.id DESC",
    )?;
    let rows = stmt.query_map([group_id], from_row)?;
    rows.collect()
}

/// A server that is both globally enabled and turned on for this group.
pub(crate) struct ReadyServer {
    pub id: String,
    pub name: String,
    pub config: robroz_mcp::ConnectConfig,
}

pub(crate) fn ready_servers_for_group(
    conn: &Connection,
    group_id: &str,
) -> rusqlite::Result<Vec<ReadyServer>> {
    let rows = list_enabled_for_group(conn, group_id)?;
    let mut out = Vec::new();
    for row in rows {
        if !row.enabled {
            continue;
        }
        match row.connect_config() {
            Ok(config) => out.push(ReadyServer {
                id: row.id,
                name: row.name,
                config,
            }),
            Err(err) => {
                eprintln!("mcp: skip server {} ({err})", row.id);
            }
        }
    }
    Ok(out)
}

fn insert_row(conn: &Connection, row: &Row) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO mcp_servers (id, name, transport, command, args, url, secrets, enabled, tool_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            row.id,
            row.name,
            row.transport,
            row.command,
            serde_json::to_string(&row.args).unwrap_or_else(|_| "[]".into()),
            row.url,
            secrets_json(&row.secrets),
            if row.enabled { 1 } else { 0 },
            row.tool_count.map(|n| n as i64),
            row.created_at,
            row.updated_at,
        ],
    )?;
    Ok(())
}

fn update_row(conn: &Connection, row: &Row) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE mcp_servers SET name = ?1, transport = ?2, command = ?3, args = ?4, url = ?5,
             secrets = ?6, enabled = ?7, tool_count = ?8, updated_at = ?9
         WHERE id = ?10",
        params![
            row.name,
            row.transport,
            row.command,
            serde_json::to_string(&row.args).unwrap_or_else(|_| "[]".into()),
            row.url,
            secrets_json(&row.secrets),
            if row.enabled { 1 } else { 0 },
            row.tool_count.map(|n| n as i64),
            row.updated_at,
            row.id,
        ],
    )?;
    Ok(())
}

fn secrets_from_input(
    kind: TransportKind,
    inputs: &[McpSecretInput],
    existing: &[Secret],
) -> Result<Vec<Secret>, &'static str> {
    let mut out = existing.to_vec();
    for input in inputs {
        let name = clean_secret_name(&input.name)?;
        let placement = default_placement(kind, input.placement);
        match &input.key {
            None => {
                if let Some(found) = out.iter_mut().find(|s| s.name == name) {
                    found.placement = placement;
                } else {
                    return Err("新增密钥时请填写值");
                }
            }
            Some(None) => {
                out.retain(|s| s.name != name);
            }
            Some(Some(value)) => {
                if value.is_empty() {
                    return Err("密钥不能为空");
                }
                if let Some(found) = out.iter_mut().find(|s| s.name == name) {
                    found.placement = placement;
                    found.value = value.clone();
                } else {
                    out.push(Secret {
                        name,
                        placement,
                        value: value.clone(),
                    });
                }
            }
        }
    }
    Ok(out)
}

fn build_create(body: CreateMcpServer) -> Result<Row, &'static str> {
    let name = clean_name(&body.name)?;
    let kind = parse_transport(&body.transport).ok_or("transport 只能是 stdio 或 http")?;
    let (command, args, url) = match kind {
        TransportKind::Stdio => {
            let command = clean_command(body.command.as_deref().unwrap_or(""))?;
            let args = body.args.unwrap_or_default();
            if args.iter().any(|a| a.contains('\0')) {
                return Err("参数无效");
            }
            (Some(command), args, None)
        }
        TransportKind::Http => {
            let url = clean_url(body.url.as_deref().unwrap_or(""))?;
            (None, Vec::new(), Some(url))
        }
    };
    let secrets = secrets_from_input(kind, body.secrets.as_deref().unwrap_or(&[]), &[])?;
    let now = now_rfc3339();
    Ok(Row {
        id: new_id(),
        name,
        transport: transport_str(kind).to_string(),
        command,
        args,
        url,
        secrets,
        enabled: body.enabled.unwrap_or(true),
        tool_count: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

fn apply_patch(row: &mut Row, patch: UpdateMcpServer) -> Result<(), &'static str> {
    if let Some(name) = patch.name {
        row.name = clean_name(&name)?;
    }
    if let Some(transport) = patch.transport {
        let kind = parse_transport(&transport).ok_or("transport 只能是 stdio 或 http")?;
        row.transport = transport_str(kind).to_string();
        match kind {
            TransportKind::Stdio => row.url = None,
            TransportKind::Http => {
                row.command = None;
                row.args = Vec::new();
            }
        }
    }
    let kind = parse_transport(&row.transport).ok_or("transport 只能是 stdio 或 http")?;
    if let Some(command) = patch.command {
        match command {
            None => row.command = None,
            Some(value) => row.command = Some(clean_command(&value)?),
        }
    }
    if let Some(args) = patch.args {
        if args.iter().any(|a| a.contains('\0')) {
            return Err("参数无效");
        }
        row.args = args;
    }
    if let Some(url) = patch.url {
        match url {
            None => row.url = None,
            Some(value) => row.url = Some(clean_url(&value)?),
        }
    }
    match kind {
        TransportKind::Stdio => {
            if row.command.as_deref().unwrap_or("").is_empty() {
                return Err("请填写启动命令");
            }
            row.url = None;
        }
        TransportKind::Http => {
            if row.url.as_deref().unwrap_or("").is_empty() {
                return Err("请填写地址");
            }
            row.command = None;
            row.args = Vec::new();
        }
    }
    if let Some(inputs) = patch.secrets {
        row.secrets = secrets_from_input(kind, &inputs, &row.secrets)?;
    }
    if let Some(enabled) = patch.enabled {
        row.enabled = enabled;
    }
    row.updated_at = now_rfc3339();
    Ok(())
}

fn redact(err: &str, secrets: &[String]) -> String {
    let mut out = err.to_string();
    for secret in secrets {
        if !secret.is_empty() && out.contains(secret) {
            out = out.replace(secret, "***");
        }
    }
    out
}

async fn list_tools_now(row: &Row) -> Result<Vec<robroz_mcp::ListedTool>, String> {
    let cfg = row.connect_config()?;
    let secrets = row.secret_values();
    let join = tokio::spawn(async move { RmcpConnector.list_tools(&cfg).await });
    match join.await {
        Ok(Ok(tools)) => Ok(tools),
        Ok(Err(err)) => Err(redact(&err, &secrets)),
        Err(err) => Err(format!("连接中断：{err}")),
    }
}

pub(crate) async fn list_servers(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    match list_all(&conn) {
        Ok(rows) => {
            let servers = rows.iter().map(Row::public).collect();
            (StatusCode::OK, Json(McpServers { servers })).into_response()
        }
        Err(err) => db_error(err),
    }
}

pub(crate) async fn create_server(
    State(state): State<AppState>,
    body: Result<Json<CreateMcpServer>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return body_error(rejection),
    };
    let row = match build_create(body) {
        Ok(row) => row,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let conn = lock_db(&state);
    let result = insert_row(&conn, &row);
    drop(conn);
    match result {
        Ok(()) => (StatusCode::CREATED, Json(row.public())).into_response(),
        Err(err) if is_unique_violation(&err) => error_response(StatusCode::CONFLICT, NAME_TAKEN),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn patch_server(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Result<Json<UpdateMcpServer>, JsonRejection>,
) -> Response {
    let Json(patch) = match body {
        Ok(body) => body,
        Err(rejection) => return body_error(rejection),
    };
    let conn = lock_db(&state);
    let result = (|| {
        let Some(mut row) = load_by_id(&conn, &id)? else {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        };
        if let Err(msg) = apply_patch(&mut row, patch) {
            return Ok(Err((StatusCode::BAD_REQUEST, msg)));
        }
        match update_row(&conn, &row) {
            Ok(()) => Ok(Ok(row)),
            Err(err) if is_unique_violation(&err) => Ok(Err((StatusCode::CONFLICT, NAME_TAKEN))),
            Err(err) => Err(err),
        }
    })();
    drop(conn);
    match result {
        Ok(Ok(row)) => (StatusCode::OK, Json(row.public())).into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn delete_server(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if load_by_id(&conn, &id)?.is_none() {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        }
        conn.execute("DELETE FROM group_mcp WHERE server_id = ?1", [&id])?;
        conn.execute("DELETE FROM mcp_servers WHERE id = ?1", [&id])?;
        Ok::<_, rusqlite::Error>(Ok(()))
    })();
    drop(conn);
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn list_tools(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let loaded = {
        let conn = lock_db(&state);
        load_by_id(&conn, &id)
    };
    let row = match loaded {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, NOT_FOUND),
        Err(err) => return db_error(err),
    };
    match list_tools_now(&row).await {
        Ok(tools) => {
            let count = tools.len() as u32;
            {
                let conn = lock_db(&state);
                let _ = conn.execute(
                    "UPDATE mcp_servers SET tool_count = ?1, updated_at = ?2 WHERE id = ?3",
                    params![count as i64, now_rfc3339(), row.id],
                );
            }
            let body = McpTools {
                tools: tools
                    .into_iter()
                    .map(|t| {
                        let needs_approval = t.needs_approval();
                        McpTool {
                            name: t.name,
                            description: t.description,
                            needs_approval,
                        }
                    })
                    .collect(),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(err) => error_response(StatusCode::BAD_GATEWAY, &err),
    }
}

pub(crate) async fn test_connection(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let loaded = {
        let conn = lock_db(&state);
        load_by_id(&conn, &id)
    };
    let row = match loaded {
        Ok(Some(row)) => row,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, NOT_FOUND),
        Err(err) => return db_error(err),
    };
    let result = match list_tools_now(&row).await {
        Ok(tools) => {
            let tool_count = tools.len() as u32;
            {
                let conn = lock_db(&state);
                let _ = conn.execute(
                    "UPDATE mcp_servers SET tool_count = ?1, updated_at = ?2 WHERE id = ?3",
                    params![tool_count as i64, now_rfc3339(), row.id],
                );
            }
            McpTestResult {
                ok: true,
                error: None,
                tool_count: Some(tool_count),
            }
        }
        Err(err) => McpTestResult {
            ok: false,
            error: Some(err),
            tool_count: None,
        },
    };
    (StatusCode::OK, Json(result)).into_response()
}

pub(crate) async fn list_group_mcp(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        Ok(Ok(McpServers {
            servers: list_enabled_for_group(&conn, &group_id)?
                .iter()
                .map(Row::public)
                .collect(),
        }))
    })();
    drop(conn);
    match result {
        Ok(Ok(body)) => (StatusCode::OK, Json(body)).into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn get_group_mcp(
    State(state): State<AppState>,
    AxumPath((group_id, server_id)): AxumPath<(String, String)>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        if load_by_id(&conn, &server_id)?.is_none() {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        }
        let enabled = list_enabled_for_group(&conn, &group_id)?
            .into_iter()
            .find(|s| s.id == server_id);
        Ok(Ok(enabled))
    })();
    drop(conn);
    match result {
        Ok(Ok(Some(row))) => (StatusCode::OK, Json(row.public())).into_response(),
        Ok(Ok(None)) => error_response(StatusCode::NOT_FOUND, "这个群还没启用这台服务器"),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn enable_group_mcp(
    State(state): State<AppState>,
    AxumPath((group_id, server_id)): AxumPath<(String, String)>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        if load_by_id(&conn, &server_id)?.is_none() {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        }
        conn.execute(
            "INSERT OR IGNORE INTO group_mcp (group_id, server_id, created_at) VALUES (?1, ?2, ?3)",
            params![group_id, server_id, now_rfc3339()],
        )?;
        Ok::<_, rusqlite::Error>(Ok(()))
    })();
    drop(conn);
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn disable_group_mcp(
    State(state): State<AppState>,
    AxumPath((group_id, server_id)): AxumPath<(String, String)>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        if load_by_id(&conn, &server_id)?.is_none() {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        }
        conn.execute(
            "DELETE FROM group_mcp WHERE group_id = ?1 AND server_id = ?2",
            params![group_id, server_id],
        )?;
        Ok::<_, rusqlite::Error>(Ok(()))
    })();
    drop(conn);
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn temp_path(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn app() -> (PathBuf, Router) {
        let db = std::env::temp_dir().join(format!("robroz-mcp-{}.db", uuid::Uuid::new_v4()));
        let volumes = temp_path("robroz-mcp-vol");
        let host = crate::Host::open(&db, &volumes, "robroz-runner:local").unwrap();
        (db, host.router())
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

    async fn create_stdio(
        app: &Router,
        name: &str,
        command: &str,
        key: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut body = json!({
            "name": name,
            "transport": "stdio",
            "command": command,
            "args": ["--ok"],
        });
        if let Some(key) = key {
            body["secrets"] = json!([{ "name": "TOKEN", "in": "env", "key": key }]);
        }
        http(app, json_req("POST", "/api/mcp/servers", body)).await
    }

    fn fixture_bin() -> PathBuf {
        if let Some(path) = find_fixture_bin() {
            return path;
        }
        let status = std::process::Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "robroz-mcp",
                "--features",
                "fixture",
                "--bin",
                "robroz-mcp-fixture",
                "--offline",
                "--quiet",
            ])
            .status()
            .expect("build fixture");
        assert!(status.success(), "failed to build robroz-mcp-fixture");
        find_fixture_bin().expect("fixture bin after build")
    }

    fn find_fixture_bin() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
            candidates.push(PathBuf::from(dir).join("debug/robroz-mcp-fixture"));
        }
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest.join("../target/debug/robroz-mcp-fixture"));
        if let Ok(exe) = std::env::current_exe() {
            if let Some(debug) = exe.parent().and_then(|p| p.parent()) {
                candidates.push(debug.join("robroz-mcp-fixture"));
            }
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    #[tokio::test]
    async fn crud_and_secrets_never_echo_the_key() {
        let (_db, app) = app();
        let key = "secret-key-WXYZ";
        let (status, body) = create_stdio(&app, "git", "/bin/true", Some(key)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["id"].as_str().unwrap().to_string();
        assert_eq!(body["name"], "git");
        assert_eq!(body["transport"], "stdio");
        assert_eq!(body["command"], "/bin/true");
        assert_eq!(body["args"], json!(["--ok"]));
        assert_eq!(body["enabled"], true);
        assert!(body["toolCount"].is_null(), "{body}");
        assert_eq!(body["secrets"][0]["name"], "TOKEN");
        assert_eq!(body["secrets"][0]["in"], "env");
        assert_eq!(body["secrets"][0]["hasKey"], true);
        assert_eq!(body["secrets"][0]["keyLast4"], "WXYZ");
        let raw = body.to_string();
        assert!(!raw.contains(key), "must not echo the key: {raw}");
        assert!(body["secrets"][0].get("key").is_none(), "{body}");
        assert!(body["secrets"][0].get("value").is_none(), "{body}");

        let (status, body) = http(&app, get("/api/mcp/servers")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["servers"].as_array().unwrap().len(), 1);
        assert!(!body.to_string().contains(key), "{}", body);

        let (status, body) = http(
            &app,
            json_req(
                "PATCH",
                &format!("/api/mcp/servers/{id}"),
                json!({ "name": "git-tools" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["name"], "git-tools");
        assert_eq!(body["secrets"][0]["hasKey"], true);
        assert_eq!(body["secrets"][0]["keyLast4"], "WXYZ");

        let (status, body) = http(
            &app,
            json_req(
                "PATCH",
                &format!("/api/mcp/servers/{id}"),
                json!({ "secrets": [{ "name": "TOKEN", "in": "env", "key": null }] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["secrets"].as_array().unwrap().is_empty(), "{body}");

        let (status, body) = create_stdio(&app, "git-tools", "/bin/true", None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"], super::NAME_TAKEN);

        let (status, _) = http(
            &app,
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/mcp/servers/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = http(&app, get("/api/mcp/servers")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["servers"].as_array().unwrap().is_empty(), "{body}");
    }

    #[tokio::test]
    async fn group_enable_lists_and_clears_on_delete() {
        let (_db, app) = app();
        let (status, group) = http(
            &app,
            json_req("POST", "/api/groups", json!({ "name": "MCP" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group}");
        let group_id = group["id"].as_str().unwrap().to_string();
        let (status, server) = create_stdio(&app, "docs", "/bin/true", None).await;
        assert_eq!(status, StatusCode::CREATED, "{server}");
        let server_id = server["id"].as_str().unwrap().to_string();

        let (status, body) = http(&app, get(&format!("/api/groups/{group_id}/mcp"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["servers"].as_array().unwrap().is_empty());

        let (status, _) = http(
            &app,
            Request::builder()
                .method("PUT")
                .uri(format!("/api/groups/{group_id}/mcp/{server_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = http(&app, get(&format!("/api/groups/{group_id}/mcp"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["servers"][0]["id"], server_id);
        let (status, body) = http(
            &app,
            get(&format!("/api/groups/{group_id}/mcp/{server_id}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["id"], server_id);

        let (status, _) = http(
            &app,
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/groups/{group_id}/mcp/{server_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = http(&app, get(&format!("/api/groups/{group_id}/mcp"))).await;
        assert!(body["servers"].as_array().unwrap().is_empty(), "{body}");
        assert_eq!(status, StatusCode::OK);

        let (status, _) = http(
            &app,
            Request::builder()
                .method("PUT")
                .uri(format!("/api/groups/{group_id}/mcp/{server_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = http(
            &app,
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/mcp/servers/{server_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = http(&app, get(&format!("/api/groups/{group_id}/mcp"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["servers"].as_array().unwrap().is_empty(), "{body}");
    }

    #[tokio::test]
    async fn tools_and_test_over_http_fixture() {
        let server = robroz_mcp::fixture::serve_http().await.unwrap();
        let (_db, app) = app();
        let (status, body) = http(
            &app,
            json_req(
                "POST",
                "/api/mcp/servers",
                json!({ "name": "http-fix", "transport": "http", "url": server.url }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["id"].as_str().unwrap().to_string();

        let (status, body) = http(&app, get(&format!("/api/mcp/servers/{id}/tools"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let tools = body["tools"].as_array().unwrap();
        let ping = tools.iter().find(|t| t["name"] == "ping").unwrap();
        let write = tools.iter().find(|t| t["name"] == "write_note").unwrap();
        assert_eq!(ping["needsApproval"], false, "{body}");
        assert_eq!(write["needsApproval"], true, "{body}");
        assert_eq!(ping["description"], "Check the server is up");

        let (status, body) = http(
            &app,
            Request::builder()
                .method("POST")
                .uri(format!("/api/mcp/servers/{id}/test"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["toolCount"], 2);

        let (status, listed) = http(&app, get("/api/mcp/servers")).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["servers"][0]["toolCount"], 2);
    }

    #[tokio::test]
    async fn tools_and_test_over_stdio_fixture() {
        let bin = fixture_bin();
        let (_db, app) = app();
        let (status, body) = http(
            &app,
            json_req(
                "POST",
                "/api/mcp/servers",
                json!({
                    "name": "stdio-fix",
                    "transport": "stdio",
                    "command": bin.to_string_lossy(),
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["id"].as_str().unwrap().to_string();
        let (status, body) = http(&app, get(&format!("/api/mcp/servers/{id}/tools"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let tools = body["tools"].as_array().unwrap();
        assert!(
            tools
                .iter()
                .any(|t| t["name"] == "ping" && t["needsApproval"] == false),
            "{body}"
        );
        assert!(
            tools
                .iter()
                .any(|t| t["name"] == "write_note" && t["needsApproval"] == true),
            "{body}"
        );

        let (status, body) = http(
            &app,
            Request::builder()
                .method("POST")
                .uri(format!("/api/mcp/servers/{id}/test"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["toolCount"], 2);
    }

    #[tokio::test]
    async fn invalid_create_is_400() {
        let (_db, app) = app();
        let (status, _) = http(
            &app,
            json_req(
                "POST",
                "/api/mcp/servers",
                json!({ "name": "x", "transport": "ftp" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = http(
            &app,
            json_req(
                "POST",
                "/api/mcp/servers",
                json!({ "name": "x", "transport": "http" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = http(&app, get("/api/groups/missing/mcp")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
