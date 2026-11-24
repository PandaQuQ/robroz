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

fn fail(msg: &str) -> ! {
    eprintln!("robroz-host: {msg}");
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
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
        .with_token(token.clone());
    let app = host.router();
    let pumping = host.clone();

    // Docker CLI calls block, so the loop runs on its own OS thread, not on a
    // tokio worker. pump only takes the DB lock for SQL, so HTTP keeps serving.
    std::thread::Builder::new()
        .name("robroz-pump".into())
        .spawn(move || {
            let mut sandbox = DockerSandbox::new();
            loop {
                // Schedules only create queued tasks; pump is what starts containers.
                if let Err(err) = pumping.tick_once(chrono::Utc::now()) {
                    eprintln!("tick: {err}");
                }
                if let Err(err) = pumping.pump_once(&mut sandbox) {
                    eprintln!("pump: {err}");
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("无法启动 pump 线程");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|err| panic!("无法监听 {addr}: {err}"));
    let guard = if token.is_some() {
        "需要令牌"
    } else {
        "仅本机，无令牌"
    };
    eprintln!("robroz-host listening on {addr}（{guard}）");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|err| panic!("服务器退出: {err}"));
}
