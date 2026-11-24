use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use robroz_contract::host_text::{parse_router_step_progress, router_step_progress};
use robroz_mcp::{FakeConnector, ListedTool};
use robroz_model::{AssistantTurn, ChatMessage, ModelError, ToolCall, ToolDefinition};
use rusqlite::Connection;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::{parse_steps, step_prompt, Plan, MAX_STEPS};
use crate::sandbox::FakeSandbox;

#[test]
fn steps_are_read_from_json_and_capped() {
    assert_eq!(
        parse_steps(r#"{"steps":["a","b"]}"#),
        Some(vec!["a".to_string(), "b".to_string()])
    );
    assert_eq!(
        parse_steps("好的：\n```json\n[\" a \", \"b\", \"c\", \"d\", \"e\"]\n```"),
        Some(vec!["a".into(), "b".into(), "c".into()])
    );
    assert_eq!(parse_steps(r#"{"steps":["only"]}"#), None);
    assert_eq!(parse_steps(r#"{"steps":["a", ""]}"#), None);
    assert_eq!(parse_steps(r#"{"steps":[1, 2]}"#), None);
    assert_eq!(parse_steps("就一步"), None);
    assert_eq!(parse_steps(""), None);
    assert_eq!(MAX_STEPS, 3);
}

#[test]
fn a_step_sees_the_task_and_earlier_results() {
    let plan = Plan {
        steps: vec!["查资料".into(), "写草稿".into(), "润色".into()],
        results: vec!["资料 A".into(), "草稿 B".into()],
    };
    let first = step_prompt("写周报", &plan, 0);
    assert!(first.starts_with("写周报"), "{first}");
    assert!(first.contains("第 1 步：查资料"), "{first}");
    assert!(!first.contains("资料 A"), "{first}");
    let third = step_prompt("写周报", &plan, 2);
    assert!(third.contains("第 3 步：润色"), "{third}");
    assert!(
        third.contains("资料 A") && third.contains("草稿 B"),
        "{third}"
    );
    assert_eq!(plan.progress(2), "3/3 步");
    assert_eq!(plan.merged(), "资料 A\n\n草稿 B");
}

/// Splits from a queue of `reply` answers and works through `chat` turns.
/// Records each chat's user prompt and the task's `progress` at that moment.
struct RouterModel {
    db: PathBuf,
    split: Mutex<VecDeque<Result<String, String>>>,
    turns: Mutex<VecDeque<AssistantTurn>>,
    replies: Mutex<usize>,
    prompts: Mutex<Vec<String>>,
    progress: Mutex<Vec<Option<String>>>,
}

impl RouterModel {
    fn new(db: &Path, split: Vec<Result<&str, &str>>, turns: Vec<AssistantTurn>) -> Arc<Self> {
        Arc::new(Self {
            db: db.to_path_buf(),
            split: Mutex::new(
                split
                    .into_iter()
                    .map(|r| r.map(String::from).map_err(String::from))
                    .collect(),
            ),
            turns: Mutex::new(turns.into()),
            replies: Mutex::new(0),
            prompts: Mutex::new(Vec::new()),
            progress: Mutex::new(Vec::new()),
        })
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }

    fn progress(&self) -> Vec<Option<String>> {
        self.progress.lock().unwrap().clone()
    }

    fn replies(&self) -> usize {
        *self.replies.lock().unwrap()
    }

    fn turns_left(&self) -> usize {
        self.turns.lock().unwrap().len()
    }
}

impl robroz_model::Model for RouterModel {
    fn reply(&self, _: &str, _: Option<&str>) -> Result<String, ModelError> {
        *self.replies.lock().unwrap() += 1;
        match self.split.lock().unwrap().pop_front() {
            Some(Ok(text)) => Ok(text),
            Some(Err(err)) => Err(ModelError(err)),
            None => Err(ModelError("no split answer".into())),
        }
    }

    fn chat(
        &self,
        messages: &[ChatMessage],
        _: &[ToolDefinition],
    ) -> Result<AssistantTurn, ModelError> {
        let user = messages
            .iter()
            .find_map(|m| match m {
                ChatMessage::User(text) => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        self.prompts.lock().unwrap().push(user);
        let conn = Connection::open(&self.db).unwrap();
        let progress: Option<String> = conn
            .query_row(
                "SELECT progress FROM tasks ORDER BY created_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        self.progress.lock().unwrap().push(progress);
        self.turns
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ModelError("no more turns".into()))
    }
}

struct Harness {
    db: PathBuf,
    volumes: PathBuf,
    control: PathBuf,
    host: crate::Host,
    app: Router,
    fake_mcp: FakeConnector,
    group_id: String,
    model: Arc<RouterModel>,
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
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn host_for(
    h_db: &Path,
    volumes: &Path,
    control: &Path,
    model: Arc<RouterModel>,
    mcp: FakeConnector,
) -> crate::Host {
    crate::Host::open(h_db, volumes, "robroz-runner:local")
        .unwrap()
        .with_control_root(control)
        .with_model(Some(model))
        .with_router(true)
        .with_mcp(Arc::new(mcp))
}

async fn setup(split: Vec<Result<&str, &str>>, turns: Vec<AssistantTurn>, mcp: bool) -> Harness {
    let db = std::env::temp_dir().join(format!("robroz-router-{}.db", uuid::Uuid::new_v4()));
    let volumes = temp_path("robroz-router-vol");
    let control = temp_path("robroz-router-ctl");
    let fake_mcp = FakeConnector::new();
    fake_mcp.set_tools(
        "http://notes.local",
        vec![ListedTool {
            name: "write_note".into(),
            description: Some("write_note".into()),
            read_only: false,
            input_schema: json!({ "type": "object", "properties": {} }),
        }],
    );
    fake_mcp.set_result("http://notes.local", "write_note", Ok("written".into()));
    let model = RouterModel::new(&db, split, turns);
    let host = host_for(&db, &volumes, &control, model.clone(), fake_mcp.clone());
    let app = host.router();
    let (status, group) = http(
        &app,
        "POST",
        "/api/groups",
        Some(json!({ "name": "Router" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().unwrap().to_string();
    let (status, body) = http(
        &app,
        "POST",
        &format!("/api/groups/{group_id}/members"),
        Some(json!({ "botId": "bot-ada", "name": "Ada" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    if mcp {
        let (status, server) = http(
            &app,
            "POST",
            "/api/mcp/servers",
            Some(json!({ "name": "notes", "transport": "http", "url": "http://notes.local" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{server}");
        let server_id = server["id"].as_str().unwrap();
        let (status, body) = http(
            &app,
            "PUT",
            &format!("/api/groups/{group_id}/mcp/{server_id}"),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    Harness {
        db,
        volumes,
        control,
        host,
        app,
        fake_mcp,
        group_id,
        model,
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

    async fn events(&self) -> Vec<Value> {
        let (status, body) = http(
            &self.app,
            "GET",
            &format!("/api/groups/{}/events", self.group_id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["events"].as_array().unwrap().clone()
    }

    async fn status(&self) -> Value {
        let events = self.events().await;
        let statuses: Vec<&Value> = events.iter().filter(|e| e["type"] == "status").collect();
        assert_eq!(statuses.len(), 1, "one status card: {events:?}");
        statuses[0].clone()
    }

    async fn deliveries(&self) -> Vec<Value> {
        self.events()
            .await
            .into_iter()
            .filter(|e| e["type"] == "delivery")
            .collect()
    }

    fn row(&self, task_id: &str) -> (String, Option<String>, Option<String>) {
        let conn = Connection::open(&self.db).unwrap();
        conn.query_row(
            "SELECT state, progress, failure FROM tasks WHERE id = ?1",
            [task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn has_plan(&self, task_id: &str) -> bool {
        let conn = Connection::open(&self.db).unwrap();
        super::load(&conn, task_id).unwrap().is_some()
    }

    fn result(&self, task_id: &str) -> String {
        std::fs::read_to_string(
            self.volumes
                .join(&self.group_id)
                .join(format!("tasks/{task_id}/result.txt")),
        )
        .unwrap()
    }
}

fn text(t: &str) -> AssistantTurn {
    AssistantTurn::Text(t.into())
}

fn write_note() -> AssistantTurn {
    AssistantTurn::ToolCalls(vec![ToolCall {
        id: "c1".into(),
        name: "notes__write_note".into(),
        arguments: r#"{"title":"hi"}"#.into(),
    }])
}

fn steps(progress: &[Option<String>]) -> Vec<Option<(usize, usize)>> {
    progress
        .iter()
        .map(|p| p.as_deref().and_then(parse_router_step_progress))
        .collect()
}

#[tokio::test]
async fn a_task_split_into_three_runs_three_steps_and_delivers_once() {
    let h = setup(
        vec![Ok(r#"{"steps":["查资料","写草稿","润色"]}"#)],
        vec![text("资料 A"), text("草稿 B"), text("定稿 C")],
        false,
    )
    .await;
    let task_id = h.task("写周报").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();

    assert_eq!(h.model.replies(), 1, "one split call");
    assert_eq!(
        steps(&h.model.progress()),
        vec![Some((1, 3)), Some((2, 3)), Some((3, 3))]
    );
    let prompts = h.model.prompts();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[0].contains("第 1 步：查资料"), "{}", prompts[0]);
    assert!(
        prompts[2].contains("资料 A") && prompts[2].contains("草稿 B"),
        "{}",
        prompts[2]
    );
    // Container up: still step progress, and the runner's own lines don't replace it.
    let status = h.status().await;
    assert_eq!(status["state"], "running", "{status}");
    assert_eq!(status["progress"], router_step_progress(3, 3), "{status}");
    let group_dir = h.volumes.join(&h.group_id);
    let progress_file = robroz_runner::progress_path(&group_dir, &task_id);
    std::fs::create_dir_all(progress_file.parent().unwrap()).unwrap();
    std::fs::write(&progress_file, "写到一半").unwrap();
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.status().await["progress"], router_step_progress(3, 3));

    sandbox.finish(0);
    h.host.pump_once(&mut sandbox).unwrap();
    let status = h.status().await;
    assert_eq!(status["state"], "done", "{status}");
    let deliveries = h.deliveries().await;
    assert_eq!(deliveries.len(), 1, "{deliveries:?}");
    assert!(deliveries[0]["failure"].is_null(), "{:?}", deliveries[0]);
    assert_eq!(h.result(&task_id), "资料 A\n\n草稿 B\n\n定稿 C");
    assert!(!h.has_plan(&task_id), "plan is dropped once the task ends");
}

#[tokio::test]
async fn more_than_three_steps_are_cut_to_three() {
    let h = setup(
        vec![Ok(r#"["a","b","c","d","e"]"#)],
        vec![text("1"), text("2"), text("3"), text("4")],
        false,
    )
    .await;
    let task_id = h.task("五步的活").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.model.prompts().len(), 3);
    assert_eq!(h.model.turns_left(), 1);
    assert_eq!(
        steps(&h.model.progress()).last().cloned().flatten(),
        Some((3, 3))
    );
    sandbox.finish(0);
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.result(&task_id), "1\n\n2\n\n3");
}

#[tokio::test]
async fn an_unusable_split_falls_back_to_one_worker_without_step_progress() {
    for split in [
        Ok("就一步，直接做"),
        Ok(r#"{"steps":["只有一步"]}"#),
        Err("模型服务返回 500"),
    ] {
        let h = setup(vec![split], vec![text("做好了")], false).await;
        let task_id = h.task("写一句欢迎语").await;
        let mut sandbox = FakeSandbox::new();
        h.host.pump_once(&mut sandbox).unwrap();
        assert_eq!(h.model.replies(), 1, "{split:?}");
        assert_eq!(
            h.model.prompts(),
            vec!["写一句欢迎语".to_string()],
            "{split:?}"
        );
        assert_eq!(
            h.model.progress(),
            vec![Some(crate::progress::THINKING.to_string())],
            "{split:?}"
        );
        assert!(!h.has_plan(&task_id), "{split:?}");
        let status = h.status().await;
        assert_eq!(
            status["progress"],
            crate::progress::IN_SANDBOX,
            "{split:?}: {status}"
        );
        sandbox.finish(0);
        h.host.pump_once(&mut sandbox).unwrap();
        assert_eq!(h.result(&task_id), "做好了", "{split:?}");
        assert_eq!(h.deliveries().await.len(), 1, "{split:?}");
    }
}

#[tokio::test]
async fn without_with_router_a_fake_model_is_never_asked_to_split() {
    let h = setup(vec![Ok(r#"["a","b"]"#)], vec![text("做好了")], false).await;
    let host = crate::Host::open(&h.db, &h.volumes, "robroz-runner:local")
        .unwrap()
        .with_control_root(&h.control)
        .with_model(Some(h.model.clone()));
    let task_id = h.task("写一句欢迎语").await;
    let mut sandbox = FakeSandbox::new();
    host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.model.replies(), 0);
    assert!(!h.has_plan(&task_id));
}

#[tokio::test]
async fn a_failed_step_ends_the_task_with_no_result_and_no_requeue() {
    // Step 2 has no turn left: the model call fails.
    let h = setup(vec![Ok(r#"["a","b","c"]"#)], vec![text("1")], false).await;
    let task_id = h.task("三步的活").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.model.prompts().len(), 2);
    assert!(
        sandbox.containers.is_empty(),
        "no container after a failed step"
    );
    let (state, progress, failure) = h.row(&task_id);
    assert_eq!(state, "done");
    assert_eq!(progress, None);
    assert_eq!(failure.as_deref(), Some(robroz_card::NO_RESULT));
    assert!(h.deliveries().await.is_empty(), "no partial delivery");
    assert!(!h.has_plan(&task_id));

    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.model.prompts().len(), 2, "not run again");
    assert_eq!(h.row(&task_id).0, "done");
    assert!(sandbox.containers.is_empty());
}

#[tokio::test]
async fn an_approval_mid_step_keeps_its_card_progress_and_resumes_the_remaining_steps() {
    let h = setup(
        vec![Ok(r#"["写笔记","总结"]"#)],
        vec![write_note(), text("笔记写好了"), text("总结 B")],
        true,
    )
    .await;
    let task_id = h.task("记一下再总结").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    assert!(h.fake_mcp.calls().is_empty(), "not before approval");
    assert_eq!(steps(&h.model.progress()), vec![Some((1, 2))]);
    // Blocked: the usual approval progress, not step progress.
    let status = h.status().await;
    assert_eq!(status["state"], "blocked", "{status}");
    assert_eq!(status["waitingFor"], "approval", "{status}");
    assert_eq!(
        status["progress"],
        robroz_contract::APPROVAL_SENT_TO_OWNER_DM_NOTICE,
        "{status}"
    );
    let (_, progress, _) = h.row(&task_id);
    assert!(
        parse_router_step_progress(progress.as_deref().unwrap_or("")).is_none(),
        "{progress:?}"
    );
    assert!(h.has_plan(&task_id));
    // A pass while blocked leaves it alone.
    h.host.pump_once(&mut sandbox).unwrap();
    assert_eq!(
        h.status().await["progress"],
        robroz_contract::APPROVAL_SENT_TO_OWNER_DM_NOTICE
    );

    let (_, dm) = http(&h.app, "POST", "/api/bros/bot-ada/dm", Some(json!({}))).await;
    let dm = dm["dmGroupId"].as_str().unwrap().to_string();
    let (_, dm_events) = http(&h.app, "GET", &format!("/api/groups/{dm}/events"), None).await;
    let card = dm_events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "approval")
        .unwrap()
        .clone();
    let id = card["id"].as_str().unwrap();
    let (status, body) = http(
        &h.app,
        "POST",
        &format!("/api/approvals/{id}/approve"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // After a restart: the plan and step index come back from SQLite.
    let host = host_for(
        &h.db,
        &h.volumes,
        &h.control,
        h.model.clone(),
        h.fake_mcp.clone(),
    );
    host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.fake_mcp.calls().len(), 1, "the approved call ran once");
    assert_eq!(h.model.replies(), 1, "not split again");
    assert_eq!(
        steps(&h.model.progress()),
        vec![Some((1, 2)), Some((1, 2)), Some((2, 2))]
    );
    sandbox.finish(0);
    host.pump_once(&mut sandbox).unwrap();
    assert_eq!(h.status().await["state"], "done");
    assert_eq!(h.result(&task_id), "笔记写好了\n\n总结 B");
    let deliveries = h.deliveries().await;
    assert_eq!(deliveries.len(), 1, "{deliveries:?}");
    assert!(!h.has_plan(&task_id));
}

#[tokio::test]
async fn a_denied_step_ends_the_task_with_no_result_and_no_delivery() {
    let h = setup(vec![Ok(r#"["写笔记","总结"]"#)], vec![write_note()], true).await;
    let task_id = h.task("记一下再总结").await;
    let mut sandbox = FakeSandbox::new();
    h.host.pump_once(&mut sandbox).unwrap();
    let (_, dm) = http(&h.app, "POST", "/api/bros/bot-ada/dm", Some(json!({}))).await;
    let dm = dm["dmGroupId"].as_str().unwrap().to_string();
    let (_, dm_events) = http(&h.app, "GET", &format!("/api/groups/{dm}/events"), None).await;
    let id = dm_events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "approval")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = http(
        &h.app,
        "POST",
        &format!("/api/approvals/{id}/deny"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    h.host.pump_once(&mut sandbox).unwrap();
    let (state, _, failure) = h.row(&task_id);
    assert_eq!(state, "done");
    assert_eq!(failure.as_deref(), Some(robroz_card::NO_RESULT));
    assert!(h.deliveries().await.is_empty(), "no 「未执行」 delivery");
    assert!(h.fake_mcp.calls().is_empty());
    assert!(!h.has_plan(&task_id));
    assert!(sandbox.containers.is_empty());
}
