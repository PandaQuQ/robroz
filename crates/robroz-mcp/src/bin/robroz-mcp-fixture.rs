//! Test-only MCP server. Speaks stdio. Not part of the host's public tools.
#[tokio::main]
async fn main() {
    if let Err(err) = robroz_mcp::fixture::serve_stdio().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
