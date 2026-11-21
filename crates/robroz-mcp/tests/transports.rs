use robroz_mcp::{fixture, ConnectConfig, ListedTool, McpConnector, RmcpConnector, TransportKind};

fn expect_fixture_tools(tools: &[ListedTool]) {
    let ping = tools
        .iter()
        .find(|t| t.name == fixture::READ_ONLY_TOOL)
        .expect("ping");
    let write = tools
        .iter()
        .find(|t| t.name == fixture::WRITE_TOOL)
        .expect("write_note");
    assert_eq!(ping.description.as_deref(), Some("Check the server is up"));
    assert!(ping.read_only, "{ping:?}");
    assert!(!ping.needs_approval());
    assert_eq!(write.description.as_deref(), Some("Write a note"));
    assert!(!write.read_only, "{write:?}");
    assert!(write.needs_approval());
}

#[tokio::test]
async fn lists_tools_over_http() {
    let server = fixture::serve_http().await.unwrap();
    let tools = RmcpConnector
        .list_tools(&ConnectConfig {
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            url: Some(server.url.clone()),
            env: vec![],
            headers: vec![],
        })
        .await
        .unwrap();
    expect_fixture_tools(&tools);
}

#[tokio::test]
async fn lists_tools_over_stdio() {
    let bin = env!("CARGO_BIN_EXE_robroz-mcp-fixture");
    let tools = RmcpConnector
        .list_tools(&ConnectConfig {
            transport: TransportKind::Stdio,
            command: Some(bin.to_string()),
            args: vec![],
            url: None,
            env: vec![],
            headers: vec![],
        })
        .await
        .unwrap();
    expect_fixture_tools(&tools);
}
