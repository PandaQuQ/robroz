//! [`McpPool`] against the real stdio fixture server.

use std::time::{Duration, Instant};

use robroz_mcp::{fixture, ConnectConfig, McpPool, PoolConfig, TransportKind};
use serde_json::json;

fn stdio_cfg() -> ConnectConfig {
    ConnectConfig {
        transport: TransportKind::Stdio,
        command: Some(env!("CARGO_BIN_EXE_robroz-mcp-fixture").to_string()),
        args: vec![],
        url: None,
        env: vec![],
        headers: vec![],
    }
}

/// True while `pid` is a running (not zombie) process.
#[cfg(target_os = "linux")]
fn running(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // Field 3, after the parenthesised command name, is the state.
        Ok(stat) => stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z" && state != "X"),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
async fn wait_gone(pid: u32) -> bool {
    for _ in 0..100 {
        if !running(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn second_task_reuses_the_process_and_the_tool_list() {
    let pool = McpPool::new(PoolConfig::default());
    let cfg = stdio_cfg();
    let t0 = Instant::now();
    let tools = pool.list_tools("srv", "g", &cfg).await.unwrap();
    let cold = t0.elapsed();
    assert!(tools.iter().any(|t| t.name == fixture::READ_ONLY_TOOL));
    let pid = pool.stdio().live_pid("srv", "g").await.expect("live pid");

    let t1 = Instant::now();
    let again = pool.list_tools("srv", "g", &cfg).await.unwrap();
    let warm = t1.elapsed();
    assert_eq!(tools, again);

    let t2 = Instant::now();
    let pong = pool
        .call_tool("srv", "g", &cfg, fixture::READ_ONLY_TOOL, &json!({}))
        .await
        .unwrap();
    let call = t2.elapsed();
    assert_eq!(pong, "pong");
    assert_eq!(
        pool.call_tool("srv", "g", &cfg, fixture::WRITE_TOOL, &json!({}))
            .await
            .unwrap(),
        "written"
    );
    assert_eq!(pool.stdio().spawn_count(), 1);
    assert_eq!(pool.stdio().live_pid("srv", "g").await, Some(pid));
    eprintln!("pool latency: cold tools/list {cold:?}, warm tools/list {warm:?}, warm tools/call {call:?}");
    pool.shutdown(Duration::from_secs(5)).await;
}

/// The host's step threads block on a new runtime for every call. Processes
/// must outlive those runtimes.
#[test]
fn processes_outlive_the_callers_runtime() {
    let pool = std::sync::Arc::new(McpPool::new(PoolConfig::default()));
    let mut pids = Vec::new();
    for _ in 0..3 {
        let pool = pool.clone();
        let pid = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let out = pool
                    .call_tool(
                        "srv",
                        "g",
                        &stdio_cfg(),
                        fixture::READ_ONLY_TOOL,
                        &json!({}),
                    )
                    .await
                    .unwrap();
                assert_eq!(out, "pong");
                pool.stdio().live_pid("srv", "g").await
            })
        })
        .join()
        .unwrap();
        pids.push(pid.expect("pid"));
    }
    assert!(pids.windows(2).all(|w| w[0] == w[1]), "{pids:?}");
    assert_eq!(pool.stdio().spawn_count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn config_change_starts_a_new_process_and_stops_the_old_one() {
    let pool = McpPool::new(PoolConfig::default());
    let cfg = stdio_cfg();
    pool.list_tools("srv", "g", &cfg).await.unwrap();
    let old = pool.stdio().live_pid("srv", "g").await.unwrap();
    let rotated = ConnectConfig {
        env: vec![("FIXTURE_KEY".into(), "rotated".into())],
        ..cfg
    };
    pool.list_tools("srv", "g", &rotated).await.unwrap();
    let new = pool.stdio().live_pid("srv", "g").await.unwrap();
    assert_ne!(old, new);
    assert_eq!(pool.stdio().spawn_count(), 2);
    #[cfg(target_os = "linux")]
    assert!(wait_gone(old).await, "old process {old} still running");
    pool.shutdown(Duration::from_secs(5)).await;
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn reconnects_after_the_child_is_killed() {
    let pool = McpPool::new(PoolConfig::default());
    let cfg = stdio_cfg();
    pool.list_tools("srv", "g", &cfg).await.unwrap();
    let pid = pool.stdio().live_pid("srv", "g").await.unwrap();
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(wait_gone(pid).await);
    // Give the client a moment to see EOF; a call made before that is
    // covered by the broken-pipe path.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let out = pool
        .call_tool("srv", "g", &cfg, fixture::READ_ONLY_TOOL, &json!({}))
        .await
        .unwrap();
    assert_eq!(out, "pong");
    let new = pool.stdio().live_pid("srv", "g").await.unwrap();
    assert_ne!(pid, new);
    pool.shutdown(Duration::from_secs(5)).await;
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn idle_process_is_stopped_by_the_reaper() {
    let pool = McpPool::new(PoolConfig {
        idle_timeout: Duration::from_millis(200),
        reap_interval: Duration::from_millis(50),
        ..PoolConfig::default()
    });
    pool.list_tools("srv", "g", &stdio_cfg()).await.unwrap();
    let pid = pool.stdio().live_pid("srv", "g").await.unwrap();
    assert!(running(pid));
    assert!(wait_gone(pid).await, "idle process {pid} still running");
    assert_eq!(pool.stdio().live_count().await, 0);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_stops_children_and_refuses_new_ones() {
    let pool = McpPool::new(PoolConfig::default());
    pool.list_tools("a", "g", &stdio_cfg()).await.unwrap();
    pool.list_tools("b", "g", &stdio_cfg()).await.unwrap();
    let a = pool.stdio().live_pid("a", "g").await.unwrap();
    let b = pool.stdio().live_pid("b", "g").await.unwrap();
    pool.shutdown(Duration::from_secs(5)).await;
    assert!(wait_gone(a).await && wait_gone(b).await);
    let err = pool.list_tools("a", "g", &stdio_cfg()).await.unwrap_err();
    assert_eq!(err, robroz_mcp::pool::POOL_CLOSED);
}

#[cfg(target_os = "linux")]
#[test]
fn dropping_the_pool_kills_children() {
    let pool = McpPool::new(PoolConfig::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let pid = rt.block_on(async {
        pool.list_tools("srv", "g", &stdio_cfg()).await.unwrap();
        pool.stdio().live_pid("srv", "g").await.unwrap()
    });
    drop(pool);
    assert!(rt.block_on(wait_gone(pid)), "child {pid} outlived the pool");
}

#[tokio::test(flavor = "multi_thread")]
async fn pooling_off_starts_a_process_per_request() {
    let pool = McpPool::new(PoolConfig {
        idle_timeout: Duration::ZERO,
        ..PoolConfig::default()
    });
    let cfg = stdio_cfg();
    pool.list_tools("srv", "g", &cfg).await.unwrap();
    pool.list_tools("srv", "g", &cfg).await.unwrap();
    assert_eq!(pool.stdio().spawn_count(), 0, "the pool is bypassed");
    assert_eq!(pool.stdio().live_pid("srv", "g").await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_servers_are_not_pooled() {
    let server = fixture::serve_http().await.unwrap();
    let pool = McpPool::new(PoolConfig::default());
    let cfg = ConnectConfig {
        transport: TransportKind::Http,
        command: None,
        args: vec![],
        url: Some(server.url.clone()),
        env: vec![],
        headers: vec![],
    };
    assert_eq!(
        pool.call_tool("h", "g", &cfg, fixture::READ_ONLY_TOOL, &json!({}))
            .await
            .unwrap(),
        "pong"
    );
    assert_eq!(pool.stdio().spawn_count(), 0);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn two_groups_get_different_pids_and_the_same_group_reuses_one() {
    let pool = McpPool::new(PoolConfig::default());
    let cfg = stdio_cfg();
    pool.list_tools("srv", "ga", &cfg).await.unwrap();
    let a = pool.stdio().live_pid("srv", "ga").await.unwrap();
    pool.list_tools("srv", "gb", &cfg).await.unwrap();
    let b = pool.stdio().live_pid("srv", "gb").await.unwrap();
    assert_ne!(a, b, "groups must not share a process");
    pool.list_tools("srv", "ga", &cfg).await.unwrap();
    assert_eq!(pool.stdio().live_pid("srv", "ga").await, Some(a));
    assert_eq!(pool.stdio().spawn_count(), 2);
    pool.evict("srv");
    // evict is async in the background via the pool runtime
    for _ in 0..40 {
        if !running(a) && !running(b) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!running(a) && !running(b), "evict must stop every group");
    pool.shutdown(Duration::from_secs(5)).await;
}
