use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use robroz_mcp::{FakeConnector, ListedTool};
use robroz_model::{AssistantTurn, ChatMessage, ModelError, ToolCall, ToolDefinition};
use rusqlite::Connection;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::{overdue, started_at};
use crate::sandbox::FakeSandbox;

const TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[test]
fn only_running_tasks_past_the_timeout_are_overdue() {
    let conn = Connection::open_in_memory().unwrap();
    crate::init_schema(&conn).unwrap();
    let now = Utc::now();
    let at = |mins: i64| {
        (now - chrono::Duration::minutes(mins)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    for (id, state, started) in [
        ("late", "running", at(11)),
        ("fresh", "running", at(9)),
        ("blocked", "blocked", at(60)),
        ("done", "done", at(60)),
        ("broken", "running", "not a time".to_string()),
    ] {
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at)
             VALUES (?1, 'g', 'b', 'p', ?2, ?3, ?3)",
            rusqlite::params![id, state, at(70)],
        )
        .unwrap();
        super::start(&conn, id, &started).unwrap();
    }
    let mut late = overdue(&conn, now, TIMEOUT).unwrap();
    late.sort();
    assert_eq!(late, vec!["broken".to_string(), "late".to_string()]);
    super::clear(&conn, "late").unwrap();
    assert_eq!(started_at(&conn, "late").unwrap(), None);
}

enum Turn {
    Now(AssistantTurn),
    /// Say "entered", wait for the gate, then answer.
    Gated(AssistantTurn),
}

/// Splits from `split` (for `reply`) and answers `chat` from `turns`.
struct Script {
    split: Mutex<VecDeque<String>>,
    turns: Mutex<VecDeque<Turn>>,
    gate: Mutex<Receiver<()>>,
    entered: Mutex<Sender<()>>,
    chats: Mutex<usize>,
}

struct Gate {
    release: Sender<()>,
    entered: Receiver<()>,
}

fn script(split: Vec<&str>, turns: Vec<Turn>) -> (Arc<Script>, Gate) {
    let (release, gate) = channel();
    let (entered_tx, entered) = channel();
    (
        Arc::new(Script {
            split: Mutex::new(split.into_iter().map(String::from).collect()),
            turns: Mutex::new(turns.into()),
            gate: Mutex::new(gate),
            entered: Mutex::new(entered_tx),
            chats: Mutex::new(0),
        }),
        Gate { release, entered },
    )
}

impl Script {
    fn chats(&self) -> usize {
        *self.chats.lock().unwrap()
    }
}

impl robroz_model::Model for Script {
    fn reply(&self, _: &str, _: Option<&str>) -> Result<String, ModelError> {
        self.split
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ModelError::new("no split"))
    }

    fn chat(&self, _: &[ChatMessage], _: &[ToolDefinition]) -> Result<AssistantTurn, ModelError> {
        *self.chats.lock().unwrap() += 1;
        let next = self.turns.lock().unwrap().pop_front();
        match next {
            Some(Turn::Now(turn)) => Ok(turn),
            Some(Turn::Gated(turn)) => {
                let _ = self.entered.lock().unwrap().send(());
                let _ = self.gate.lock().unwrap().recv();
                Ok(turn)
            }
            None => Err(ModelError::new("no more turns")),
        }
    }
}

struct Harness {
    db: PathBuf,
    volumes: PathBuf,
    control: PathBuf,
    host: crate::Host,
    app: Router,
    mcp: FakeConnector,
    group_id: String,
}

fn temp_path(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn http(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Inline,
    Offloaded,
}

fn host_for(
    db: &Path,
    volumes: &Path,
    control: &Path,
    model: Arc<Script>,
    mcp: FakeConnector,
    mode: Mode,
) -> crate::Host {
    let host = crate::Host::open(db, volumes, "robroz-runner:local")
        .unwrap()
        .with_control_root(control)
        .with_model(Some(model))
        .with_router(true)
        .with_mcp(Arc::new(mcp));
    match mode {
        Mode::Inline => host.with_step_timeout(Some(TIMEOUT)),
        Mode::Offloaded => host.with_offloaded_steps(crate::watchdog::WatchdogConfig {
            step_timeout: TIMEOUT,
            ..Default::default()
        }),
    }
}

async fn setup(model: Arc<Script>, mode: Mode) -> Harness {
    let db = std::env::temp_dir().join(format!("robroz-timeout-{}.db", uuid::Uuid::new_v4()));
    let volumes = temp_path("robroz-timeout-vol");
    let control = temp_path("robroz-timeout-ctl");
    let mcp = FakeConnector::new();
    let tool = |name: &str, read_only: bool| ListedTool {
        name: name.into(),
        description: Some(name.into()),
        read_only,
        input_schema: json!({ "type": "object", "properties": {} }),
    };
    mcp.set_tools(
        "http://notes.local",
        vec![tool("read_note", true), tool("write_note", false)],
    );
    mcp.set_result("http://notes.local", "read_note", Ok("note".into()));
    mcp.set_result("http://notes.local", "write_note", Ok("written".into()));
    let host = host_for(&db, &volumes, &control, model, mcp.clone(), mode);
    let app = host.router();
    let (_, group) = http(
        &app,
        "POST",
        "/api/groups",
        Some(json!({ "name": "Timeouts" })),
    )
    .await;
    let group_id = group["id"].as_str().unwrap().to_string();
    let (status, _) = http(
        &app,
        "POST",
        &format!("/api/groups/{group_id}/members"),
        Some(json!({ "botId": "bot-ada", "name": "Ada" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, server) = http(
        &app,
        "POST",
        "/api/mcp/servers",
        Some(json!({ "name": "notes", "transport": "http", "url": "http://notes.local" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{server}");
    let server_id = server["id"].as_str().unwrap();
    let (status, _) = http(
        &app,
        "PUT",
        &format!("/api/groups/{group_id}/mcp/{server_id}"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    Harness {
        db,
        volumes,
        control,
        host,
        app,
        mcp,
        group_id,
    }
}

impl Harness {
    async fn task(&self, prompt: &str) -> String {
        let (status, task) = http(
            &self.app,
            "POST",
            &format!("/api/groups/{}/tasks", self.group_id),
            Some(json!({ "assigneeBotId": "bot-ada", "prompt": prompt })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        task["id"].as_str().unwrap().to_string()
    }

    fn conn(&self) -> Connection {
        Connection::open(&self.db).unwrap()
    }

    fn row(&self, task_id: &str) -> (String, Option<String>) {
        self.conn()
            .query_row(
                "SELECT state, failure FROM tasks WHERE id = ?1",
                [task_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn deliveries(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM deliveries")
    }

    fn has_plan(&self, task_id: &str) -> bool {
        crate::router::load(&self.conn(), task_id)
            .unwrap()
            .is_some()
    }

    fn clock(&self, task_id: &str) -> Option<String> {
        started_at(&self.conn(), task_id).unwrap()
    }

    /// Move the current step's start `mins` minutes into the past.
    fn age_step(&self, task_id: &str, mins: i64) {
        let at = (Utc::now() - chrono::Duration::minutes(mins))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let n = self
            .conn()
            .execute(
                "UPDATE step_clocks SET started_at = ?2 WHERE task_id = ?1",
                rusqlite::params![task_id, at],
            )
            .unwrap();
        assert_eq!(n, 1, "the step has a clock");
    }

    async fn approve_in_dm(&self) {
        let (_, dm) = http(&self.app, "POST", "/api/bros/bot-ada/dm", Some(json!({}))).await;
        let dm = dm["dmGroupId"].as_str().unwrap().to_string();
        let (_, events) = http(&self.app, "GET", &format!("/api/groups/{dm}/events"), None).await;
        let id = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "approval_reminder")
            .expect("approval reminder (v0.2: the card is in the group)")["approvalId"]
            .as_str()
            .unwrap()
            .to_string();
        let (status, body) = http(
            &self.app,
            "POST",
            &format!("/api/approvals/{id}/approve"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

fn text(t: &str) -> AssistantTurn {
    AssistantTurn::Text(t.into())
}

fn call(tool: &str) -> AssistantTurn {
    AssistantTurn::ToolCalls(vec![ToolCall {
        id: "c1".into(),
        name: format!("notes__{tool}"),
        arguments: "{}".into(),
    }])
}

fn assert_cancelled(h: &Harness, task_id: &str, sandbox: &FakeSandbox) {
    assert_eq!(
        h.row(task_id),
        (
            "done".to_string(),
            Some(robroz_contract::legacy::NO_RESULT.to_string())
        )
    );
    assert!(sandbox.containers.is_empty(), "container stopped");
    assert_eq!(h.deliveries(), 0, "no partial delivery");
    assert!(!h.has_plan(task_id), "plan dropped");
    assert_eq!(h.clock(task_id), None, "clock dropped");
    assert_eq!(
        h.count("SELECT COUNT(*) FROM tasks WHERE state = 'queued'"),
        0
    );
}

#[tokio::test]
async fn a_worker_past_the_timeout_is_stopped_and_ends_with_no_result() {
    let (model, _gate) = script(vec!["one step only"], vec![Turn::Now(text("done"))]);
    let h = setup(model, Mode::Inline).await;
    let task_id = h.task("write a line").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(sandbox.containers.len(), 1, "container running");
    // Just under the timeout: left alone.
    h.age_step(&task_id, 9);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.row(&task_id).0, "running");
    assert_eq!(sandbox.containers.len(), 1);

    h.age_step(&task_id, 11);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_cancelled(&h, &task_id, &sandbox);
    for _ in 0..3 {
        h.host.pump_once(&mut sandbox).unwrap();
    }
    assert_cancelled(&h, &task_id, &sandbox);
}

#[tokio::test]
async fn a_router_step_past_the_timeout_cancels_the_task_and_its_late_reply() {
    let (model, gate) = script(
        vec![r#"["look it up","write it"]"#],
        vec![Turn::Now(text("r1")), Turn::Gated(call("read_note"))],
    );
    let h = setup(model.clone(), Mode::Offloaded).await;
    let task_id = h.task("two steps").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    gate.entered
        .recv_timeout(Duration::from_secs(5))
        .expect("step 2 started");
    let progress: Option<String> = h
        .conn()
        .query_row(
            "SELECT progress FROM tasks WHERE id = ?1",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(progress.as_deref(), Some("2/2 步"));
    assert!(h.has_plan(&task_id));

    h.age_step(&task_id, 11);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_cancelled(&h, &task_id, &sandbox);

    // The step comes back late, asking for a tool: it is not run, and nothing
    // of the step is kept.
    gate.release.send(()).unwrap();
    for _ in 0..50 {
        h.host.pump_once(&mut sandbox).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(h.mcp.calls().is_empty(), "no tool call after the cancel");
    assert_eq!(model.chats(), 2, "no model call after the cancel");
    assert_cancelled(&h, &task_id, &sandbox);
}

#[tokio::test]
async fn the_hint_shows_at_half_the_timeout_and_the_cancel_comes_at_the_full_timeout() {
    let (model, gate) = script(vec!["one step only"], vec![Turn::Gated(text("late"))]);
    let h = setup(model, Mode::Offloaded).await;
    let task_id = h.task("think slowly").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    gate.entered
        .recv_timeout(Duration::from_secs(5))
        .expect("step started");
    let progress = |h: &Harness| -> Option<String> {
        h.conn()
            .query_row(
                "SELECT progress FROM tasks WHERE id = ?1",
                [&task_id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let hint = robroz_contract::host_text::step_timeout_hint(6);

    // Under half of the 10-minute budget: no hint yet.
    let now = std::time::Instant::now();
    h.host
        .watchdog_once(now + Duration::from_secs(4 * 60))
        .unwrap();
    assert_ne!(progress(&h), Some(hint.clone()));

    // Past half, under the full budget: the hint is up and the step runs on.
    h.host
        .watchdog_once(now + Duration::from_secs(6 * 60))
        .unwrap();
    assert_eq!(progress(&h), Some(hint.clone()));
    h.age_step(&task_id, 6);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.row(&task_id), ("running".to_string(), None));
    assert_eq!(progress(&h), Some(hint));
    assert!(h.clock(&task_id).is_some());

    // At the full budget: cancelled.
    h.age_step(&task_id, 10);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_cancelled(&h, &task_id, &sandbox);
    gate.release.send(()).unwrap();
    for _ in 0..20 {
        h.host.pump_once(&mut sandbox).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_cancelled(&h, &task_id, &sandbox);
}

#[tokio::test]
async fn waiting_on_an_approval_card_is_not_step_time() {
    let (model, _gate) = script(
        vec!["no split"],
        vec![Turn::Now(call("write_note")), Turn::Now(text("noted"))],
    );
    let h = setup(model, Mode::Inline).await;
    let task_id = h.task("write a note").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.row(&task_id).0, "blocked");
    // An hour on the card: still blocked, nothing cancelled.
    h.age_step(&task_id, 60);
    for _ in 0..3 {
        h.host.pump_once(&mut sandbox).unwrap();
    }
    assert_eq!(h.row(&task_id), ("blocked".to_string(), None));

    h.approve_in_dm().await;
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.mcp.calls().len(), 1, "the approved call ran");
    assert_eq!(h.row(&task_id).0, "running", "fresh clock after the card");
    let started = h.clock(&task_id).expect("clock");
    let age =
        Utc::now().signed_duration_since(chrono::DateTime::parse_from_rfc3339(&started).unwrap());
    assert!(age < chrono::Duration::minutes(1), "{started}");
    sandbox.finish(0);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.row(&task_id), ("done".to_string(), None));
    assert_eq!(h.deliveries(), 1);
    assert_eq!(h.clock(&task_id), None);
}

#[tokio::test]
async fn a_timeout_is_still_enforced_after_a_host_restart() {
    let (model, _gate) = script(vec!["one step only"], vec![Turn::Now(text("done"))]);
    let h = setup(model.clone(), Mode::Inline).await;
    let task_id = h.task("write a line").await;
    // Docker keeps the container across a host restart.
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(sandbox.containers.len(), 1);

    h.age_step(&task_id, 11);
    let restarted = host_for(
        &h.db,
        &h.volumes,
        &h.control,
        model,
        h.mcp.clone(),
        Mode::Offloaded,
    );
    restarted.pump_once(&mut sandbox).unwrap();
    let h = Harness {
        host: restarted,
        ..h
    };
    assert_cancelled(&h, &task_id, &sandbox);
}

#[tokio::test]
async fn without_a_timeout_nothing_is_cancelled() {
    let (model, _gate) = script(vec!["one step only"], vec![Turn::Now(text("done"))]);
    let h = setup(model.clone(), Mode::Inline).await;
    let host = h.host.clone().with_step_timeout(None);
    let task_id = h.task("write a line").await;
    let mut sandbox = FakeSandbox::new();
    host.pump_once(&mut sandbox).unwrap();
    h.age_step(&task_id, 600);
    host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.row(&task_id).0, "running");
    sandbox.finish(0);
    host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.row(&task_id), ("done".to_string(), None));
}
