//! MCP client used by the host: list and call tools over stdio or streamable HTTP.
//!
//! The host talks to servers through [`McpConnector`]. Production uses
//! [`RmcpConnector`] (the official `rmcp` crate). Tests can swap in
//! [`FakeConnector`] or start the bundled fixture server.
//!
//! The host itself uses [`McpPool`], which keeps stdio servers running
//! between tasks ([`pool`]) and sends HTTP servers through [`RmcpConnector`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

pub mod managed;
pub mod pool;

pub use managed::{McpPool, RmcpStdioSpawner};
pub use pool::{PoolConfig, StdioPool};

/// How long a connect + `tools/list` or `tools/call` is allowed to take.
/// [`McpPool`] applies it to starting a stdio process and to each request.
pub const MCP_TIMEOUT: Duration = Duration::from_secs(10);

/// A tool as the host shows it: approval is required unless the server
/// marked the tool `readOnlyHint: true`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedTool {
    pub name: String,
    pub description: Option<String>,
    pub read_only: bool,
    /// JSON Schema for the arguments object. Empty object if the server
    /// did not provide one.
    pub input_schema: Value,
}

impl ListedTool {
    /// Tools without an explicit `readOnlyHint: true` need the person's approval.
    pub fn needs_approval(&self) -> bool {
        !self.read_only
    }
}

/// How to reach an MCP server. Secrets belong in `env` (stdio) or `headers` (HTTP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectConfig {
    pub transport: TransportKind,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Stdio,
    Http,
}

/// Talks to an MCP server. Implementations must not put secret values in
/// returned error text.
pub trait McpConnector: Send + Sync {
    fn list_tools(
        &self,
        cfg: &ConnectConfig,
    ) -> impl std::future::Future<Output = Result<Vec<ListedTool>, String>> + Send;

    fn call_tool(
        &self,
        cfg: &ConnectConfig,
        name: &str,
        arguments: &Value,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
}

/// The official `rmcp` client.
#[derive(Debug, Default, Clone, Copy)]
pub struct RmcpConnector;

impl McpConnector for RmcpConnector {
    async fn list_tools(&self, cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
        let secrets = secret_values(cfg);
        match timeout(MCP_TIMEOUT, list_tools_inner(cfg)).await {
            Ok(Ok(tools)) => Ok(tools),
            Ok(Err(err)) => Err(redact(&err, &secrets)),
            Err(_) => Err("连接超时".into()),
        }
    }

    async fn call_tool(
        &self,
        cfg: &ConnectConfig,
        name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        let secrets = secret_values(cfg);
        match timeout(MCP_TIMEOUT, call_tool_inner(cfg, name, arguments)).await {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(err)) => Err(redact(&err, &secrets)),
            Err(_) => Err("连接超时".into()),
        }
    }
}

pub(crate) fn secret_values(cfg: &ConnectConfig) -> Vec<String> {
    cfg.env
        .iter()
        .map(|(_, v)| v.clone())
        .chain(cfg.headers.iter().map(|(_, v)| v.clone()))
        .filter(|v| !v.is_empty())
        .collect()
}

pub(crate) fn redact(err: &str, secrets: &[String]) -> String {
    let mut out = err.to_string();
    for secret in secrets {
        if !secret.is_empty() && out.contains(secret) {
            out = out.replace(secret, "***");
        }
    }
    out
}

async fn list_tools_inner(cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
    match cfg.transport {
        TransportKind::Stdio => serve_and_list(stdio_transport(cfg)?).await,
        TransportKind::Http => {
            let url = cfg.url.as_deref().ok_or_else(|| "请填写地址".to_string())?;
            let mut headers = HashMap::new();
            for (name, value) in &cfg.headers {
                let name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| format!("无效的请求头名称：{name}"))?;
                let value =
                    HeaderValue::from_str(value).map_err(|_| "请求头的值无效".to_string())?;
                headers.insert(name, value);
            }
            let config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
            serve_and_list(StreamableHttpClientTransport::from_config(config)).await
        }
    }
}

async fn call_tool_inner(
    cfg: &ConnectConfig,
    name: &str,
    arguments: &Value,
) -> Result<String, String> {
    match cfg.transport {
        TransportKind::Stdio => serve_and_call(stdio_transport(cfg)?, name, arguments).await,
        TransportKind::Http => {
            let url = cfg.url.as_deref().ok_or_else(|| "请填写地址".to_string())?;
            let mut headers = HashMap::new();
            for (name, value) in &cfg.headers {
                let name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| format!("无效的请求头名称：{name}"))?;
                let value =
                    HeaderValue::from_str(value).map_err(|_| "请求头的值无效".to_string())?;
                headers.insert(name, value);
            }
            let config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
            serve_and_call(
                StreamableHttpClientTransport::from_config(config),
                name,
                arguments,
            )
            .await
        }
    }
}

pub(crate) fn stdio_transport(cfg: &ConnectConfig) -> Result<TokioChildProcess, String> {
    let command = cfg
        .command
        .as_deref()
        .ok_or_else(|| "请填写启动命令".to_string())?;
    let args = cfg.args.clone();
    let env = cfg.env.clone();
    TokioChildProcess::new(Command::new(command).configure(|cmd| {
        cmd.args(&args);
        // A child left behind by a dropped client (or the host exiting) is killed.
        cmd.kill_on_drop(true);
        for (k, v) in &env {
            cmd.env(k, v);
        }
    }))
    .map_err(|err| format!("无法启动 MCP 进程：{err}"))
}

async fn serve_and_list<T, E, A>(transport: T) -> Result<Vec<ListedTool>, String>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let client = client_service()
        .serve(transport)
        .await
        .map_err(|err| format!("连不上 MCP 服务器：{err}"))?;
    let result = client
        .list_tools(Default::default())
        .await
        .map_err(|err| format!("无法列出工具：{err}"));
    let _ = client.cancel().await;
    Ok(listed_tools(result?))
}

pub(crate) fn listed_tools(listed: rmcp::model::ListToolsResult) -> Vec<ListedTool> {
    listed
        .tools
        .into_iter()
        .map(|tool| ListedTool {
            name: tool.name.to_string(),
            description: tool.description.map(|d| d.to_string()),
            read_only: tool
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint)
                .unwrap_or(false),
            input_schema: Value::Object(tool.input_schema.as_ref().clone()),
        })
        .collect()
}

async fn serve_and_call<T, E, A>(
    transport: T,
    name: &str,
    arguments: &Value,
) -> Result<String, String>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let client = client_service()
        .serve(transport)
        .await
        .map_err(|err| format!("连不上 MCP 服务器：{err}"))?;
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(obj) = arguments.as_object() {
        params = params.with_arguments(obj.clone());
    }
    let result = client
        .call_tool(params)
        .await
        .map_err(|err| format!("调用失败：{err}"));
    let _ = client.cancel().await;
    Ok(format_tool_result(&result?))
}

pub(crate) fn format_tool_result(result: &CallToolResult) -> String {
    let mut texts: Vec<String> = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect();
    if texts.is_empty() {
        if let Some(value) = &result.structured_content {
            texts.push(value.to_string());
        }
    }
    let body = texts.join("\n");
    if result.is_error == Some(true) {
        if body.is_empty() {
            "调用失败".into()
        } else {
            format!("调用失败：{body}")
        }
    } else {
        body
    }
}

pub(crate) fn client_service() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("robroz-host", env!("CARGO_PKG_VERSION")),
    )
}

/// A key used by [`FakeConnector`] to look up a server: HTTP `url`, or stdio `command`.
pub fn server_key(cfg: &ConnectConfig) -> String {
    match cfg.transport {
        TransportKind::Http => cfg.url.clone().unwrap_or_default(),
        TransportKind::Stdio => cfg.command.clone().unwrap_or_default(),
    }
}

/// In-memory connector for tests. Does not start a real MCP server.
#[derive(Clone, Default)]
pub struct FakeConnector {
    inner: Arc<Mutex<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    /// `Ok` lists the tools; `Err` means the server is offline / unreachable.
    servers: HashMap<String, Result<Vec<ListedTool>, String>>,
    results: HashMap<(String, String), Result<String, String>>,
    calls: Vec<FakeCall>,
}

/// One `call_tool` recorded by [`FakeConnector`].
#[derive(Debug, Clone, PartialEq)]
pub struct FakeCall {
    pub server: String,
    pub tool: String,
    pub arguments: Value,
}

impl FakeConnector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register tools for a server key (`url` or `command`).
    pub fn set_tools(&self, key: impl Into<String>, tools: Vec<ListedTool>) {
        self.inner
            .lock()
            .unwrap()
            .servers
            .insert(key.into(), Ok(tools));
    }

    /// Subsequent [`McpConnector::list_tools`] for this key fails, so the host skips it.
    pub fn set_offline(&self, key: impl Into<String>, error: impl Into<String>) {
        self.inner
            .lock()
            .unwrap()
            .servers
            .insert(key.into(), Err(error.into()));
    }

    pub fn set_result(
        &self,
        server: impl Into<String>,
        tool: impl Into<String>,
        result: Result<String, String>,
    ) {
        self.inner
            .lock()
            .unwrap()
            .results
            .insert((server.into(), tool.into()), result);
    }

    pub fn calls(&self) -> Vec<FakeCall> {
        self.inner.lock().unwrap().calls.clone()
    }

    /// Synchronous [`McpConnector::list_tools`], for host tests that cannot
    /// `block_in_place` on a current-thread runtime.
    pub fn list_tools_sync(&self, cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
        let key = server_key(cfg);
        match self.inner.lock().unwrap().servers.get(&key) {
            Some(Ok(tools)) => Ok(tools.clone()),
            Some(Err(err)) => Err(err.clone()),
            None => Err("服务器离线".into()),
        }
    }

    /// Synchronous [`McpConnector::call_tool`].
    pub fn call_tool_sync(
        &self,
        cfg: &ConnectConfig,
        name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        let key = server_key(cfg);
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(FakeCall {
            server: key.clone(),
            tool: name.to_string(),
            arguments: arguments.clone(),
        });
        match inner.results.get(&(key, name.to_string())) {
            Some(Ok(text)) => Ok(text.clone()),
            Some(Err(err)) => Err(err.clone()),
            None => Ok(String::new()),
        }
    }
}

impl McpConnector for FakeConnector {
    async fn list_tools(&self, cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
        self.list_tools_sync(cfg)
    }

    async fn call_tool(
        &self,
        cfg: &ConnectConfig,
        name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        self.call_tool_sync(cfg, name, arguments)
    }
}

#[cfg(feature = "fixture")]
pub mod fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listed(name: &str, read_only: bool) -> ListedTool {
        ListedTool {
            name: name.into(),
            description: None,
            read_only,
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn needs_approval_is_false_only_when_read_only() {
        let ro = listed("ping", true);
        let rw = listed("write", false);
        assert!(!ro.needs_approval());
        assert!(rw.needs_approval());
    }

    #[test]
    fn redact_replaces_secret_values() {
        assert_eq!(
            redact("bad key secret-xyz", &["secret-xyz".into()]),
            "bad key ***"
        );
    }

    #[test]
    fn format_tool_result_joins_text_and_marks_errors() {
        let ok = CallToolResult::success(vec![
            rmcp::model::ContentBlock::text("a"),
            rmcp::model::ContentBlock::text("b"),
        ]);
        assert_eq!(format_tool_result(&ok), "a\nb");
        let err = CallToolResult::error(vec![rmcp::model::ContentBlock::text("nope")]);
        assert_eq!(format_tool_result(&err), "调用失败：nope");
    }

    #[tokio::test]
    async fn fake_connector_records_calls_and_can_be_offline() {
        let fake = FakeConnector::new();
        fake.set_tools(
            "http://mcp",
            vec![listed("ping", true), listed("write_note", false)],
        );
        fake.set_offline("http://down", "boom");
        fake.set_result("http://mcp", "ping", Ok("pong".into()));
        let cfg = ConnectConfig {
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            url: Some("http://mcp".into()),
            env: vec![],
            headers: vec![],
        };
        let tools = fake.list_tools(&cfg).await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(
            fake.call_tool(&cfg, "ping", &json!({"n": 1}))
                .await
                .unwrap(),
            "pong"
        );
        assert_eq!(fake.calls()[0].tool, "ping");
        let down = ConnectConfig {
            url: Some("http://down".into()),
            ..cfg
        };
        assert_eq!(fake.list_tools(&down).await.unwrap_err(), "boom");
    }
}
