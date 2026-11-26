//! [`McpPool`]: the production MCP client with pooled stdio connections.
//!
//! Stdio servers go through a [`StdioPool`] that runs on a small tokio
//! runtime owned by the pool, so a caller that blocks on a throwaway runtime
//! per call (the host's step threads do) still reuses processes. HTTP servers
//! are not pooled and go through [`RmcpConnector`] on the caller's runtime.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use rmcp::service::{Peer, RunningService, ServiceError};
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use tokio::runtime::{Handle, Runtime};

use crate::pool::{
    BoxFuture, CallError, PoolConfig, StdioConnection, StdioPool, StdioSpawner, POOL_CLOSED,
};
use crate::{
    client_service, format_tool_result, listed_tools, redact, secret_values, stdio_transport,
    ConnectConfig, ListedTool, McpConnector, RmcpConnector, TransportKind,
};

/// Starts stdio servers with the official `rmcp` client.
#[derive(Debug, Default, Clone, Copy)]
pub struct RmcpStdioSpawner;

impl StdioSpawner for RmcpStdioSpawner {
    fn spawn<'a>(
        &'a self,
        cfg: &'a ConnectConfig,
    ) -> BoxFuture<'a, Result<Arc<dyn StdioConnection>, String>> {
        Box::pin(async move {
            let transport = stdio_transport(cfg)?;
            let pid = transport.id();
            let service = client_service()
                .serve(transport)
                .await
                .map_err(|err| format!("连不上 MCP 服务器：{err}"))?;
            let peer = service.peer().clone();
            Ok(Arc::new(RmcpStdio {
                peer,
                service: tokio::sync::Mutex::new(Some(service)),
                pid,
            }) as Arc<dyn StdioConnection>)
        })
    }
}

struct RmcpStdio {
    peer: Peer<RoleClient>,
    service: tokio::sync::Mutex<Option<RunningService<RoleClient, rmcp::model::ClientConfig>>>,
    pid: Option<u32>,
}

fn classify(err: ServiceError, prefix: &str) -> CallError {
    let broken = matches!(
        err,
        ServiceError::TransportClosed
            | ServiceError::TransportSend(_)
            | ServiceError::Cancelled { .. }
    );
    let msg = format!("{prefix}{err}");
    if broken {
        CallError::Broken(msg)
    } else {
        CallError::Failed(msg)
    }
}

impl StdioConnection for RmcpStdio {
    fn list_tools(&self) -> BoxFuture<'_, Result<Vec<ListedTool>, CallError>> {
        Box::pin(async move {
            self.peer
                .list_tools(Default::default())
                .await
                .map(listed_tools)
                .map_err(|err| classify(err, "无法列出工具："))
        })
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<String, CallError>> {
        Box::pin(async move {
            let mut params = CallToolRequestParams::new(name.to_string());
            if let Some(obj) = arguments.as_object() {
                params = params.with_arguments(obj.clone());
            }
            self.peer
                .call_tool(params)
                .await
                .map(|result| format_tool_result(&result))
                .map_err(|err| classify(err, "调用失败："))
        })
    }

    fn is_alive(&self) -> bool {
        !self.peer.is_transport_closed()
            && self
                .service
                .try_lock()
                .map(|s| s.as_ref().is_some_and(|s| !s.is_closed()))
                // Locked means `close` is running.
                .unwrap_or(false)
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(service) = self.service.lock().await.take() {
                // Closes stdin, waits for the child, then terminates it.
                let _ = service.cancel().await;
            }
        })
    }
}

/// The production MCP client. See the module docs.
pub struct McpPool {
    stdio: Arc<StdioPool>,
    runtime: Mutex<Option<Runtime>>,
    handle: OnceLock<Handle>,
}

impl McpPool {
    pub fn new(cfg: PoolConfig) -> Self {
        Self::with_spawner(cfg, Arc::new(RmcpStdioSpawner))
    }

    /// Use another spawner, for tests.
    pub fn with_spawner(cfg: PoolConfig, spawner: Arc<dyn StdioSpawner>) -> Self {
        Self {
            stdio: Arc::new(StdioPool::new(cfg, spawner)),
            runtime: Mutex::new(None),
            handle: OnceLock::new(),
        }
    }

    pub fn config(&self) -> PoolConfig {
        self.stdio.config()
    }

    /// The stdio pool, for diagnostics and tests.
    pub fn stdio(&self) -> &Arc<StdioPool> {
        &self.stdio
    }

    /// The pool's own runtime, started on first use with the idle reaper.
    fn handle(&self) -> Handle {
        self.handle
            .get_or_init(|| {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("robroz-mcp")
                    .enable_all()
                    .build()
                    .expect("tokio runtime for MCP");
                let handle = runtime.handle().clone();
                spawn_reaper(&handle, Arc::downgrade(&self.stdio));
                *self.runtime.lock().unwrap() = Some(runtime);
                handle
            })
            .clone()
    }

    fn started(&self) -> Option<Handle> {
        self.handle.get().cloned()
    }

    async fn on_pool<T: Send + 'static>(
        &self,
        fut: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
    ) -> Result<T, String> {
        match self.handle().spawn(fut).await {
            Ok(result) => result,
            // The pool's runtime is shutting down, or the request panicked.
            Err(err) if err.is_cancelled() => Err(POOL_CLOSED.into()),
            Err(err) => Err(format!("连接中断：{err}")),
        }
    }

    /// `tools/list` for (`server_id`, `group_id`).
    pub async fn list_tools(
        &self,
        server_id: &str,
        group_id: &str,
        cfg: &ConnectConfig,
    ) -> Result<Vec<ListedTool>, String> {
        if cfg.transport == TransportKind::Http || !self.config().pooling() {
            return RmcpConnector.list_tools(cfg).await;
        }
        let secrets = secret_values(cfg);
        let pool = self.stdio.clone();
        let (id, gid, cfg) = (server_id.to_string(), group_id.to_string(), cfg.clone());
        self.on_pool(async move { pool.list_tools(&id, &gid, &cfg).await })
            .await
            .map_err(|err| redact(&err, &secrets))
    }

    /// `tools/call` for (`server_id`, `group_id`).
    pub async fn call_tool(
        &self,
        server_id: &str,
        group_id: &str,
        cfg: &ConnectConfig,
        name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        if cfg.transport == TransportKind::Http || !self.config().pooling() {
            return RmcpConnector.call_tool(cfg, name, arguments).await;
        }
        let secrets = secret_values(cfg);
        let pool = self.stdio.clone();
        let (id, gid, cfg, name, arguments) = (
            server_id.to_string(),
            group_id.to_string(),
            cfg.clone(),
            name.to_string(),
            arguments.clone(),
        );
        self.on_pool(async move { pool.call_tool(&id, &gid, &cfg, &name, &arguments).await })
            .await
            .map_err(|err| redact(&err, &secrets))
    }

    /// Stop every process for `server_id` (all groups) in the background.
    /// Does nothing before the pool has started one.
    pub fn evict(&self, server_id: &str) {
        if let Some(handle) = self.started() {
            let pool = self.stdio.clone();
            let id = server_id.to_string();
            handle.spawn(async move { pool.evict(&id).await });
        }
    }

    /// Stop every process started for `group_id`. Ready for a later
    /// delete-group route; nothing calls this yet.
    pub fn evict_group(&self, group_id: &str) {
        if let Some(handle) = self.started() {
            let pool = self.stdio.clone();
            let id = group_id.to_string();
            handle.spawn(async move { pool.evict_group(&id).await });
        }
    }

    /// Stop every pooled process and refuse new ones. Waits at most `grace`.
    pub async fn shutdown(&self, grace: Duration) {
        let Some(handle) = self.started() else {
            return;
        };
        let pool = self.stdio.clone();
        let join = handle.spawn(async move { pool.shutdown().await });
        let _ = tokio::time::timeout(grace, join).await;
    }
}

impl Drop for McpPool {
    fn drop(&mut self) {
        // Safe to call from inside another runtime. Children not stopped by
        // `shutdown` are killed on drop (`kill_on_drop`).
        if let Some(runtime) = self.runtime.lock().unwrap().take() {
            runtime.shutdown_background();
        }
    }
}

fn spawn_reaper(handle: &Handle, pool: Weak<StdioPool>) {
    let Some(every) = pool.upgrade().map(|p| p.config().reap_interval) else {
        return;
    };
    let every = every.max(Duration::from_millis(10));
    handle.spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(pool) = pool.upgrade() else {
                break;
            };
            pool.reap_idle(Instant::now()).await;
        }
    });
}
