use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
use hive_contract::{
    AddMember, CreateGroup, CreateTask, ErrorBody, Event, Events, Group, Member, Message,
    MessageKind, PostMessage, Task, TaskState,
};
use rusqlite::{params, Connection};
use uuid::Uuid;

mod pump;
mod sandbox;

pub use pump::pump;
pub use sandbox::DockerSandbox;

use sandbox::Sandbox;

const TENANT_ID: &str = "local";

#[derive(Clone)]
pub struct Host {
    db: Arc<Mutex<Connection>>,
    volumes_root: std::path::PathBuf,
    image: String,
    get_limit: u64,
}

impl Host {
    pub fn open(
        db_path: &Path,
        volumes_root: impl Into<std::path::PathBuf>,
        image: impl Into<String>,
    ) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        init_schema(&conn)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            volumes_root: volumes_root.into(),
            image: image.into(),
            get_limit: get_limit_from_env(),
        })
    }

    pub fn router(&self) -> Router {
        router_from(self.db.clone(), self.volumes_root.clone())
    }

    /// One pass over queued and running tasks. The binary calls this in a loop.
    /// Tests call [`pump`] directly and do not start that loop.
    pub fn pump_once(&self, sandbox: &mut dyn Sandbox) -> rusqlite::Result<()> {
        let conn = self.db.lock().expect("sqlite connection lock poisoned");
        pump(&conn, sandbox, &self.volumes_root, &self.image, self.get_limit)
    }
}

fn get_limit_from_env() -> u64 {
    std::env::var("OPENBOTS_GET_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8)
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    volumes_root: std::path::PathBuf,
}

/// Build the host router. Opens (and creates) the SQLite file at `db_path`
/// and ensures the node-2 schema. Callers, including tests, pass an explicit
/// path; this function does not read `OPENBOTS_DB`.
pub fn router(db_path: &Path) -> rusqlite::Result<Router> {
    let host = Host::open(db_path, "./data/groups", "openbots-runner:local")?;
    Ok(host.router())
}

fn router_from(db: Arc<Mutex<Connection>>, volumes_root: std::path::PathBuf) -> Router {
    Router::new()
        .route("/api/groups", post(create_group))
        .route("/api/groups/{id}/members", post(add_member))
        .route("/api/groups/{id}/messages", post(post_message))
        .route("/api/groups/{id}/tasks", post(create_task))
        .route("/api/groups/{id}/events", get(list_events))
        .route("/api/tasks/{id}/resume", post(resume_task))
        .with_state(AppState { db, volumes_root })
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS groups (
            id TEXT PRIMARY KEY,
            tenant_id TEXT NOT NULL,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS bots (
            id TEXT PRIMARY KEY,
            tenant_id TEXT NOT NULL,
            name TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memberships (
            group_id TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            PRIMARY KEY (group_id, bot_id)
        );
        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            group_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            group_id TEXT NOT NULL,
            assignee_bot_id TEXT NOT NULL,
            prompt TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            container_up INTEGER NOT NULL DEFAULT 0,
            get_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS deliveries (
            id TEXT PRIMARY KEY,
            group_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            summary TEXT NOT NULL,
            path TEXT NOT NULL,
            briefing INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );
        ",
    )?;
    // Older databases created before node 4.
    let _ = conn.execute(
        "ALTER TABLE tasks ADD COLUMN container_up INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE tasks ADD COLUMN get_count INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.to_string(),
        }),
    )
        .into_response()
}

fn db_error(err: rusqlite::Error) -> Response {
    eprintln!("sqlite error: {err}");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "数据库错误")
}

fn lock_db(state: &AppState) -> std::sync::MutexGuard<'_, Connection> {
    state.db.lock().expect("sqlite connection lock poisoned")
}

fn group_exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM groups WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn parse_kind(value: String) -> rusqlite::Result<MessageKind> {
    MessageKind::parse(&value).ok_or_else(|| rusqlite::Error::InvalidParameterName(value))
}

fn parse_state(value: String) -> rusqlite::Result<TaskState> {
    TaskState::parse(&value).ok_or_else(|| rusqlite::Error::InvalidParameterName(value))
}

async fn create_group(State(state): State<AppState>, Json(body): Json<CreateGroup>) -> Response {
    let name = body.name.trim();
    if name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "请填写名称");
    }
    let id = new_id();
    let created_at = now_rfc3339();
    let conn = lock_db(&state);
    let result = conn.execute(
        "INSERT INTO groups (id, tenant_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, TENANT_ID, name, created_at],
    );
    drop(conn);
    if let Err(err) = result {
        return db_error(err);
    }
    (
        StatusCode::CREATED,
        Json(Group {
            id,
            name: name.to_string(),
            tenant_id: TENANT_ID.to_string(),
        }),
    )
        .into_response()
}

async fn add_member(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(body): Json<AddMember>,
) -> Response {
    let bot_id = body.bot_id.trim();
    let name = body.name.trim();
    if bot_id.is_empty() || name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "请填写名称");
    }

    let conn = lock_db(&state);
    let result = (|| {
        let tx = conn.unchecked_transaction()?;
        if !group_exists(&tx, &group_id)? {
            return Ok(None);
        }
        tx.execute(
            "INSERT OR IGNORE INTO bots (id, tenant_id, name) VALUES (?1, ?2, ?3)",
            params![bot_id, TENANT_ID, name],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO memberships (group_id, bot_id) VALUES (?1, ?2)",
            params![group_id, bot_id],
        )?;
        let stored_name: String =
            tx.query_row("SELECT name FROM bots WHERE id = ?1", [bot_id], |row| row.get(0))?;
        tx.commit()?;
        Ok(Some((inserted > 0, stored_name)))
    })();
    drop(conn);

    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some((created, stored_name))) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(Member {
                    group_id,
                    bot_id: bot_id.to_string(),
                    name: stored_name,
                }),
            )
                .into_response()
        }
        Err(err) => db_error(err),
    }
}

async fn post_message(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    // Unknown kind stays 400 with the existing error. Serde would otherwise
    // reject MessageKind before the handler and return 422.
    let kind_is_known = raw
        .get("kind")
        .and_then(|value| value.as_str())
        .and_then(MessageKind::parse)
        .is_some();
    if raw.get("kind").is_some() && !kind_is_known {
        return error_response(StatusCode::BAD_REQUEST, "kind 只能是 text、image 或 file");
    }
    let body: PostMessage = match serde_json::from_value(raw) {
        Ok(body) => body,
        Err(_) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorBody {
                    error: "请求格式不正确".to_string(),
                }),
            )
                .into_response();
        }
    };
    let text = body.body.trim();
    if text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "请填写内容");
    }

    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(None);
        }
        let id = new_id();
        let created_at = now_rfc3339();
        conn.execute(
            "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, group_id, body.kind.as_str(), text, created_at],
        )?;
        Ok(Some(Message {
            id,
            kind: body.kind,
            body: text.to_string(),
            created_at,
        }))
    })();
    drop(conn);

    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(message)) => (StatusCode::CREATED, Json(message)).into_response(),
        Err(err) => db_error(err),
    }
}

async fn create_task(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(body): Json<CreateTask>,
) -> Response {
    let prompt = body.prompt.trim();
    if prompt.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "请填写内容");
    }
    let assignee = body.assignee_bot_id.trim();

    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err(error_response(StatusCode::NOT_FOUND, "群不存在")));
        }
        let member: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memberships WHERE group_id = ?1 AND bot_id = ?2",
            params![group_id, assignee],
            |row| row.get(0),
        )?;
        if member == 0 {
            return Ok(Err(error_response(
                StatusCode::BAD_REQUEST,
                "这个 Bot 不在群里",
            )));
        }
        let id = new_id();
        let now = now_rfc3339();
        let state = TaskState::Queued;
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, group_id, assignee, prompt, state.as_str(), now, now],
        )?;
        Ok(Ok(Task {
            id,
            assignee_bot_id: assignee.to_string(),
            prompt: prompt.to_string(),
            state,
        }))
    })();
    drop(conn);

    match result {
        Ok(Ok(task)) => (StatusCode::CREATED, Json(task)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

fn status_label(state: TaskState, container_up: bool) -> String {
    if state == TaskState::Running && !container_up {
        "没写出结果".to_string()
    } else {
        state.label().to_string()
    }
}

fn strip_blocking_actions(prompt: &str) -> String {
    prompt
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed != "hive-action:external" && trimmed != "hive-action:captcha"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn resume_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let found = conn.query_row(
        "SELECT group_id, state, prompt FROM tasks WHERE id = ?1",
        [&task_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    );
    let (group_id, task_state, prompt) = match found {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return error_response(StatusCode::NOT_FOUND, "任务不存在");
        }
        Err(err) => return db_error(err),
    };
    if task_state != TaskState::Blocked.as_str() {
        return error_response(StatusCode::CONFLICT, "这次不能恢复");
    }
    let prompt = strip_blocking_actions(&prompt);
    let updated_at = now_rfc3339();
    if let Err(err) = conn.execute(
        "UPDATE tasks SET state = 'queued', container_up = 0, prompt = ?1, updated_at = ?2 WHERE id = ?3",
        params![prompt, updated_at, task_id],
    ) {
        return db_error(err);
    }
    drop(conn);
    let raw = state.volumes_root.join(&group_id);
    let _ = std::fs::remove_file(raw.join("tasks").join(&task_id).join("intent"));
    if let Ok(canon) = std::fs::canonicalize(&raw) {
        let _ = std::fs::remove_file(canon.join("tasks").join(&task_id).join("intent"));
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "id": task_id, "state": "queued" })),
    )
        .into_response()
}

struct TimedEvent {
    created_at: String,
    id: String,
    event: Event,
}

async fn list_events(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(None);
        }
        let mut timed = Vec::new();

        {
            let mut message_stmt = conn.prepare(
                "SELECT id, kind, body, created_at FROM messages WHERE group_id = ?1",
            )?;
            let messages = message_stmt.query_map([group_id.as_str()], |row| {
                let id: String = row.get(0)?;
                let created_at: String = row.get(3)?;
                Ok(TimedEvent {
                    created_at: created_at.clone(),
                    id: id.clone(),
                    event: Event::Message {
                        id,
                        kind: parse_kind(row.get(1)?)?,
                        body: row.get(2)?,
                        created_at,
                    },
                })
            })?;
            for item in messages {
                timed.push(item?);
            }
        }

        {
            let mut task_stmt = conn.prepare(
                "SELECT t.id, t.assignee_bot_id, b.name, t.state, t.created_at, t.updated_at, t.container_up
                 FROM tasks t
                 JOIN bots b ON b.id = t.assignee_bot_id
                 WHERE t.group_id = ?1",
            )?;
            let tasks = task_stmt.query_map([group_id.as_str()], |row| {
                let id: String = row.get(0)?;
                let created_at: String = row.get(4)?;
                let state = parse_state(row.get(3)?)?;
                let container_up: i64 = row.get(6)?;
                let container_up = container_up != 0;
                Ok(TimedEvent {
                    created_at: created_at.clone(),
                    id: id.clone(),
                    event: Event::Status {
                        id: id.clone(),
                        task_id: id,
                        bot_id: row.get(1)?,
                        bot_name: row.get(2)?,
                        label: status_label(state, container_up),
                        state,
                        created_at,
                        updated_at: row.get(5)?,
                        container_up,
                    },
                })
            })?;
            for item in tasks {
                timed.push(item?);
            }
        }

        {
            let mut delivery_stmt = conn.prepare(
                "SELECT id, summary, path, briefing, created_at FROM deliveries WHERE group_id = ?1",
            )?;
            let deliveries = delivery_stmt.query_map([group_id.as_str()], |row| {
                let id: String = row.get(0)?;
                let created_at: String = row.get(4)?;
                let briefing: i64 = row.get(3)?;
                Ok(TimedEvent {
                    created_at: created_at.clone(),
                    id: id.clone(),
                    event: Event::Delivery {
                        id,
                        summary: row.get(1)?,
                        path: row.get(2)?,
                        briefing: briefing != 0,
                        created_at,
                    },
                })
            })?;
            for item in deliveries {
                timed.push(item?);
            }
        }

        timed.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let events = timed.into_iter().map(|item| item.event).collect();
        Ok(Some(Events { events }))
    })();
    drop(conn);

    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(events)) => (StatusCode::OK, Json(events)).into_response(),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn temp_db() -> PathBuf {
        let path = std::env::temp_dir().join(format!("openbots-test-{}.db", uuid::Uuid::new_v4()));
        assert_ne!(
            path,
            PathBuf::from("./openbots.db"),
            "tests must not use the default database path"
        );
        path
    }

    fn test_app(path: &Path) -> Router {
        super::router(path).expect("open test database")
    }

    async fn call(app: &Router, request: Request<Body>) -> Response<Body> {
        app.clone().oneshot(request).await.expect("oneshot")
    }

    async fn read_json(response: Response<Body>) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("body").to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            panic!("response was not json: {}", String::from_utf8_lossy(&bytes))
        });
        (status, value)
    }

    fn post(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
    }

    async fn create_group(app: &Router, name: &str) -> Value {
        let (status, body) = read_json(call(app, post("/api/groups", json!({ "name": name }))).await).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    async fn add_member(app: &Router, group_id: &str, bot_id: &str, name: &str) -> (StatusCode, Value) {
        read_json(
            call(
                app,
                post(
                    &format!("/api/groups/{group_id}/members"),
                    json!({ "botId": bot_id, "name": name }),
                ),
            )
            .await,
        )
        .await
    }

    #[tokio::test]
    async fn flow_message_and_one_queued_status() {
        let path = temp_db();
        let app = test_app(&path);

        let group = create_group(&app, "设计群").await;
        assert_eq!(group["tenantId"], "local");
        assert_eq!(group["name"], "设计群");
        let group_id = group["id"].as_str().unwrap();

        let (status, member) = add_member(&app, group_id, "bot-ada", "Ada").await;
        assert_eq!(status, StatusCode::CREATED, "{member}");
        assert_eq!(member["groupId"], group_id);
        assert_eq!(member["botId"], "bot-ada");
        assert_eq!(member["name"], "Ada");

        let (status, message) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/messages"),
                    json!({ "kind": "text", "body": "hello" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{message}");
        assert_eq!(message["kind"], "text");
        assert_eq!(message["body"], "hello");
        assert!(message["createdAt"].as_str().unwrap().contains('T'));

        let (status, task) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/tasks"),
                    json!({ "assigneeBotId": "bot-ada", "prompt": "画一张草图" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        assert_eq!(
            task,
            json!({
                "id": task["id"],
                "assigneeBotId": "bot-ada",
                "prompt": "画一张草图",
                "state": "queued"
            })
        );

        let (status, events) = read_json(
            call(&app, get(&format!("/api/groups/{group_id}/events"))).await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{events}");
        let list = events["events"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{events}");

        let messages: Vec<&Value> = list.iter().filter(|e| e["type"] == "message").collect();
        let statuses: Vec<&Value> = list.iter().filter(|e| e["type"] == "status").collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(statuses.len(), 1, "exactly one status card");
        assert_eq!(messages[0]["id"], message["id"]);
        assert_eq!(messages[0]["kind"], "text");
        assert_eq!(messages[0]["body"], "hello");
        assert_eq!(messages[0]["createdAt"], message["createdAt"]);

        let card = statuses[0];
        assert_eq!(card["id"], task["id"]);
        assert_eq!(card["taskId"], task["id"]);
        assert_eq!(card["botId"], "bot-ada");
        assert_eq!(card["botName"], "Ada");
        assert_eq!(card["state"], "queued");
        assert_eq!(card["label"], "排队");
        assert!(card["createdAt"].as_str().unwrap().contains('T'));
        assert!(card["updatedAt"].as_str().unwrap().contains('T'));

        let db = Connection::open(&path).unwrap();
        let tenant: String = db
            .query_row("SELECT tenant_id FROM groups WHERE id = ?1", [group_id], |row| row.get(0))
            .unwrap();
        assert_eq!(tenant, "local");
        let bot_tenant: String = db
            .query_row("SELECT tenant_id FROM bots WHERE id = 'bot-ada'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(bot_tenant, "local");
        let task_state: String = db
            .query_row("SELECT state FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(task_state, "queued");
    }

    #[tokio::test]
    async fn task_for_non_member_is_rejected_and_absent_from_events() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "空群").await["id"].as_str().unwrap().to_string();

        let (status, body) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/tasks"),
                    json!({ "assigneeBotId": "bot-missing", "prompt": "做点什么" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "这个 Bot 不在群里" }));

        let (status, events) = read_json(
            call(&app, get(&format!("/api/groups/{group_id}/events"))).await,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events, json!({ "events": [] }));

        let db = Connection::open(&path).unwrap();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn two_tasks_produce_two_status_cards() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "双任务").await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (status, _) = add_member(&app, &group_id, "bot-ada", "Ada").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = add_member(&app, &group_id, "bot-bea", "Bea").await;
        assert_eq!(status, StatusCode::CREATED);

        for (bot, prompt) in [("bot-ada", "第一件"), ("bot-bea", "第二件")] {
            let (status, task) = read_json(
                call(
                    &app,
                    post(
                        &format!("/api/groups/{group_id}/tasks"),
                        json!({ "assigneeBotId": bot, "prompt": prompt }),
                    ),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{task}");
            assert_eq!(task["state"], "queued");
        }

        let (status, events) = read_json(
            call(&app, get(&format!("/api/groups/{group_id}/events"))).await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{events}");
        let list = events["events"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{events}");
        assert!(list.iter().all(|e| e["type"] == "status"));
        assert!(list.iter().all(|e| e["state"] == "queued" && e["label"] == "排队"));
        let mut bots: Vec<&str> = list.iter().map(|e| e["botId"].as_str().unwrap()).collect();
        bots.sort_unstable();
        assert_eq!(bots, ["bot-ada", "bot-bea"]);
        assert!(list.iter().all(|e| e["id"] == e["taskId"]));
        assert_ne!(list[0]["id"], list[1]["id"]);
        assert!(list[0]["createdAt"].as_str().unwrap() <= list[1]["createdAt"].as_str().unwrap());
    }

    #[tokio::test]
    async fn image_and_file_kinds_are_stored() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "附件群").await["id"]
            .as_str()
            .unwrap()
            .to_string();

        for (kind, body) in [("image", "pic.png"), ("file", "notes.pdf")] {
            let (status, message) = read_json(
                call(
                    &app,
                    post(
                        &format!("/api/groups/{group_id}/messages"),
                        json!({ "kind": kind, "body": body }),
                    ),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{message}");
            assert_eq!(message["kind"], kind);
            assert_eq!(message["body"], body);
        }

        let (status, events) = read_json(
            call(&app, get(&format!("/api/groups/{group_id}/events"))).await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{events}");
        let list = events["events"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|e| e["type"] == "message"));
        let image = list.iter().find(|e| e["kind"] == "image").unwrap();
        let file = list.iter().find(|e| e["kind"] == "file").unwrap();
        assert_eq!(image["body"], "pic.png");
        assert_eq!(file["body"], "notes.pdf");
        assert!(list[0]["createdAt"].as_str().unwrap() <= list[1]["createdAt"].as_str().unwrap());
    }

    #[tokio::test]
    async fn repeat_member_add_does_not_duplicate() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "成员群").await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let (first_status, first) = add_member(&app, &group_id, "bot-ada", "Ada").await;
        assert_eq!(first_status, StatusCode::CREATED, "{first}");
        let (second_status, second) = add_member(&app, &group_id, "bot-ada", "Ada").await;
        assert_eq!(second_status, StatusCode::OK, "{second}");
        assert_eq!(first, second);

        let db = Connection::open(&path).unwrap();
        let memberships: i64 = db
            .query_row("SELECT COUNT(*) FROM memberships", [], |row| row.get(0))
            .unwrap();
        let bots: i64 = db
            .query_row("SELECT COUNT(*) FROM bots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(memberships, 1);
        assert_eq!(bots, 1);
    }

    #[tokio::test]
    async fn validation_errors() {
        let path = temp_db();
        let app = test_app(&path);

        let (status, body) = read_json(call(&app, post("/api/groups", json!({ "name": "  " }))).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "请填写名称" }));

        let (status, body) = read_json(
            call(&app, post("/api/groups/missing/members", json!({ "botId": "b", "name": "n" }))).await,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "群不存在" }));

        let group_id = create_group(&app, "校验").await["id"].as_str().unwrap().to_string();

        let (status, body) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/members"),
                    json!({ "botId": "", "name": "n" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "请填写名称" }));

        let (status, body) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/messages"),
                    json!({ "kind": "video", "body": "x" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "kind 只能是 text、image 或 file" }));

        let (status, body) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/messages"),
                    json!({ "kind": "text", "body": " " }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "请填写内容" }));

        let (status, body) = read_json(
            call(&app, get("/api/groups/missing/events")).await,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "群不存在" }));

        let (status, body) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/tasks"),
                    json!({ "assigneeBotId": "bot-ada", "prompt": "" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "请填写内容" }));
    }

    fn temp_volumes() -> PathBuf {
        let path = std::env::temp_dir().join(format!("openbots-vol-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn pump_db(db: &Path, volumes: &Path, sandbox: &mut dyn super::Sandbox) {
        pump_db_limit(db, volumes, sandbox, 8);
    }

    fn pump_db_limit(db: &Path, volumes: &Path, sandbox: &mut dyn super::Sandbox, limit: u64) {
        let conn = Connection::open(db).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        super::pump(&conn, sandbox, volumes, "openbots-runner:local", limit).unwrap();
    }

    fn post_empty(uri: &str) -> Request<Body> {
        Request::builder().method("POST").uri(uri).body(Body::empty()).unwrap()
    }

    fn app_on(db: &Path, volumes: &Path) -> Router {
        super::Host::open(db, volumes, "openbots-runner:local")
            .unwrap()
            .router()
    }

    fn statuses(events: &Value) -> Vec<&Value> {
        events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "status").collect()
    }

    fn deliveries(events: &Value) -> Vec<&Value> {
        events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "delivery").collect()
    }

    async fn events_of(app: &Router, group_id: &str) -> Value {
        let (status, events) = read_json(call(app, get(&format!("/api/groups/{group_id}/events"))).await).await;
        assert_eq!(status, StatusCode::OK, "{events}");
        events
    }

    async fn seed_task(app: &Router, group_name: &str, prompt: &str) -> (String, String) {
        let group_id = create_group(app, group_name).await["id"].as_str().unwrap().to_string();
        let (status, _) = add_member(app, &group_id, "bot-ada", "Ada").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, task) = read_json(
            call(
                app,
                post(
                    &format!("/api/groups/{group_id}/tasks"),
                    json!({ "assigneeBotId": "bot-ada", "prompt": prompt }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        assert_eq!(task["state"], "queued");
        (group_id, task["id"].as_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn created_task_without_pump_stays_queued_without_delivery() {
        let path = temp_db();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "未推进", "先别跑").await;
        let events = events_of(&app, &group_id).await;
        let cards = statuses(&events);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["id"], task_id);
        assert_eq!(cards[0]["state"], "queued");
        assert_eq!(cards[0]["label"], "排队");
        assert_eq!(cards[0]["containerUp"], false);
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn pump_finishes_container_keeps_file_and_delivers() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let prompt = "画一张草图";
        let (group_id, task_id) = seed_task(&app, "沙箱", prompt).await;
        let before = events_of(&app, &group_id).await;
        let created_at = statuses(&before)[0]["createdAt"].as_str().unwrap().to_string();
        let updated_queued = statuses(&before)[0]["updatedAt"].as_str().unwrap().to_string();

        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.names(), vec![format!("openbots-{task_id}")]);

        let running = events_of(&app, &group_id).await;
        let cards = statuses(&running);
        assert_eq!(cards.len(), 1, "same status card, not one per state");
        assert_eq!(cards[0]["id"], task_id);
        assert_eq!(cards[0]["taskId"], task_id);
        assert_eq!(cards[0]["state"], "running");
        assert_eq!(cards[0]["label"], "进行");
        assert_eq!(cards[0]["containerUp"], true);
        assert_eq!(cards[0]["createdAt"], created_at);
        assert_ne!(cards[0]["updatedAt"], updated_queued);
        assert!(deliveries(&running).is_empty());

        fake.finish(0);
        let group_dir = fake.containers[0].group_dir.clone();
        let recorded = group_dir.join(format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read(&recorded).unwrap(), prompt.as_bytes());
        assert!(group_dir.ends_with(&group_id));

        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty(), "container list is empty after finish");
        assert!(group_dir.is_dir(), "group directory remains");
        assert!(recorded.is_file(), "file remains after the container is removed");
        assert_eq!(std::fs::read(&recorded).unwrap(), prompt.as_bytes());

        let done = events_of(&app, &group_id).await;
        let cards = statuses(&done);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["id"], task_id);
        assert_eq!(cards[0]["state"], "done");
        assert_eq!(cards[0]["label"], "完成");
        assert_eq!(cards[0]["containerUp"], false);
        assert_eq!(cards[0]["createdAt"], created_at);
        assert_ne!(cards[0]["updatedAt"], created_at);
        let delivered = deliveries(&done);
        assert_eq!(delivered.len(), 1);
        let rel = format!("tasks/{task_id}/result.txt");
        assert_eq!(delivered[0]["path"], rel);
        assert!(!rel.starts_with('/'));
        assert!(!rel.contains("workspace"));
        assert_eq!(delivered[0]["summary"], "已写入结果");
        assert_eq!(delivered[0]["briefing"], false);
        assert_eq!(delivered[0]["type"], "delivery");
    }

    #[tokio::test]
    async fn start_failure_leaves_task_queued_without_delivery() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "起不来", "不要容器").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        fake.fail_start = true;
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        let cards = statuses(&events);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["id"], task_id);
        assert_eq!(cards[0]["state"], "queued");
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn nonzero_exit_removes_container_and_stays_running() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, _) = seed_task(&app, "失败退出", "会失败").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish(1);
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "running");
        assert_eq!(statuses(&events)[0]["label"], "没写出结果");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn missing_file_removes_container_and_stays_running() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, _) = seed_task(&app, "没文件", "没有结果").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish_without_output(0);
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "running");
        assert_eq!(statuses(&events)[0]["label"], "没写出结果");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn groups_do_not_share_a_volume() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_a, _) = seed_task(&app, "甲", "a").await;
        let (group_b, _) = seed_task(&app, "乙", "b").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.containers.len(), 2);
        let dir_a = fake
            .containers
            .iter()
            .find(|c| c.group_dir.ends_with(&group_a))
            .unwrap()
            .group_dir
            .clone();
        let dir_b = fake
            .containers
            .iter()
            .find(|c| c.group_dir.ends_with(&group_b))
            .unwrap()
            .group_dir
            .clone();
        assert_ne!(dir_a, dir_b);
        assert!(dir_a.is_dir());
        assert!(dir_b.is_dir());

        let (status, _) = add_member(&app, &group_a, "bot-bea", "Bea").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_a}/tasks"),
                    json!({ "assigneeBotId": "bot-bea", "prompt": "again" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        pump_db(&path, &volumes, &mut fake);
        let again = fake.containers.iter().find(|c| c.prompt == "again").unwrap();
        assert_eq!(again.group_dir, dir_a);
    }


    #[tokio::test]
    async fn get_under_limit_still_finishes_done() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let prompt = "hive-action:get\n画一张草图";
        let (group_id, task_id) = seed_task(&app, "配额内", prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db_limit(&path, &volumes, &mut fake, 8);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["containerUp"], true);
        fake.finish(0);
        let group_dir = fake.containers[0].group_dir.clone();
        pump_db_limit(&path, &volumes, &mut fake, 8);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "done");
        assert_eq!(statuses(&events)[0]["label"], "完成");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert_eq!(deliveries(&events).len(), 1);
        assert_eq!(deliveries(&events)[0]["path"], format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read(group_dir.join(format!("tasks/{task_id}/result.txt"))).unwrap(), prompt.as_bytes());
        let conn = Connection::open(&path).unwrap();
        let gets: i64 = conn.query_row("SELECT get_count FROM tasks WHERE id = ?1", [&task_id], |row| row.get(0)).unwrap();
        assert_eq!(gets, 1);
    }

    #[tokio::test]
    async fn get_over_limit_is_dead_run_not_blocked() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let prompt = "hive-action:get\nhive-action:get\nhive-action:get";
        let (group_id, _) = seed_task(&app, "超限", prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db_limit(&path, &volumes, &mut fake, 2);
        fake.finish(0);
        pump_db_limit(&path, &volumes, &mut fake, 2);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        let card = statuses(&events)[0];
        assert_eq!(card["state"], "running");
        assert_eq!(card["label"], "没写出结果");
        assert_eq!(card["containerUp"], false);
        assert_ne!(card["state"], "blocked");
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn non_get_outbound_fails_without_blocking() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "非GET", "普通写入").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish(0);
        let outbound = fake.containers[0].group_dir.join(format!("tasks/{task_id}/outbound.txt"));
        std::fs::write(&outbound, "POST\n").unwrap();
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "running");
        assert_eq!(statuses(&events)[0]["label"], "没写出结果");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn captcha_blocks_like_external() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "验证码", "hive-action:captcha").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish(0);
        let group_dir = fake.containers[0].group_dir.clone();
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        assert!(!group_dir.join(format!("tasks/{task_id}/result.txt")).exists());
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "blocked");
        assert_eq!(statuses(&events)[0]["label"], "卡住");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn external_blocks_then_resume_rewrites_on_same_directory() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = "hive-action:external\n画一张草图";
        let (group_id, task_id) = seed_task(&app, "外呼", prompt).await;

        let (status, body) = read_json(call(&app, post_empty("/api/tasks/missing/resume")).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "任务不存在" }));
        let (status, body) = read_json(call(&app, post_empty(&format!("/api/tasks/{task_id}/resume"))).await).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body, json!({ "error": "这次不能恢复" }));

        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish(0);
        let group_dir = fake.containers[0].group_dir.clone();
        assert_eq!(std::fs::read(group_dir.join(format!("tasks/{task_id}/intent"))).unwrap(), b"external");
        assert!(!group_dir.join(format!("tasks/{task_id}/result.txt")).exists());
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let blocked = events_of(&app, &group_id).await;
        assert_eq!(statuses(&blocked)[0]["state"], "blocked");
        assert_eq!(statuses(&blocked)[0]["label"], "卡住");
        assert_eq!(statuses(&blocked)[0]["containerUp"], false);
        assert!(deliveries(&blocked).is_empty());

        let (status, body) = read_json(call(&app, post_empty(&format!("/api/tasks/{task_id}/resume"))).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "id": task_id, "state": "queued" }));
        assert!(!group_dir.join(format!("tasks/{task_id}/intent")).exists());
        let queued = events_of(&app, &group_id).await;
        assert_eq!(statuses(&queued)[0]["state"], "queued");
        assert_eq!(statuses(&queued)[0]["containerUp"], false);

        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.containers.len(), 1);
        assert_eq!(fake.containers[0].group_dir, group_dir);
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let result = group_dir.join(format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read(&result).unwrap(), "画一张草图".as_bytes());
        assert!(group_dir.is_dir());
        let done = events_of(&app, &group_id).await;
        assert_eq!(statuses(&done)[0]["state"], "done");
        assert_eq!(statuses(&done)[0]["label"], "完成");
        assert_eq!(statuses(&done)[0]["containerUp"], false);
        assert_eq!(deliveries(&done).len(), 1);
        assert_eq!(deliveries(&done)[0]["path"], format!("tasks/{task_id}/result.txt"));
        assert_eq!(deliveries(&done)[0]["briefing"], false);
    }

}
