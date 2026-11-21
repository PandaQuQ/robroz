//! MCP client used by the host: list tools over stdio or streamable HTTP.
//!
//! The host talks to servers through [`McpConnector`]. Production uses
//! [`RmcpConnector`] (the official `rmcp` crate). Tests can swap in a fake
//! or start the bundled fixture server.

use std::collections::HashMap;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;
use tokio::process::Command;
use tokio::time::timeout;

/// How long a connect + `tools/list` is allowed to take.
pub const MCP_TIMEOUT: Duration = Duration::from_secs(10);

/// A tool as the host shows it: approval is required unless the server
/// marked the tool `readOnlyHint: true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTool {
    pub name: String,
    pub description: Option<String>,
    pub read_only: bool,
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
}

fn secret_values(cfg: &ConnectConfig) -> Vec<String> {
    cfg.env
        .iter()
        .map(|(_, v)| v.clone())
        .chain(cfg.headers.iter().map(|(_, v)| v.clone()))
        .filter(|v| !v.is_empty())
        .collect()
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

async fn list_tools_inner(cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
    match cfg.transport {
        TransportKind::Stdio => list_via_stdio(cfg).await,
        TransportKind::Http => list_via_http(cfg).await,
    }
}

async fn list_via_stdio(cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
    let command = cfg
        .command
        .as_deref()
        .ok_or_else(|| "请填写启动命令".to_string())?;
    let args = cfg.args.clone();
    let env = cfg.env.clone();
    let transport = TokioChildProcess::new(Command::new(command).configure(|cmd| {
        cmd.args(&args);
        for (k, v) in &env {
            cmd.env(k, v);
        }
    }))
    .map_err(|err| format!("无法启动 MCP 进程：{err}"))?;
    serve_and_list(transport).await
}

async fn list_via_http(cfg: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
    let url = cfg.url.as_deref().ok_or_else(|| "请填写地址".to_string())?;
    let mut headers = HashMap::new();
    for (name, value) in &cfg.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("无效的请求头名称：{name}"))?;
        let value = HeaderValue::from_str(value).map_err(|_| "请求头的值无效".to_string())?;
        headers.insert(name, value);
    }
    let config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
    let transport = StreamableHttpClientTransport::from_config(config);
    serve_and_list(transport).await
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
    let listed = result?;
    Ok(listed
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
        })
        .collect())
}

fn client_service() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("robroz-host", "0.1.0"),
    )
}

#[cfg(feature = "fixture")]
pub mod fixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_approval_is_false_only_when_read_only() {
        let ro = ListedTool {
            name: "ping".into(),
            description: None,
            read_only: true,
        };
        let rw = ListedTool {
            name: "write".into(),
            description: None,
            read_only: false,
        };
        assert!(!ro.needs_approval());
        assert!(rw.needs_approval());
    }

    #[test]
    fn redact_replaces_secret_values() {
        assert_eq!(
            redact("bad key secret-xyz", &["secret-xyz".into()]),
            "bad key ***"
        );
        assert_eq!(
            redact("no secrets here", &["abc".into()]),
            "no secrets here"
        );
    }
}
