//! Keep stdio MCP servers running between tasks.
//!
//! Without a pool every `tools/list` and `tools/call` starts the server's
//! command, does the MCP handshake and stops it again. [`StdioPool`] keeps one
//! live connection per server id instead:
//!
//! - The connection is tied to a hash of everything that changes how the
//!   process behaves (command, args, env, which includes the secrets). When the
//!   hash changes (the command was edited or a key was rotated), the old
//!   process is stopped and a new one is started on the next use.
//! - `tools/list` is cached for the life of one connection. A new process, for
//!   whatever reason, lists again.
//! - Before each use the pool checks the connection is still up. A child that
//!   exited, or a broken pipe, is replaced on the next use. `tools/list` is
//!   retried once on a fresh process; `tools/call` is never retried, because a
//!   write tool may already have run.
//! - A request that hits the timeout stops the process, the same as before the
//!   pool existed, so a hung server does not stay around.
//! - Each connection takes at most [`PoolConfig::max_in_flight`] requests at a
//!   time. `1` serialises calls per server.
//! - [`StdioPool::reap_idle`] stops connections unused for
//!   [`PoolConfig::idle_timeout`]. [`StdioPool::shutdown`] stops them all.
//!
//! The pool never decides whether a call is allowed. Approval and metering
//! happen in the host before a call reaches it, exactly as without a pool.
//! Secret values are only hashed, never logged; error text is redacted by the
//! caller with the same rule as [`crate::RmcpConnector`].
//!
//! [`StdioPool`] runs on whatever tokio runtime calls it, and the processes it
//! starts belong to that runtime. [`crate::McpPool`] wraps it with a runtime of
//! its own so callers that build a short-lived runtime per call still share
//! processes.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

use crate::{ConnectConfig, ListedTool, TransportKind, MCP_TIMEOUT};

/// A boxed `Send` future, so [`StdioConnection`] and [`StdioSpawner`] stay object safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Text returned when the host is shutting down and the pool no longer starts processes.
pub const POOL_CLOSED: &str = "宿主机正在关闭";
/// Text returned when a request or a process start runs past the timeout.
/// Same wording as [`crate::RmcpConnector`].
pub const TIMED_OUT: &str = "连接超时";

/// Pool tuning. See [`PoolConfig::default`] for the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    /// Stop a connection that has not been used for this long.
    /// [`Duration::ZERO`] turns pooling off: every request starts its own process.
    pub idle_timeout: Duration,
    /// Limit for starting a process plus the MCP handshake, and separately for
    /// each request, and for waiting for a free slot on a busy connection.
    pub call_timeout: Duration,
    /// Requests one connection takes at the same time. `1` serialises per server.
    pub max_in_flight: usize,
    /// How often [`crate::McpPool`] looks for idle connections.
    pub reap_interval: Duration,
}

impl Default for PoolConfig {
    /// 10 minutes idle, [`MCP_TIMEOUT`] per request, 4 requests in flight, reap every 30 s.
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(600),
            call_timeout: MCP_TIMEOUT,
            max_in_flight: 4,
            reap_interval: Duration::from_secs(30),
        }
    }
}

impl PoolConfig {
    /// Whether processes are kept between requests at all.
    pub fn pooling(&self) -> bool {
        !self.idle_timeout.is_zero()
    }
}

/// Why a request on a live connection failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// The process is gone or the pipe broke. The pool drops the connection.
    Broken(String),
    /// The server answered with an error. The connection stays.
    Failed(String),
}

impl CallError {
    fn into_message(self) -> String {
        match self {
            CallError::Broken(msg) | CallError::Failed(msg) => msg,
        }
    }
}

/// One running MCP server process with a finished handshake.
pub trait StdioConnection: Send + Sync {
    fn list_tools(&self) -> BoxFuture<'_, Result<Vec<ListedTool>, CallError>>;
    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<String, CallError>>;
    /// False once the child exited or the transport closed.
    fn is_alive(&self) -> bool;
    /// The child's process id, when known.
    fn pid(&self) -> Option<u32>;
    /// Stop the process: close stdin, then terminate it if it does not exit.
    fn close(&self) -> BoxFuture<'_, ()>;
}

/// Starts a process for a config and does the MCP handshake.
pub trait StdioSpawner: Send + Sync + 'static {
    fn spawn<'a>(
        &'a self,
        cfg: &'a ConnectConfig,
    ) -> BoxFuture<'a, Result<Arc<dyn StdioConnection>, String>>;
}

/// Hash of the fields that change how a server behaves. Secret values go
/// into the hash and nowhere else; the hash itself is never logged.
pub fn config_hash(cfg: &ConnectConfig) -> String {
    fn field(h: &mut Sha256, bytes: &[u8]) {
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    let mut h = Sha256::new();
    field(
        &mut h,
        match cfg.transport {
            TransportKind::Stdio => b"stdio",
            TransportKind::Http => b"http",
        },
    );
    field(&mut h, cfg.command.as_deref().unwrap_or("").as_bytes());
    field(&mut h, &(cfg.args.len() as u64).to_le_bytes());
    for arg in &cfg.args {
        field(&mut h, arg.as_bytes());
    }
    field(&mut h, cfg.url.as_deref().unwrap_or("").as_bytes());
    for (list, tag) in [(&cfg.env, b"env"), (&cfg.headers, b"hdr")] {
        field(&mut h, tag);
        field(&mut h, &(list.len() as u64).to_le_bytes());
        for (k, v) in list {
            field(&mut h, k.as_bytes());
            field(&mut h, v.as_bytes());
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

struct Live {
    hash: String,
    conn: Arc<dyn StdioConnection>,
    permits: Arc<Semaphore>,
    tools: Option<Vec<ListedTool>>,
    last_used: Instant,
}

#[derive(Default)]
struct Slot {
    live: Option<Live>,
}

/// One live stdio connection per server id. See the module docs.
pub struct StdioPool {
    cfg: PoolConfig,
    spawner: Arc<dyn StdioSpawner>,
    slots: StdMutex<HashMap<String, Arc<Mutex<Slot>>>>,
    closed: AtomicBool,
    spawns: AtomicU64,
}

/// What a caller got from [`StdioPool::checkout`].
struct Checkout {
    conn: Arc<dyn StdioConnection>,
    cached_tools: Option<Vec<ListedTool>>,
    fresh: bool,
}

impl StdioPool {
    pub fn new(cfg: PoolConfig, spawner: Arc<dyn StdioSpawner>) -> Self {
        Self {
            cfg,
            spawner,
            slots: StdMutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            spawns: AtomicU64::new(0),
        }
    }

    pub fn config(&self) -> PoolConfig {
        self.cfg
    }

    /// Processes started so far, for tests and diagnostics.
    pub fn spawn_count(&self) -> u64 {
        self.spawns.load(Ordering::Relaxed)
    }

    /// The pid of the live process for `server_id`, if any.
    pub async fn live_pid(&self, server_id: &str) -> Option<u32> {
        let slot = self.slots.lock().unwrap().get(server_id).cloned()?;
        let guard = slot.lock().await;
        guard.live.as_ref().and_then(|live| live.conn.pid())
    }

    /// Number of servers with a live process.
    pub async fn live_count(&self) -> usize {
        let slots: Vec<_> = self.slots.lock().unwrap().values().cloned().collect();
        let mut n = 0;
        for slot in slots {
            if slot.lock().await.live.is_some() {
                n += 1;
            }
        }
        n
    }

    fn slot(&self, server_id: &str) -> Arc<Mutex<Slot>> {
        self.slots
            .lock()
            .unwrap()
            .entry(server_id.to_string())
            .or_default()
            .clone()
    }

    async fn spawn_live(&self, cfg: &ConnectConfig, hash: String) -> Result<Live, String> {
        self.spawns.fetch_add(1, Ordering::Relaxed);
        let conn = match timeout(self.cfg.call_timeout, self.spawner.spawn(cfg)).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err(TIMED_OUT.into()),
        };
        Ok(Live {
            hash,
            conn,
            permits: Arc::new(Semaphore::new(self.cfg.max_in_flight.max(1))),
            tools: None,
            last_used: Instant::now(),
        })
    }

    /// Get a usable connection for `server_id`, replacing a stale, dead,
    /// idle-expired or differently-configured one. Returns a request permit.
    async fn checkout(
        &self,
        server_id: &str,
        cfg: &ConnectConfig,
        hash: &str,
    ) -> Result<(Checkout, Arc<Mutex<Slot>>, Arc<Semaphore>), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err(POOL_CLOSED.into());
        }
        let slot = self.slot(server_id);
        let mut guard = slot.lock().await;
        let now = Instant::now();
        let stale = guard.live.as_ref().is_some_and(|live| {
            live.hash != hash
                || !live.conn.is_alive()
                || now.saturating_duration_since(live.last_used) >= self.cfg.idle_timeout
        });
        if stale {
            if let Some(old) = guard.live.take() {
                self.retire(old);
            }
        }
        let fresh = guard.live.is_none();
        if fresh {
            guard.live = Some(self.spawn_live(cfg, hash.to_string()).await?);
        }
        let live = guard.live.as_mut().expect("live connection");
        live.last_used = now;
        Ok((
            Checkout {
                conn: live.conn.clone(),
                cached_tools: live.tools.clone(),
                fresh,
            },
            slot.clone(),
            live.permits.clone(),
        ))
    }

    /// Stop a replaced connection once the requests already on it are done,
    /// so editing a server does not cut off another task's call.
    fn retire(&self, old: Live) {
        let max = self.cfg.max_in_flight.max(1) as u32;
        let wait = self.cfg.call_timeout;
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // No runtime: dropping the last handle kills the child (`kill_on_drop`).
            drop(old);
            return;
        };
        handle.spawn(async move {
            let _ = timeout(wait, old.permits.acquire_many(max)).await;
            old.conn.close().await;
        });
    }

    async fn permit(&self, permits: Arc<Semaphore>) -> Result<OwnedSemaphorePermit, String> {
        match timeout(self.cfg.call_timeout, permits.acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(POOL_CLOSED.into()),
            Err(_) => Err(TIMED_OUT.into()),
        }
    }

    /// Drop `conn` from the slot if it is still the live one, and stop it.
    async fn discard(slot: &Mutex<Slot>, conn: &Arc<dyn StdioConnection>) {
        let mut guard = slot.lock().await;
        if guard
            .live
            .as_ref()
            .is_some_and(|live| Arc::ptr_eq(&live.conn, conn))
        {
            if let Some(old) = guard.live.take() {
                close_in_background(old.conn);
            }
        }
    }

    async fn touch(slot: &Mutex<Slot>, conn: &Arc<dyn StdioConnection>) {
        let mut guard = slot.lock().await;
        if let Some(live) = guard.live.as_mut() {
            if Arc::ptr_eq(&live.conn, conn) {
                live.last_used = Instant::now();
            }
        }
    }

    /// `tools/list` through the pool. Served from cache while the same
    /// process is up and its config has not changed.
    pub async fn list_tools(
        &self,
        server_id: &str,
        cfg: &ConnectConfig,
    ) -> Result<Vec<ListedTool>, String> {
        let hash = config_hash(cfg);
        for attempt in 0..2 {
            let (checkout, slot, permits) = self.checkout(server_id, cfg, &hash).await?;
            if let Some(tools) = checkout.cached_tools {
                return Ok(tools);
            }
            let permit = self.permit(permits).await?;
            let result = timeout(self.cfg.call_timeout, checkout.conn.list_tools()).await;
            drop(permit);
            match result {
                Ok(Ok(tools)) => {
                    let mut guard = slot.lock().await;
                    if let Some(live) = guard.live.as_mut() {
                        if Arc::ptr_eq(&live.conn, &checkout.conn) {
                            live.tools = Some(tools.clone());
                            live.last_used = Instant::now();
                        }
                    }
                    return Ok(tools);
                }
                Ok(Err(CallError::Failed(err))) => {
                    Self::touch(&slot, &checkout.conn).await;
                    return Err(err);
                }
                Ok(Err(CallError::Broken(err))) => {
                    Self::discard(&slot, &checkout.conn).await;
                    // A reused process may have died between tasks: listing is
                    // safe to repeat once on a new one.
                    if checkout.fresh || attempt == 1 {
                        return Err(err);
                    }
                }
                Err(_) => {
                    Self::discard(&slot, &checkout.conn).await;
                    return Err(TIMED_OUT.into());
                }
            }
        }
        unreachable!("the loop returns on the second attempt")
    }

    /// `tools/call` through the pool. Never retried: a write tool may have run.
    pub async fn call_tool(
        &self,
        server_id: &str,
        cfg: &ConnectConfig,
        name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        let hash = config_hash(cfg);
        let (checkout, slot, permits) = self.checkout(server_id, cfg, &hash).await?;
        let permit = self.permit(permits).await?;
        let result = timeout(
            self.cfg.call_timeout,
            checkout.conn.call_tool(name, arguments),
        )
        .await;
        drop(permit);
        match result {
            Ok(Ok(text)) => {
                Self::touch(&slot, &checkout.conn).await;
                Ok(text)
            }
            Ok(Err(err @ CallError::Failed(_))) => {
                Self::touch(&slot, &checkout.conn).await;
                Err(err.into_message())
            }
            Ok(Err(err @ CallError::Broken(_))) => {
                Self::discard(&slot, &checkout.conn).await;
                Err(err.into_message())
            }
            Err(_) => {
                Self::discard(&slot, &checkout.conn).await;
                Err(TIMED_OUT.into())
            }
        }
    }

    /// Stop the process for `server_id` now, for example after the server was
    /// edited, disabled or deleted.
    pub async fn evict(&self, server_id: &str) {
        let slot = self.slots.lock().unwrap().remove(server_id);
        if let Some(slot) = slot {
            if let Some(old) = slot.lock().await.live.take() {
                self.retire(old);
            }
        }
    }

    /// Stop connections idle since before `now - idle_timeout` with no request
    /// in flight. Busy slots are skipped and looked at next time. Returns how
    /// many processes were stopped.
    pub async fn reap_idle(&self, now: Instant) -> usize {
        let slots: Vec<(String, Arc<Mutex<Slot>>)> = self
            .slots
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut stopped = 0;
        for (id, slot) in slots {
            let Ok(mut guard) = slot.try_lock() else {
                continue;
            };
            let expired = guard.live.as_ref().is_some_and(|live| {
                let idle = now.saturating_duration_since(live.last_used) >= self.cfg.idle_timeout;
                let quiet = live.permits.available_permits() == self.cfg.max_in_flight.max(1);
                (idle && quiet) || !live.conn.is_alive()
            });
            if expired {
                if let Some(old) = guard.live.take() {
                    old.conn.close().await;
                    stopped += 1;
                }
            }
            if guard.live.is_none() {
                drop(guard);
                let mut slots = self.slots.lock().unwrap();
                if slots.get(&id).is_some_and(|s| Arc::ptr_eq(s, &slot)) {
                    slots.remove(&id);
                }
            }
        }
        stopped
    }

    /// Stop every process and refuse new ones. Requests already in flight
    /// finish or fail with a closed connection.
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        let slots: Vec<_> = self.slots.lock().unwrap().drain().map(|(_, v)| v).collect();
        let mut closing = Vec::new();
        for slot in slots {
            if let Some(old) = slot.lock().await.live.take() {
                old.permits.close();
                closing.push(old.conn);
            }
        }
        for conn in &closing {
            conn.close().await;
        }
    }
}

fn close_in_background(conn: Arc<dyn StdioConnection>) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move { conn.close().await });
        }
        // No runtime: dropping the last handle still kills the child
        // (`kill_on_drop`).
        Err(_) => drop(conn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct FakeConn {
        id: u32,
        alive: AtomicBool,
        lists: AtomicUsize,
        calls: AtomicUsize,
        closed: AtomicBool,
        /// The next call returns `Broken`.
        break_next: AtomicBool,
        /// Calls sleep this long (ms) before answering.
        delay_ms: AtomicU64,
        in_flight: AtomicUsize,
        peak: AtomicUsize,
    }

    impl StdioConnection for FakeConn {
        fn list_tools(&self) -> BoxFuture<'_, Result<Vec<ListedTool>, CallError>> {
            Box::pin(async move {
                self.lists.fetch_add(1, Ordering::SeqCst);
                if self.break_next.swap(false, Ordering::SeqCst) {
                    self.alive.store(false, Ordering::SeqCst);
                    return Err(CallError::Broken("Transport closed".into()));
                }
                Ok(vec![ListedTool {
                    name: format!("tool{}", self.id),
                    description: None,
                    read_only: true,
                    input_schema: json!({}),
                }])
            })
        }

        fn call_tool<'a>(
            &'a self,
            name: &'a str,
            _arguments: &'a Value,
        ) -> BoxFuture<'a, Result<String, CallError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                let delay = self.delay_ms.load(Ordering::SeqCst);
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                if self.break_next.swap(false, Ordering::SeqCst) {
                    self.alive.store(false, Ordering::SeqCst);
                    return Err(CallError::Broken("Transport closed".into()));
                }
                if name == "fail" {
                    return Err(CallError::Failed("调用失败：nope".into()));
                }
                Ok(format!("{name}@{}", self.id))
            })
        }

        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }

        fn pid(&self) -> Option<u32> {
            Some(self.id)
        }

        fn close(&self) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                self.alive.store(false, Ordering::SeqCst);
                self.closed.store(true, Ordering::SeqCst);
            })
        }
    }

    #[derive(Default)]
    struct FakeSpawner {
        made: StdMutex<Vec<Arc<FakeConn>>>,
        fail: AtomicBool,
        delay_ms: AtomicU64,
    }

    impl FakeSpawner {
        fn conn(&self, i: usize) -> Arc<FakeConn> {
            self.made.lock().unwrap()[i].clone()
        }
        fn count(&self) -> usize {
            self.made.lock().unwrap().len()
        }
    }

    impl StdioSpawner for FakeSpawner {
        fn spawn<'a>(
            &'a self,
            _cfg: &'a ConnectConfig,
        ) -> BoxFuture<'a, Result<Arc<dyn StdioConnection>, String>> {
            Box::pin(async move {
                if self.fail.load(Ordering::SeqCst) {
                    return Err("无法启动 MCP 进程：nope".into());
                }
                let mut made = self.made.lock().unwrap();
                let conn = Arc::new(FakeConn {
                    id: made.len() as u32 + 1,
                    alive: AtomicBool::new(true),
                    delay_ms: AtomicU64::new(self.delay_ms.load(Ordering::SeqCst)),
                    ..Default::default()
                });
                made.push(conn.clone());
                Ok(conn as Arc<dyn StdioConnection>)
            })
        }
    }

    fn cfg(cmd: &str) -> ConnectConfig {
        ConnectConfig {
            transport: TransportKind::Stdio,
            command: Some(cmd.into()),
            args: vec!["--x".into()],
            url: None,
            env: vec![("TOKEN".into(), "secret-1".into())],
            headers: vec![],
        }
    }

    fn pool_with(cfg: PoolConfig) -> (StdioPool, Arc<FakeSpawner>) {
        let spawner = Arc::new(FakeSpawner::default());
        (StdioPool::new(cfg, spawner.clone()), spawner)
    }

    fn pool() -> (StdioPool, Arc<FakeSpawner>) {
        pool_with(PoolConfig::default())
    }

    #[test]
    fn config_hash_changes_with_every_behaviour_field() {
        let base = cfg("srv");
        let h = config_hash(&base);
        assert_eq!(h, config_hash(&base.clone()));
        assert_eq!(h.len(), 64);
        let variants = [
            ConnectConfig {
                command: Some("srv2".into()),
                ..base.clone()
            },
            ConnectConfig {
                args: vec!["--y".into()],
                ..base.clone()
            },
            ConnectConfig {
                env: vec![("TOKEN".into(), "secret-2".into())],
                ..base.clone()
            },
            ConnectConfig {
                env: vec![],
                ..base.clone()
            },
            // Field boundaries are length-prefixed: these must not collide.
            ConnectConfig {
                args: vec!["--".into(), "x".into()],
                ..base.clone()
            },
        ];
        for v in variants {
            assert_ne!(h, config_hash(&v), "{v:?}");
        }
        assert!(!h.contains("secret-1"));
    }

    #[tokio::test]
    async fn reuses_one_process_and_caches_tools_across_tasks() {
        let (pool, spawner) = pool();
        let c = cfg("srv");
        let first = pool.list_tools("s1", &c).await.unwrap();
        let second = pool.list_tools("s1", &c).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap(),
            "ping@1"
        );
        assert_eq!(
            pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap(),
            "ping@1"
        );
        assert_eq!(spawner.count(), 1, "one process for four requests");
        assert_eq!(
            spawner.conn(0).lists.load(Ordering::SeqCst),
            1,
            "tools/list cached"
        );
        assert_eq!(pool.live_pid("s1").await, Some(1));
    }

    #[tokio::test]
    async fn servers_get_separate_processes() {
        let (pool, spawner) = pool();
        pool.list_tools("a", &cfg("srv")).await.unwrap();
        pool.list_tools("b", &cfg("srv")).await.unwrap();
        assert_eq!(spawner.count(), 2);
        assert_eq!(pool.live_count().await, 2);
    }

    #[tokio::test]
    async fn config_change_restarts_the_process_and_drops_the_tool_cache() {
        let (pool, spawner) = pool();
        let old = cfg("srv");
        assert_eq!(pool.list_tools("s1", &old).await.unwrap()[0].name, "tool1");
        let rotated = ConnectConfig {
            env: vec![("TOKEN".into(), "secret-2".into())],
            ..old.clone()
        };
        assert_eq!(
            pool.list_tools("s1", &rotated).await.unwrap()[0].name,
            "tool2"
        );
        assert_eq!(spawner.count(), 2);
        tokio::task::yield_now().await;
        assert!(
            spawner.conn(0).closed.load(Ordering::SeqCst),
            "old process stopped"
        );
        assert_eq!(pool.live_count().await, 1);
    }

    #[tokio::test]
    async fn a_config_change_lets_the_call_in_flight_finish() {
        let (pool, spawner) = pool();
        spawner.delay_ms.store(50, Ordering::SeqCst);
        let pool = Arc::new(pool);
        let old = cfg("srv");
        pool.list_tools("s1", &old).await.unwrap();
        let slow = {
            let pool = pool.clone();
            let old = old.clone();
            tokio::spawn(async move { pool.call_tool("s1", &old, "slow", &json!({})).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let new = ConnectConfig {
            args: vec!["--y".into()],
            ..old
        };
        pool.list_tools("s1", &new).await.unwrap();
        assert_eq!(slow.await.unwrap().unwrap(), "slow@1", "not cut off");
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            spawner.conn(0).closed.load(Ordering::SeqCst),
            "closed afterwards"
        );
    }

    #[tokio::test]
    async fn reconnects_after_the_child_exits() {
        let (pool, spawner) = pool();
        let c = cfg("srv");
        pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap();
        spawner.conn(0).alive.store(false, Ordering::SeqCst);
        assert_eq!(
            pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap(),
            "ping@2"
        );
        assert_eq!(spawner.count(), 2);
    }

    #[tokio::test]
    async fn broken_pipe_on_list_retries_once_on_a_new_process() {
        let (pool, spawner) = pool();
        let c = cfg("srv");
        pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap();
        spawner.conn(0).break_next.store(true, Ordering::SeqCst);
        let tools = pool.list_tools("s1", &c).await.unwrap();
        assert_eq!(tools[0].name, "tool2");
        assert_eq!(spawner.count(), 2);
    }

    #[tokio::test]
    async fn broken_pipe_on_call_is_not_retried_but_the_next_call_reconnects() {
        let (pool, spawner) = pool();
        let c = cfg("srv");
        pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap();
        spawner.conn(0).break_next.store(true, Ordering::SeqCst);
        let err = pool
            .call_tool("s1", &c, "write", &json!({}))
            .await
            .unwrap_err();
        assert_eq!(err, "Transport closed");
        assert_eq!(spawner.conn(0).calls.load(Ordering::SeqCst), 2, "no retry");
        assert_eq!(spawner.count(), 1);
        assert_eq!(
            pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap(),
            "ping@2"
        );
    }

    #[tokio::test]
    async fn server_errors_keep_the_connection() {
        let (pool, spawner) = pool();
        let c = cfg("srv");
        let err = pool
            .call_tool("s1", &c, "fail", &json!({}))
            .await
            .unwrap_err();
        assert_eq!(err, "调用失败：nope");
        pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap();
        assert_eq!(spawner.count(), 1);
    }

    #[tokio::test]
    async fn spawn_errors_are_returned_and_not_cached() {
        let (pool, spawner) = pool();
        let c = cfg("srv");
        spawner.fail.store(true, Ordering::SeqCst);
        assert!(pool
            .list_tools("s1", &c)
            .await
            .unwrap_err()
            .contains("无法启动"));
        spawner.fail.store(false, Ordering::SeqCst);
        assert!(pool.list_tools("s1", &c).await.is_ok());
    }

    #[tokio::test]
    async fn idle_connections_are_reaped_and_busy_ones_are_not() {
        let (pool, spawner) = pool_with(PoolConfig {
            idle_timeout: Duration::from_millis(50),
            ..PoolConfig::default()
        });
        let c = cfg("srv");
        pool.list_tools("s1", &c).await.unwrap();
        assert_eq!(pool.reap_idle(Instant::now()).await, 0, "not idle yet");
        let later = Instant::now() + Duration::from_millis(60);
        assert_eq!(pool.reap_idle(later).await, 1);
        assert!(spawner.conn(0).closed.load(Ordering::SeqCst));
        assert_eq!(pool.live_count().await, 0);
        // Next use starts a new process.
        pool.list_tools("s1", &c).await.unwrap();
        assert_eq!(spawner.count(), 2);
    }

    #[tokio::test]
    async fn an_expired_connection_is_replaced_on_use_even_before_the_reaper_runs() {
        let (pool, spawner) = pool_with(PoolConfig {
            idle_timeout: Duration::from_millis(20),
            ..PoolConfig::default()
        });
        let c = cfg("srv");
        pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            pool.call_tool("s1", &c, "ping", &json!({})).await.unwrap(),
            "ping@2"
        );
        assert_eq!(spawner.count(), 2);
    }

    #[tokio::test]
    async fn per_connection_concurrency_is_bounded() {
        let (pool, spawner) = pool_with(PoolConfig {
            max_in_flight: 2,
            ..PoolConfig::default()
        });
        spawner.delay_ms.store(30, Ordering::SeqCst);
        let pool = Arc::new(pool);
        let c = cfg("srv");
        pool.call_tool("s1", &c, "warm", &json!({})).await.unwrap();
        let mut joins = Vec::new();
        for _ in 0..6 {
            let pool = pool.clone();
            let c = c.clone();
            joins.push(tokio::spawn(async move {
                pool.call_tool("s1", &c, "ping", &json!({})).await
            }));
        }
        for j in joins {
            j.await.unwrap().unwrap();
        }
        assert_eq!(spawner.count(), 1);
        assert_eq!(spawner.conn(0).peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_timed_out_call_stops_the_process() {
        let (pool, spawner) = pool_with(PoolConfig {
            call_timeout: Duration::from_millis(20),
            ..PoolConfig::default()
        });
        spawner.delay_ms.store(200, Ordering::SeqCst);
        let c = cfg("srv");
        assert_eq!(
            pool.call_tool("s1", &c, "slow", &json!({}))
                .await
                .unwrap_err(),
            TIMED_OUT
        );
        tokio::task::yield_now().await;
        assert!(spawner.conn(0).closed.load(Ordering::SeqCst));
        assert_eq!(pool.live_count().await, 0);
    }

    #[tokio::test]
    async fn evict_and_shutdown_stop_processes() {
        let (pool, spawner) = pool();
        pool.list_tools("a", &cfg("srv")).await.unwrap();
        pool.list_tools("b", &cfg("srv")).await.unwrap();
        pool.evict("a").await;
        tokio::task::yield_now().await;
        assert!(spawner.conn(0).closed.load(Ordering::SeqCst));
        assert!(!spawner.conn(1).closed.load(Ordering::SeqCst));
        pool.shutdown().await;
        assert!(spawner.conn(1).closed.load(Ordering::SeqCst));
        assert_eq!(
            pool.list_tools("b", &cfg("srv")).await.unwrap_err(),
            POOL_CLOSED
        );
        assert_eq!(spawner.count(), 2, "no process after shutdown");
    }
}
