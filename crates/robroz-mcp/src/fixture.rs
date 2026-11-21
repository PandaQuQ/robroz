//! A tiny MCP server used only in tests: one read-only tool and one that
//! needs approval. Serves stdio (the `robroz-mcp-fixture` binary) or
//! streamable HTTP (in-process).

use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// The fixture exposes these two tools. `ping` is read-only; `write_note` is not.
pub const READ_ONLY_TOOL: &str = "ping";
pub const WRITE_TOOL: &str = "write_note";

#[derive(Clone)]
pub struct FixtureServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl FixtureServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Check the server is up",
        annotations(read_only_hint = true)
    )]
    fn ping(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text("pong")]))
    }

    #[tool(description = "Write a note")]
    fn write_note(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text("written")]))
    }
}

impl Default for FixtureServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for FixtureServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("robroz-mcp-fixture", "0.1.0"))
            .with_instructions("Test fixture with ping (read-only) and write_note.".to_string())
    }
}

/// Serve the fixture on stdio until the client closes the connection.
pub async fn serve_stdio() -> Result<(), String> {
    let server = FixtureServer::new()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|err| format!("fixture stdio: {err}"))?;
    server
        .waiting()
        .await
        .map_err(|err| format!("fixture stdio: {err}"))?;
    Ok(())
}

/// A running in-process streamable HTTP fixture. Dropping it aborts the task.
pub struct RunningHttp {
    pub url: String,
    pub addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl Drop for RunningHttp {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Bind `127.0.0.1:0` and serve `/mcp`.
pub async fn serve_http() -> Result<RunningHttp, String> {
    let service = StreamableHttpService::new(
        || Ok(FixtureServer::new()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|err| format!("fixture bind: {err}"))?;
    let addr = listener
        .local_addr()
        .map_err(|err| format!("fixture addr: {err}"))?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(RunningHttp {
        url: format!("http://{addr}/mcp"),
        addr,
        handle,
    })
}
