use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use robroz_host::{auth, DockerSandbox, Host, DEFAULT_CONTROL_ROOT};

/// Containers can write anywhere in the group volumes, so takeover markers must
/// live outside them.
fn ensure_control_outside_volumes(control: &std::path::Path, volumes: &std::path::Path) {
    for dir in [control, volumes] {
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|err| panic!("无法创建 {}: {err}", dir.display()));
    }
    let control = std::fs::canonicalize(control).expect("ROBROZ_CONTROL 无法解析");
    let volumes = std::fs::canonicalize(volumes).expect("ROBROZ_VOLUMES 无法解析");
    if control.starts_with(&volumes) || volumes.starts_with(&control) {
        panic!("ROBROZ_CONTROL 必须在 ROBROZ_VOLUMES 外面");
    }
}

/// `robroz-host <version> (<short git sha>)`, for `--version` / `-V`.
fn version_line() -> String {
    format!(
        "robroz-host {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("ROBROZ_GIT_SHA_SHORT")
    )
}

/// `--version` / `-V` print the version and exit 0 before anything else, so
/// they need no token, database or Docker. `--health` probes the already
/// running process (used as the image `HEALTHCHECK`) and likewise needs no
/// setup: it talks to 127.0.0.1 on the port in `ROBROZ_ADDR`.
fn handle_early_flags() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("{}", version_line());
        std::process::exit(0);
    }
    if args.iter().any(|arg| arg == "--health") {
        std::process::exit(if health_ok() { 0 } else { 1 });
    }
}

/// GET /api/bros on the local listener. 401 (token required) or 200 means
/// the process is up; anything else, including a connection error, is down.
fn health_ok() -> bool {
    let raw = std::env::var("ROBROZ_ADDR").unwrap_or_else(|_| "127.0.0.1:8790".into());
    let port = raw.rsplit_once(':').map(|(_, p)| p).unwrap_or("8790");
    let addr = format!("127.0.0.1:{port}");
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(
        &addr
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:8790".parse().unwrap()),
        Duration::from_secs(2),
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if std::io::Write::write_all(
        &mut stream,
        b"GET /api/bros HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 96];
    let Ok(n) = std::io::Read::read(&mut stream, &mut buf) else {
        return false;
    };
    let head = std::str::from_utf8(&buf[..n]).unwrap_or("");
    head.starts_with("HTTP/1.") && (head.contains(" 401 ") || head.contains(" 200 "))
}

/// New files the host creates (the database, its journal, group folders)
/// are mode 0600 / 0700, so other users on the machine cannot read them.
fn tighten_umask() {
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("robroz-host: {msg}");
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    handle_early_flags();
    tighten_umask();
    let addr: SocketAddr = std::env::var("ROBROZ_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8790".to_string())
        .parse()
        .expect("ROBROZ_ADDR 不是合法的监听地址");
    // Errors here never contain the token itself.
    let token = auth::token_from_env().unwrap_or_else(|err| fail(&err));
    auth::check_listener(addr, token.as_ref()).unwrap_or_else(|err| fail(&err));
    let db_path = std::env::var("ROBROZ_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./robroz.db"));
    let volumes = std::env::var("ROBROZ_VOLUMES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data/groups"));
    let control = std::env::var("ROBROZ_CONTROL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CONTROL_ROOT));
    ensure_control_outside_volumes(&control, &volumes);
    // Only a conflicting or invalid key stops startup. A partial env is fine:
    // the rest can be filled in through the settings page, and takes effect
    // on the next task without restarting. Do not freeze the env model onto
    // the host; each pump reads the latest settings and env.
    robroz_host::validate_model_env().unwrap_or_else(|err| fail(&err));
    let echo = std::env::var("ROBROZ_MODEL")
        .ok()
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("echo"));
    match robroz_model::config_from_env().ok().flatten() {
        Some(cfg) => eprintln!(
            "robroz-host: 环境变量里有模型 {} @ {}，设置页保存的值优先",
            cfg.model, cfg.base_url
        ),
        None if echo => eprintln!("robroz-host: 测试回显（ROBROZ_MODEL=echo）"),
        None => eprintln!("robroz-host: 还没设置模型，任务会提示去设置"),
    }
    let image = std::env::var("ROBROZ_IMAGE").unwrap_or_else(|_| "robroz-runner:local".to_string());

    let host = Host::open(&db_path, volumes, image)
        .unwrap_or_else(|err| {
            panic!("无法打开数据库 {}: {err}", db_path.display());
        })
        .with_control_root(control)
        .with_token(token.clone())
        .with_single_responder(robroz_host::responder::enabled_from_env())
        .with_offloaded_steps(robroz_host::watchdog::WatchdogConfig::from_env())
        .with_stream_config(robroz_host::stream::StreamConfig::from_env());
    let app = host.router();
    let pumping = host.clone();

    // Docker CLI calls block, so the loop runs on its own OS thread, not on a
    // tokio worker. pump only takes the DB lock for SQL, so HTTP keeps serving.
    // Model and tool calls run on per-task step threads, so one slow call does
    // not hold up other tasks. The Watchdog runs in the same loop, and once an
    // hour the live stream's change log is compacted and pruned.
    let retention = robroz_host::event_log::retention_from_env();
    std::thread::Builder::new()
        .name("robroz-pump".into())
        .spawn(move || {
            let mut sandbox = DockerSandbox::new();
            let mut next_log_pass = std::time::Instant::now();
            loop {
                if std::time::Instant::now() >= next_log_pass {
                    if let Err(err) = pumping.event_log_once(chrono::Utc::now(), retention) {
                        eprintln!("event log: {err}");
                    }
                    next_log_pass =
                        std::time::Instant::now() + robroz_host::event_log::MAINTAIN_EVERY;
                }
                // Schedules only create queued tasks; pump is what starts containers.
                if let Err(err) = pumping.tick_once(chrono::Utc::now()) {
                    eprintln!("tick: {err}");
                }
                if let Err(err) = pumping.pump_once(&mut sandbox) {
                    eprintln!("pump: {err}");
                }
                if let Err(err) = pumping.watchdog_once(std::time::Instant::now()) {
                    eprintln!("watchdog: {err}");
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("无法启动 pump 线程");

    // Auto-review and approval expiry. Review calls the model, so it runs on
    // its own thread and never holds up the pump.
    let reviewing = host.clone();
    std::thread::Builder::new()
        .name("robroz-review".into())
        .spawn(move || loop {
            if let Err(err) = reviewing.review_once(chrono::Utc::now()) {
                eprintln!("review: {err}");
            }
            std::thread::sleep(Duration::from_secs(1));
        })
        .expect("无法启动审核线程");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|err| panic!("无法监听 {addr}: {err}"));
    let guard = if token.is_some() {
        "需要令牌"
    } else {
        "仅本机，无令牌"
    };
    eprintln!("robroz-host listening on {addr}（{guard}）");
    tokio::select! {
        served = async { axum::serve(listener, app).await } => {
            served.unwrap_or_else(|err| panic!("服务器退出: {err}"));
        }
        () = shutdown_signal() => {
            // Pooled stdio MCP servers are child processes: stop them before
            // exiting. Long-lived SSE streams are not drained.
            eprintln!("robroz-host: 收到退出信号，正在停止 MCP 进程");
            host.shutdown_mcp();
            // Exit now, as before this handler existed: do not wait for
            // in-flight model calls or open streams.
            std::process::exit(0);
        }
    }
}

/// Ctrl-C, or SIGTERM (what `docker stop` sends) on Unix.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
