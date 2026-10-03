use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use openbots_host::{DockerSandbox, Host};

#[tokio::main]
async fn main() {
    let addr: SocketAddr = std::env::var("OPENBOTS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8790".to_string())
        .parse()
        .expect("OPENBOTS_ADDR 不是合法的监听地址");
    let db_path = std::env::var("OPENBOTS_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./openbots.db"));
    let volumes = std::env::var("OPENBOTS_VOLUMES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data/groups"));
    let image = std::env::var("OPENBOTS_IMAGE").unwrap_or_else(|_| "openbots-runner:local".to_string());

    let host = Host::open(&db_path, volumes, image).unwrap_or_else(|err| {
        panic!("无法打开数据库 {}: {err}", db_path.display());
    });
    let app = host.router();
    let pumping = host.clone();

    tokio::spawn(async move {
        let mut sandbox = DockerSandbox::new();
        loop {
            if let Err(err) = pumping.pump_once(&mut sandbox) {
                eprintln!("pump: {err}");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|err| panic!("无法监听 {addr}: {err}"));
    eprintln!("openbots-host listening on {addr}");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|err| panic!("服务器退出: {err}"));
}
