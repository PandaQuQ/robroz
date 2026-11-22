use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
use robroz_contract::{
    AddMember, CreateGroup, CreateSchedule, CreateTask, ErrorBody, Event, Events, Group, Groups, Member,
    Message, MessageKind, PostMessage, Schedule, Task, TaskState, WaitingFor,
};
use rusqlite::{params, Connection};
use uuid::Uuid;

mod bros;
pub mod auth;
mod approvals;
mod choices;
mod memories;
mod skills;
mod progress;
mod proposals;
mod pump;
mod tools;
mod search;
mod settings;
mod model_settings;
mod mcp;
mod mcp_tools;
mod routines;
mod sandbox;
mod say;
mod schedule;
mod unread;
mod stream;
mod text;

pub use pump::{pump, pump_with, Db, PumpConfig};
pub use schedule::{tick, TickReport};
pub use sandbox::DockerSandbox;

use sandbox::Sandbox;

/// Default root for per-task control dirs, outside the group volumes.
pub const DEFAULT_CONTROL_ROOT: &str = "./data/control";

const TENANT_ID: &str = "local";

/// Check the process environment for a conflicting or invalid model key.
/// Incomplete model env (missing URL or name) is not an error.
pub fn validate_model_env() -> Result<(), String> {
    model_settings::env_key_conflict()
}

#[derive(Clone)]
pub struct Host {
    db: Arc<Mutex<Connection>>,
    volumes_root: std::path::PathBuf,
    control_root: std::path::PathBuf,
    image: String,
    get_limit: u64,
    token: Option<auth::ApiToken>,
    model: Option<Arc<dyn robroz_model::Model>>,
    mcp: Arc<dyn crate::mcp_tools::McpRuntime>,
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
            control_root: std::path::PathBuf::from(DEFAULT_CONTROL_ROOT),
            image: image.into(),
            get_limit: get_limit_from_env(),
            token: None,
            model: None,
            mcp: crate::mcp_tools::real_mcp(),
        })
    }

    /// Root for per-task control dirs (`{root}/{taskId}/takeover`). Only the host
    /// writes there; containers get their own task dir read-only at `/control`.
    pub fn with_control_root(mut self, control_root: impl Into<std::path::PathBuf>) -> Self {
        self.control_root = control_root.into();
        self
    }

    /// Require `Authorization: Bearer <token>` on every request. `None` keeps it open.
    pub fn with_token(mut self, token: Option<auth::ApiToken>) -> Self {
        self.token = token;
        self
    }

    /// Let bros think with `model`. Tests pass a fake this way. Production
    /// leaves this unset so each pump reads the latest settings and env.
    pub fn with_model(mut self, model: Option<Arc<dyn robroz_model::Model>>) -> Self {
        self.model = model;
        self
    }

    /// Swap the MCP connector. Tests pass a fake; production keeps the official client.
    pub fn with_mcp(mut self, mcp: Arc<dyn crate::mcp_tools::McpRuntime>) -> Self {
        self.mcp = mcp;
        self
    }

    pub fn router(&self) -> Router {
        let router = router_from(AppState {
            db: self.db.clone(),
            control_root: self.control_root.clone(),
        });
        auth::require_token(router, self.token.clone())
    }

    /// One pass over queued and running tasks. The binary calls this in a loop on
    /// its own thread. The DB lock is only held for SQL, never across docker calls.
    /// Tests call [`pump`] directly and do not start that loop.
    pub fn pump_once(&self, sandbox: &mut dyn Sandbox) -> rusqlite::Result<()> {
        let mut tools = crate::tools::ToolRegistry::new();
        crate::search::register_tools(&mut tools, self.db.clone());
        enum Slot {
            Override,
            Live(robroz_model::OpenAiCompat),
            Echo,
            Missing,
        }
        let slot = if self.model.is_some() {
            Slot::Override
        } else {
            let conn = self.db.lock().expect("sqlite connection lock poisoned");
            match crate::model_settings::live_model(&conn) {
                Ok(Some(cfg)) => Slot::Live(robroz_model::OpenAiCompat::new(cfg)),
                Ok(None) if crate::model_settings::echo_mode(&conn) => Slot::Echo,
                Ok(None) => Slot::Missing,
                Err(err) => {
                    eprintln!("pump: model config: {err}");
                    if crate::model_settings::echo_mode(&conn) {
                        Slot::Echo
                    } else {
                        Slot::Missing
                    }
                }
            }
        };
        let model = match &slot {
            Slot::Override => self.model.as_deref(),
            Slot::Live(model) => Some(model as &dyn robroz_model::Model),
            Slot::Echo | Slot::Missing => None,
        };
        let echo_prompt = matches!(slot, Slot::Echo);
        pump_with(
            &*self.db,
            sandbox,
            &PumpConfig {
                volumes_root: &self.volumes_root,
                control_root: &self.control_root,
                image: &self.image,
                get_limit: self.get_limit,
                model,
                echo_prompt,
                tools: &tools,
                mcp: Some(self.mcp.clone()),
            },
        )
    }

    /// One scheduler pass at `now`. Only creates queued tasks; pump starts containers.
    pub fn tick_once(&self, now: chrono::DateTime<Utc>) -> rusqlite::Result<TickReport> {
        let conn = self.db.lock().expect("sqlite connection lock poisoned");
        tick(&conn, now)
    }
}

fn get_limit_from_env() -> u64 {
    std::env::var("ROBROZ_GET_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8)
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    control_root: std::path::PathBuf,
}

/// Build the host router. Opens (and creates) the SQLite file at `db_path`
/// and ensures the node-2 schema. Callers, including tests, pass an explicit
/// path; this function does not read `ROBROZ_DB`.
pub fn router(db_path: &Path) -> rusqlite::Result<Router> {
    let host = Host::open(db_path, "./data/groups", "robroz-runner:local")?;
    Ok(host.router())
}

fn router_from(state: AppState) -> Router {
    Router::new()
        .route("/api/groups", post(create_group).get(list_groups))
        .route("/api/groups/{id}/members", post(add_member))
        .route("/api/bros", post(bros::create_bro).get(bros::list_bros))
        .route("/api/bros/{id}", axum::routing::patch(bros::rename_bro))
        .route("/api/bros/{id}/dm", post(bros::open_dm))
        .route("/api/groups/{id}/messages", post(post_message))
        .route("/api/groups/{id}/tasks", post(create_task))
        .route("/api/groups/{id}/say", post(say::say))
        .route("/api/groups/{id}/events", get(list_events))
        .route("/api/groups/{id}/unread", get(unread::group_unread))
        .route("/api/groups/{id}/read", post(unread::mark_read))
        .route("/api/unread", get(unread::list_unread))
        .route("/api/groups/{id}/stream", get(stream::stream_events))
        .route("/api/tasks/{id}/resume", post(resume_task))
        .route("/api/schedules", post(create_schedule))
        .route("/api/groups/{id}/schedules", get(routines::list_schedules))
        .route("/api/schedules/{id}", patch(routines::update_schedule))
        .route("/api/groups/{id}/choices", post(choices::create_choice))
        .route("/api/choices/{id}/answer", post(choices::answer_choice))
        .route("/api/groups/{id}/approvals", post(approvals::create_approval))
        .route("/api/approvals/{id}/approve", post(approvals::approve))
        .route("/api/approvals/{id}/deny", post(approvals::deny))
        .route("/api/memories", get(memories::list_memories).post(memories::create_memory))
        .route(
            "/api/memories/{id}",
            patch(memories::update_memory).delete(memories::forget_memory),
        )
        .route("/api/skills", get(skills::list_skills).post(skills::create_skill))
        .route(
            "/api/skills/{slug}",
            patch(skills::update_skill).delete(skills::forget_skill),
        )
        .route("/api/groups/{id}/skills", get(skills::list_group_skills))
        .route(
            "/api/groups/{id}/skills/{slug}",
            put(skills::enable_skill).delete(skills::disable_skill),
        )
        .route("/api/settings/search", get(search::get_search).put(search::put_search))
        .route("/api/settings/search/test", post(search::test_search))
        .route(
            "/api/settings/model",
            get(model_settings::get_model).put(model_settings::put_model),
        )
        .route("/api/settings/model/test", post(model_settings::test_model))
        .route("/api/mcp/servers", get(mcp::list_servers).post(mcp::create_server))
        .route(
            "/api/mcp/servers/{id}",
            patch(mcp::patch_server).delete(mcp::delete_server),
        )
        .route("/api/mcp/servers/{id}/tools", get(mcp::list_tools))
        .route("/api/mcp/servers/{id}/test", post(mcp::test_connection))
        .route(
            "/api/mcp/servers/{id}/approve-command",
            post(mcp::approve_command),
        )
        .route("/api/groups/{id}/mcp", get(mcp::list_group_mcp))
        .route(
            "/api/groups/{id}/mcp/{serverId}",
            get(mcp::get_group_mcp).put(mcp::enable_group_mcp).delete(mcp::disable_group_mcp),
        )
        .with_state(state)
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
            get_count INTEGER NOT NULL DEFAULT 0,
            briefing INTEGER NOT NULL DEFAULT 0,
            schedule_id TEXT,
            intent_kind TEXT,
            intent_line TEXT,
            intent_hash TEXT
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
        CREATE TABLE IF NOT EXISTS schedules (
            id TEXT PRIMARY KEY,
            tenant_id TEXT NOT NULL,
            group_id TEXT NOT NULL,
            assignee_bot_id TEXT NOT NULL,
            cron TEXT NOT NULL,
            prompt TEXT NOT NULL,
            next_run_at TEXT NOT NULL,
            last_task_id TEXT,
            created_at TEXT NOT NULL,
            paused_at TEXT
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
    // Node 5: scheduled tasks are briefing tasks.
    let _ = conn.execute(
        "ALTER TABLE tasks ADD COLUMN briefing INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE tasks ADD COLUMN schedule_id TEXT", []);
    // Blocked intent captured by the host at container exit; resume reads only this.
    for column in ["intent_kind", "intent_line", "intent_hash"] {
        let _ = conn.execute(&format!("ALTER TABLE tasks ADD COLUMN {column} TEXT"), []);
    }
    // Routines: a paused schedule is skipped by tick.
    let _ = conn.execute("ALTER TABLE schedules ADD COLUMN paused_at TEXT", []);
    conn.execute_batch(choices::SCHEMA)?;
    conn.execute_batch(approvals::SCHEMA)?;
    // Added later; ignore "duplicate column" on databases that have it.
    for table in ["choices", "approvals"] {
        let _ = conn.execute(&format!("ALTER TABLE {table} ADD COLUMN answered_by TEXT"), []);
    }
    // Approval cards a blocked task raised: which task and which blocking line.
    for column in ["task_id", "intent_hash"] {
        let _ = conn.execute(&format!("ALTER TABLE approvals ADD COLUMN {column} TEXT"), []);
    }
    // Confirm cards: what a bro proposed (`subject_kind`, `subject_id`).
    proposals::migrate(conn)?;
    // Why a run did not get done; copied from the task onto its delivery.
    for table in ["tasks", "deliveries"] {
        let _ = conn.execute(&format!("ALTER TABLE {table} ADD COLUMN failure TEXT"), []);
    }
    // What a running task is doing now, shown on its status card.
    let _ = conn.execute("ALTER TABLE tasks ADD COLUMN progress TEXT", []);
    bros::migrate(conn)?;
    conn.execute_batch(unread::SCHEMA)?;
    conn.execute_batch(memories::SCHEMA)?;
    conn.execute_batch(skills::SCHEMA)?;
    let _ = conn.execute("ALTER TABLE tasks ADD COLUMN needs_setup TEXT", []);
    let _ = conn.execute("ALTER TABLE deliveries ADD COLUMN needs_setup TEXT", []);
    crate::settings::migrate(conn)?;
    crate::search::migrate(conn)?;
    crate::mcp::migrate(conn)?;
    for column in ["mcp_server", "mcp_tool"] {
        let _ = conn.execute(&format!("ALTER TABLE approvals ADD COLUMN {column} TEXT"), []);
    }
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

async fn list_groups(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let mut stmt = conn.prepare(
            "SELECT id, name, tenant_id FROM groups WHERE tenant_id = ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([TENANT_ID], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
                tenant_id: row.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })();
    drop(conn);
    match result {
        Ok(groups) => (StatusCode::OK, Json(Groups { groups })).into_response(),
        Err(err) => db_error(err),
    }
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
        // A private chat keeps exactly its one bro.
        if bros::dm_owner(&tx, &group_id)?.is_some_and(|owner| owner != bot_id) {
            return Ok(Some(Err(())));
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
        Ok(Some(Ok((inserted > 0, stored_name))))
    })();
    drop(conn);

    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(Err(()))) => error_response(StatusCode::CONFLICT, "私聊里只能有一个 bro"),
        Ok(Some(Ok((created, stored_name)))) => {
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

const RESUME_NO_INTENT: &str = "恢复失败，找不到卡住记录";
const RESUME_MARKER_FAILED: &str = "恢复失败，无法写入接管标记";
const RESUME_NEEDS_APPROVAL: &str = "请先回答审批卡";

/// Resume a blocked task. Never reads the group volume.
///
/// 1. Take the blocking line the host computed when the container exited (from
///    the stored prompt and takeover hashes). None stored: 500, stays blocked.
/// 2. Append its sha256 to `{control_root}/{id}/takeover`. Only the host writes
///    there. Failure: 500, stays blocked.
/// 3. Clear the stored intent and set the task back to queued. The next pump
///    starts a container that mounts the control dir read-only and skips exactly
///    that line.
///
/// The DB lock is held throughout so two resumes can't race; this is only a
/// small local file write, never docker.
async fn resume_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let found = conn.query_row(
        "SELECT state, intent_hash, intent_kind FROM tasks WHERE id = ?1",
        [&task_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let (task_state, intent_hash, intent_kind) = match found {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return error_response(StatusCode::NOT_FOUND, "任务不存在");
        }
        Err(err) => return db_error(err),
    };
    if task_state != TaskState::Blocked.as_str() {
        return error_response(StatusCode::CONFLICT, "这次不能恢复");
    }
    if intent_kind.as_deref() == Some("mcp") {
        return error_response(StatusCode::CONFLICT, RESUME_NEEDS_APPROVAL);
    }
    let Some(hash) = intent_hash else {
        eprintln!("resume {task_id}: no stored intent");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, RESUME_NO_INTENT);
    };

    let control_dir = state.control_root.join(&task_id);
    if let Err(err) = robroz_runner::append_takeover(&control_dir, &hash) {
        eprintln!("resume {task_id}: write takeover in {}: {err}", control_dir.display());
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, RESUME_MARKER_FAILED);
    }

    let updated_at = now_rfc3339();
    if let Err(err) = conn.execute(
        "UPDATE tasks SET state = 'queued', container_up = 0, updated_at = ?1,
                          intent_kind = NULL, intent_line = NULL, intent_hash = NULL
         WHERE id = ?2 AND state = 'blocked'",
        params![updated_at, task_id],
    ) {
        return db_error(err);
    }
    // Taking over answers the approval card for the same line, if one is open.
    if let Err(err) = conn.execute(
        "UPDATE approvals SET decision = 'approved', answered_at = ?1
         WHERE task_id = ?2 AND intent_hash = ?3 AND decision IS NULL",
        params![updated_at, task_id, hash],
    ) {
        return db_error(err);
    }
    drop(conn);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "id": task_id, "state": "queued" })),
    )
        .into_response()
}

async fn create_schedule(
    State(state): State<AppState>,
    Json(body): Json<CreateSchedule>,
) -> Response {
    let group_id = body.group_id.trim().to_string();
    let assignee = body.assignee_bot_id.trim().to_string();
    let prompt = body.prompt.trim().to_string();
    let cron_text = body.cron.trim().to_string();

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
        if prompt.is_empty() {
            return Ok(Err(error_response(StatusCode::BAD_REQUEST, "请填写内容")));
        }
        let Some(next) = schedule::parse_cron(&cron_text)
            .and_then(|cron| schedule::next_after(&cron, Utc::now()))
        else {
            return Ok(Err(error_response(StatusCode::BAD_REQUEST, "cron 格式不对")));
        };
        let id = new_id();
        let next_run_at = schedule::stamp(next);
        conn.execute(
            "INSERT INTO schedules (id, tenant_id, group_id, assignee_bot_id, cron, prompt,
                                    next_run_at, last_task_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
            params![id, TENANT_ID, group_id, assignee, cron_text, prompt, next_run_at, now_rfc3339()],
        )?;
        Ok(Ok(Schedule {
            id,
            group_id: group_id.clone(),
            assignee_bot_id: assignee.clone(),
            cron: cron_text.clone(),
            prompt: prompt.clone(),
            next_run_at,
            last_task_id: None,
        }))
    })();
    drop(conn);

    match result {
        Ok(Ok(schedule)) => (StatusCode::CREATED, Json(schedule)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

struct TimedEvent {
    created_at: String,
    /// When this card last changed: `updatedAt` for status cards, `answeredAt`
    /// for answered choices, otherwise `createdAt`. Drives `?since=`.
    changed_at: String,
    id: String,
    event: Event,
}

/// `GET /api/groups/{id}/events?since=<RFC3339>`: only cards that changed after
/// `since`. Same body shape; an empty list means nothing changed, so a client
/// polling with `since` has nothing to show and posts nothing.
type EventsQuery = std::collections::HashMap<String, String>;

fn parse_since(raw: Option<&str>) -> Result<Option<String>, ()> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => chrono::DateTime::parse_from_rfc3339(v)
            .map(|t| Some(t.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true)))
            .map_err(|_| ()),
    }
}

async fn list_events(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Query(query): Query<EventsQuery>,
) -> Response {
    let Ok(since) = parse_since(query.get("since").map(String::as_str)) else {
        return error_response(StatusCode::BAD_REQUEST, "since 不是合法时间");
    };
    let conn = lock_db(&state);
    let result = group_events(&conn, &group_id, since.as_deref())
        .map(|found| found.map(|timed| Events { events: timed.into_iter().map(|item| item.event).collect() }));
    drop(conn);

    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(events)) => (StatusCode::OK, Json(events)).into_response(),
        Err(err) => db_error(err),
    }
}

/// Every card in a group, oldest first, optionally only those that changed after
/// `since` (normalised RFC3339 millis). `None` when the group does not exist.
/// Shared by `GET …/events` and the live stream.
fn group_events(conn: &Connection, group_id: &str, since: Option<&str>) -> rusqlite::Result<Option<Vec<TimedEvent>>> {
    if !group_exists(conn, group_id)? {
        return Ok(None);
    }
    let mut timed = Vec::new();

    {
        let mut message_stmt = conn.prepare(
            "SELECT id, kind, body, created_at FROM messages WHERE group_id = ?1",
        )?;
        let messages = message_stmt.query_map([group_id], |row| {
            let id: String = row.get(0)?;
            let created_at: String = row.get(3)?;
            Ok(TimedEvent {
                changed_at: created_at.clone(),
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
            "SELECT t.id, t.assignee_bot_id, b.name, t.state, t.created_at, t.updated_at, t.container_up, t.failure,
                    EXISTS(
                        SELECT 1 FROM approvals a
                        WHERE a.task_id = t.id AND a.intent_hash = t.intent_hash AND a.decision IS NULL
                    ),
                    t.progress
             FROM tasks t
             JOIN bots b ON b.id = t.assignee_bot_id
             WHERE t.group_id = ?1",
        )?;
        let tasks = task_stmt.query_map([group_id], |row| {
            let id: String = row.get(0)?;
            let created_at: String = row.get(4)?;
            let state = parse_state(row.get(3)?)?;
            let container_up: i64 = row.get(6)?;
            let container_up = container_up != 0;
            let updated_at: String = row.get(5)?;
            // Only a finished task reports why it was not done.
            let failure: Option<String> = if state == TaskState::Done { row.get(7)? } else { None };
            // Only a blocked task says whether it is waiting for approval or a takeover.
            let waiting_for = if state == TaskState::Blocked {
                let open_approval: i64 = row.get(8)?;
                Some(if open_approval != 0 {
                    WaitingFor::Approval
                } else {
                    WaitingFor::Takeover
                })
            } else {
                None
            };
            let progress = progress::shown(state, container_up, row.get(9)?);
            Ok(TimedEvent {
                changed_at: updated_at.clone(),
                created_at: created_at.clone(),
                id: id.clone(),
                event: Event::Status {
                    id: id.clone(),
                    task_id: id,
                    bot_id: row.get(1)?,
                    bot_name: row.get(2)?,
                    label: robroz_card::status_line(state, container_up).to_string(),
                    state,
                    created_at,
                    updated_at,
                    container_up,
                    failure,
                    waiting_for,
                    progress,
                },
            })
        })?;
        for item in tasks {
            timed.push(item?);
        }
    }

    {
        let mut delivery_stmt = conn.prepare(
            "SELECT id, summary, path, briefing, created_at, failure, needs_setup FROM deliveries WHERE group_id = ?1",
        )?;
        let deliveries = delivery_stmt.query_map([group_id], |row| {
            let id: String = row.get(0)?;
            let created_at: String = row.get(4)?;
            let briefing: i64 = row.get(3)?;
            Ok(TimedEvent {
                changed_at: created_at.clone(),
                created_at: created_at.clone(),
                id: id.clone(),
                event: Event::Delivery {
                    id,
                    summary: row.get(1)?,
                    path: row.get(2)?,
                    briefing: briefing != 0,
                    created_at,
                    failure: row.get(5)?,
                    needs_setup: row.get(6)?,
                },
            })
        })?;
        for item in deliveries {
            timed.push(item?);
        }
    }

    timed.extend(choices::timed_events(conn, group_id)?);
    timed.extend(approvals::timed_events(conn, group_id)?);
    timed.extend(proposals::timed_events(conn, group_id)?);
    if let Some(since) = since {
        timed.retain(|item| item.changed_at.as_str() > since);
    }

    timed.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(Some(timed))
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
        let path = std::env::temp_dir().join(format!("robroz-test-{}.db", uuid::Uuid::new_v4()));
        assert_ne!(
            path,
            PathBuf::from("./robroz.db"),
            "tests must not use the default database path"
        );
        path
    }

    /// Volumes and control dirs go to temp, never `./data` in the repo.
    fn test_app(path: &Path) -> Router {
        app_on(path, &temp_volumes())
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
        let path = std::env::temp_dir().join(format!("robroz-vol-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn pump_db(db: &Path, volumes: &Path, sandbox: &mut dyn super::Sandbox) {
        pump_db_limit(db, volumes, sandbox, 8);
    }

    fn pump_db_limit(db: &Path, volumes: &Path, sandbox: &mut dyn super::Sandbox, limit: u64) {
        let conn = Connection::open(db).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        super::pump(&conn, sandbox, volumes, &control_of(volumes), "robroz-runner:local", limit).unwrap();
    }

    fn post_empty(uri: &str) -> Request<Body> {
        Request::builder().method("POST").uri(uri).body(Body::empty()).unwrap()
    }

    /// Control root next to (never inside) the test volumes root.
    fn control_of(volumes: &Path) -> PathBuf {
        volumes.with_extension("control")
    }

    fn app_on(db: &Path, volumes: &Path) -> Router {
        super::Host::open(db, volumes, "robroz-runner:local")
            .unwrap()
            .with_control_root(control_of(volumes))
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
        assert_eq!(fake.names(), vec![format!("robroz-{task_id}")]);

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
        let prompt = "robroz-action:get\n画一张草图";
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

    /// Tier Metered: the GETs of one run against `limit`, ending as done.
    async fn run_with_gets(gets: usize, limit: u64, extra: Option<&str>) -> (Router, String, String) {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let prompt = vec!["robroz-action:get"; gets].join("\n") + "\n画一张草图";
        let (group_id, task_id) = seed_task(&app, "出网", &prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db_limit(&path, &volumes, &mut fake, limit);
        fake.finish(0);
        if let Some(line) = extra {
            let outbound = fake.containers[0].group_dir.join(format!("tasks/{task_id}/outbound.txt"));
            std::fs::write(&outbound, line).unwrap();
        }
        pump_db_limit(&path, &volumes, &mut fake, limit);
        assert!(fake.names().is_empty(), "container removed");
        (app, group_id, task_id)
    }

    #[tokio::test]
    async fn get_at_limit_finishes_done() {
        let (app, group_id, _) = run_with_gets(2, 2, None).await;
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "done");
        assert!(statuses(&events)[0].get("failure").is_none());
        assert_eq!(deliveries(&events).len(), 1);
    }

    #[tokio::test]
    async fn get_over_limit_fails_done_not_blocked() {
        let (app, group_id, _) = run_with_gets(3, 2, None).await;
        let events = events_of(&app, &group_id).await;
        let card = statuses(&events)[0];
        assert_eq!(card["state"], "done");
        assert_eq!(card["failure"], "出网超额：已用 3 次，上限 2");
        assert_eq!(card["containerUp"], false);
        assert!(deliveries(&events).is_empty(), "no successful delivery");
        assert!(events["events"].as_array().unwrap().iter().all(|e| e["type"] != "approval"));
    }

    #[tokio::test]
    async fn non_get_outbound_fails_without_blocking() {
        let (app, group_id, _) = run_with_gets(0, 8, Some("POST\n")).await;
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "done");
        assert_eq!(statuses(&events)[0]["failure"], "出网只允许 GET：用了 POST");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert!(deliveries(&events).is_empty());
    }

    /// Tier Allow: a workspace write finishes without any card for a person.
    #[tokio::test]
    async fn workspace_write_needs_no_card() {
        let (app, group_id, _) = run_with_gets(0, 8, None).await;
        let events = events_of(&app, &group_id).await;
        let kinds: Vec<&str> = events["events"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert!(kinds.iter().all(|k| matches!(*k, "message" | "status" | "delivery")), "{kinds:?}");
        assert_eq!(statuses(&events)[0]["state"], "done");
        assert!(statuses(&events)[0].get("failure").is_none());
        assert_eq!(deliveries(&events).len(), 1);
    }

    #[tokio::test]
    async fn captcha_blocks_like_external() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "验证码", "robroz-action:captcha").await;
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
        let prompt = "robroz-action:external\n画一张草图";
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
        let intent = std::fs::read_to_string(group_dir.join(format!("tasks/{task_id}/intent"))).unwrap();
        assert_eq!(intent.lines().next(), Some("external"));
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


    // ---- bros and private chats ----

    async fn new_bro(app: &Router, body: Option<Value>) -> (StatusCode, Value) {
        read_json(call(app, match body { Some(b) => post("/api/bros", b), None => post_empty("/api/bros") }).await).await
    }

    #[tokio::test]
    async fn new_bros_get_default_names_and_a_private_chat() {
        let app = test_app(&temp_db());
        let mut names = Vec::new();
        for _ in 0..3 {
            let (status, bro) = new_bro(&app, None).await;
            assert_eq!(status, StatusCode::CREATED, "{bro}");
            names.push(bro["name"].as_str().unwrap().to_string());
        }
        assert_eq!(names, ["bro", "bro 2", "bro 3"]);
        let (status, named) = new_bro(&app, Some(json!({ "name": "  小明 " }))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(named["name"], "小明");
        let (_, next) = new_bro(&app, Some(json!({}))).await;
        assert_eq!(next["name"], "bro 4");

        // The DM is an ordinary group with that one bro, usable for tasks right away.
        let dm = named["dmGroupId"].as_str().unwrap().to_string();
        let (_, groups) = read_json(call(&app, get("/api/groups")).await).await;
        assert!(groups["groups"].as_array().unwrap().iter().any(|g| g["id"] == dm && g["name"] == "小明"));
        let bro_id = named["id"].as_str().unwrap();
        let (status, task) = read_json(
            call(&app, post(&format!("/api/groups/{dm}/tasks"), json!({ "assigneeBotId": bro_id, "prompt": "你好" }))).await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        assert_eq!(statuses(&events_of(&app, &dm).await)[0]["botName"], "小明");

        let (status, list) = read_json(call(&app, get("/api/bros")).await).await;
        assert_eq!(status, StatusCode::OK);
        let listed: Vec<&str> = list["bros"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
        assert_eq!(listed, ["bro", "bro 2", "bro 3", "小明", "bro 4"]);

        let (status, _) = new_bro(&app, Some(json!({ "name": "   " }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = new_bro(&app, Some(json!({ "name": "x".repeat(65) }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rename_bro_and_its_chat_follows() {
        let app = test_app(&temp_db());
        let (_, bro) = new_bro(&app, None).await;
        let id = bro["id"].as_str().unwrap();
        let patch = |name: &str| {
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/bros/{id}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "name": name }).to_string()))
                .unwrap()
        };
        let (status, renamed) = read_json(call(&app, patch("阿强")).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(renamed["name"], "阿强");
        assert_eq!(renamed["dmGroupId"], bro["dmGroupId"]);
        let (_, groups) = read_json(call(&app, get("/api/groups")).await).await;
        assert_eq!(groups["groups"][0]["name"], "阿强");
        let (_, next) = new_bro(&app, None).await;
        assert_eq!(next["name"], "bro", "the default name is free again");
        let (status, _) = read_json(call(&app, patch("")).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let missing = Request::builder().method("PATCH").uri("/api/bros/nope")
            .header("content-type", "application/json").body(Body::from(r#"{"name":"x"}"#)).unwrap();
        let (status, _) = read_json(call(&app, missing).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn private_chat_keeps_one_bro_and_existing_bots_can_open_one() {
        let app = test_app(&temp_db());
        let (_, bro) = new_bro(&app, None).await;
        let dm = bro["dmGroupId"].as_str().unwrap();
        let (status, body) = add_member(&app, dm, "bot-other", "Other").await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let (status, _) = add_member(&app, dm, bro["id"].as_str().unwrap(), "bro").await;
        assert_eq!(status, StatusCode::OK, "re-adding its own bro is a no-op");

        // A bot that came in through a group gets its DM on demand, once.
        let group = create_group(&app, "大群").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group, "bot-ada", "Ada").await;
        let (status, first) = read_json(call(&app, post_empty("/api/bros/bot-ada/dm")).await).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, again) = read_json(call(&app, post_empty("/api/bros/bot-ada/dm")).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["dmGroupId"], again["dmGroupId"]);
        assert_ne!(first["dmGroupId"], json!(group));
        let (status, _) = read_json(call(&app, post_empty("/api/bros/nope/dm")).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Ordinary groups still take many bots.
        let (status, _) = add_member(&app, &group, "bot-bob", "Bob").await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // ---- unread ----

    async fn unread_of(app: &Router, group_id: &str) -> Value {
        let (status, body) = read_json(call(app, get(&format!("/api/groups/{group_id}/unread"))).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn mark_read(app: &Router, group_id: &str, body: Option<Value>) -> (StatusCode, Value) {
        let uri = format!("/api/groups/{group_id}/read");
        read_json(call(app, match body { Some(b) => post(&uri, b), None => post_empty(&uri) }).await).await
    }

    #[tokio::test]
    async fn unread_counts_what_bros_sent_since_read() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, _) = seed_task(&app, "未读", "robroz-action:captcha\n画一张草图").await;
        let fresh = unread_of(&app, &group_id).await;
        assert_eq!(fresh["readAt"], Value::Null);
        assert_eq!(fresh["unread"], 0, "a queued card and my own messages do not count");
        assert!(fresh["lastActivityAt"].is_string());

        // Stuck task needs you: counts.
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(unread_of(&app, &group_id).await["unread"], 0, "running does not count");
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(unread_of(&app, &group_id).await["unread"], 1);

        // Reading clears it; the response already carries the new count.
        let (status, read) = mark_read(&app, &group_id, None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert_eq!(read["unread"], 0);
        let read_at = read["readAt"].as_str().unwrap().to_string();

        // A choice card and a delivery after that count again.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let (status, _) = post_choice(
            &app,
            &group_id,
            json!({ "botId": "bot-ada", "prompt": "选一个", "options": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B" }] }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = resume(&app, &statuses(&events_of(&app, &group_id).await)[0]["id"].as_str().unwrap().to_string()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unread_of(&app, &group_id).await["unread"], 1, "resuming is not news");
        pump_db(&path, &volumes, &mut fake);
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(unread_of(&app, &group_id).await["unread"], 2, "choice + delivery");

        // Read position only moves forward.
        let (_, back) = mark_read(&app, &group_id, Some(json!({ "readAt": "2000-01-01T00:00:00Z" }))).await;
        assert_eq!(back["readAt"], read_at);
        assert_eq!(back["unread"], 2);
    }

    #[tokio::test]
    async fn unread_list_sorts_by_activity_and_validates() {
        let path = temp_db();
        let app = test_app(&path);
        let quiet = create_group(&app, "安静").await["id"].as_str().unwrap().to_string();
        let older = create_group(&app, "旧").await["id"].as_str().unwrap().to_string();
        let newer = create_group(&app, "新").await["id"].as_str().unwrap().to_string();
        for (g, text) in [(&older, "a"), (&newer, "b")] {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let (status, _) = read_json(call(&app, post(&format!("/api/groups/{g}/messages"), json!({ "kind": "text", "body": text }))).await).await;
            assert_eq!(status, StatusCode::CREATED);
        }
        let (status, body) = read_json(call(&app, get("/api/unread")).await).await;
        assert_eq!(status, StatusCode::OK);
        let order: Vec<&str> = body["groups"].as_array().unwrap().iter().map(|g| g["groupId"].as_str().unwrap()).collect();
        assert_eq!(order, [newer.as_str(), older.as_str(), quiet.as_str()]);
        assert_eq!(body["groups"][2]["lastActivityAt"], Value::Null);

        let (status, _) = read_json(call(&app, get("/api/groups/missing/unread")).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = mark_read(&app, "missing", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = mark_read(&app, &quiet, Some(json!({ "readAt": "soon" }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = mark_read(&app, &quiet, Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn unread_table_migrates_old_db() {
        let path = temp_db();
        { super::Host::open(&path, temp_volumes(), "x").unwrap(); }
        Connection::open(&path).unwrap().execute("DROP TABLE group_reads", []).unwrap();
        super::Host::open(&path, temp_volumes(), "x").unwrap();
        super::Host::open(&path, temp_volumes(), "x").unwrap();
        let n: i64 = Connection::open(&path).unwrap()
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'group_reads'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    // ---- live stream ----

    /// Reads SSE frames until `n` `data:` events arrived or `wait` passed.
    async fn sse_take(body: &mut Body, n: usize, wait: std::time::Duration) -> Vec<(String, String, Value)> {
        let mut out = Vec::new();
        let mut buf = String::new();
        let deadline = tokio::time::Instant::now() + wait;
        while out.len() < n {
            let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, body.frame()).await else { break };
            let Ok(bytes) = frame.into_data() else { continue };
            buf.push_str(std::str::from_utf8(&bytes).unwrap());
            while let Some(end) = buf.find("\n\n") {
                let block: String = buf.drain(..end + 2).collect();
                let (mut name, mut id, mut data) = (String::new(), String::new(), None);
                for line in block.lines() {
                    if let Some(v) = line.strip_prefix("event: ") { name = v.into(); }
                    if let Some(v) = line.strip_prefix("id: ") { id = v.into(); }
                    if let Some(v) = line.strip_prefix("data: ") { data = Some(serde_json::from_str(v).unwrap()); }
                }
                if let Some(data) = data { out.push((name, id, data)); }
            }
        }
        out
    }

    async fn open_stream(app: &Router, group_id: &str, last_id: Option<&str>) -> Response<Body> {
        let mut b = Request::builder().method("GET").uri(format!("/api/groups/{group_id}/stream"));
        if let Some(id) = last_id {
            b = b.header("last-event-id", id);
        }
        call(app, b.body(Body::empty()).unwrap()).await
    }

    async fn say(app: &Router, group_id: &str, text: &str) {
        let (status, _) = read_json(call(app, post(&format!("/api/groups/{group_id}/messages"), json!({ "kind": "text", "body": text }))).await).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    /// Stream contract is the set of events, not their cross-type order.
    /// README「实时推送」promises framing, `Last-Event-ID` / `?since=`, and no
    /// push when nothing changed. It does not promise order. A single snapshot
    /// follows `GET …/events` (`createdAt`, then `id`), so two cards written in
    /// the same millisecond may arrive either way.
    fn assert_events_once(events: &[(String, String, Value)], want: &[&str]) {
        let mut got: Vec<&str> = events.iter().map(|(name, _, _)| name.as_str()).collect();
        let mut expect = want.to_vec();
        got.sort_unstable();
        expect.sort_unstable();
        assert_eq!(got, expect, "complete and not repeated: {events:?}");
    }

    #[tokio::test]
    async fn stream_pushes_new_and_changed_cards_once() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "推送", "画一张草图").await;
        say(&app, &group_id, "第一条").await;

        let resp = open_stream(&app, &group_id, None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()["content-type"], "text/event-stream");
        let mut body = resp.into_body();
        let wait = std::time::Duration::from_secs(3);
        let backlog = sse_take(&mut body, 2, wait).await;
        assert_events_once(&backlog, &["status", "message"]);
        let status = backlog.iter().find(|(name, _, _)| name == "status").unwrap();
        assert_eq!(status.2["state"], "queued");
        assert!(backlog.iter().any(|(name, _, data)| name == "message" && data["body"] == "第一条"));

        // Nothing changed: nothing arrives.
        assert!(sse_take(&mut body, 1, super::stream::POLL * 4).await.is_empty());

        // New message and a milestone on the same card are pushed.
        say(&app, &group_id, "第二条").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        let live = sse_take(&mut body, 2, wait).await;
        assert_eq!(live.len(), 2, "{live:?}");
        assert!(live.iter().any(|(n, _, d)| n == "message" && d["body"] == "第二条"));
        let status = live.iter().find(|(n, _, _)| n == "status").expect("status pushed");
        assert_eq!(status.2["id"], task_id, "same card id, replaced in place");
        assert_eq!(status.2["state"], "running");
        assert!(sse_take(&mut body, 1, super::stream::POLL * 4).await.is_empty(), "no repeats");

        // Reconnect from the last id: only what changed afterwards.
        let last = live.iter().map(|(_, id, _)| id.clone()).max().unwrap();
        drop(body);
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        let mut body = open_stream(&app, &group_id, Some(&last)).await.into_body();
        let resumed = sse_take(&mut body, 3, wait).await;
        assert_events_once(&resumed, &["status", "delivery"]);
        let status = resumed.iter().find(|(name, _, _)| name == "status").unwrap();
        assert_eq!(status.2["id"], task_id);
        assert_eq!(status.2["state"], "done");
    }

    #[tokio::test]
    async fn stream_rejects_missing_group_and_bad_cursor() {
        let path = temp_db();
        let app = test_app(&path);
        let (status, body) = read_json(open_stream(&app, "missing", None).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "群不存在" }));
        let group_id = create_group(&app, "x").await["id"].as_str().unwrap().to_string();
        let (status, _) = read_json(open_stream(&app, &group_id, Some("not-a-time")).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ---- listener token ----

    #[tokio::test]
    async fn token_guards_every_route_when_set() {
        let path = temp_db();
        let token = super::auth::ApiToken::parse("0123456789abcdef-good").unwrap();
        let app = super::Host::open(&path, temp_volumes(), "robroz-runner:local").unwrap().with_token(token).router();
        let req = |auth: Option<&str>| {
            let mut b = Request::builder().method("GET").uri("/api/groups");
            if let Some(a) = auth {
                b = b.header("authorization", a);
            }
            b.body(Body::empty()).unwrap()
        };
        for bad in [None, Some("Bearer wrong-token-0000000"), Some("0123456789abcdef-good"), Some("Basic 0123456789abcdef-good")] {
            let (status, body) = read_json(call(&app, req(bad)).await).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad:?}");
            assert!(!body.to_string().contains("0123456789abcdef-good"));
        }
        let (status, body) = read_json(call(&app, post("/api/groups", json!({ "name": "x" }))).await).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "writes too: {body}");
        let (status, _) = read_json(call(&app, req(Some("Bearer 0123456789abcdef-good"))).await).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn no_token_keeps_local_use_open() {
        let app = test_app(&temp_db());
        let (status, _) = read_json(call(&app, get("/api/groups")).await).await;
        assert_eq!(status, StatusCode::OK);
    }

    // ---- bot manners: ack, milestones, quiet, data-only, no rerun ----

    fn one_status<'a>(events: &'a Value, task_id: &str) -> &'a Value {
        let cards = statuses(events);
        assert_eq!(cards.len(), 1, "one status card per task: {events}");
        assert_eq!(cards[0]["taskId"], task_id);
        cards[0]
    }

    async fn events_since(app: &Router, group_id: &str, since: &str) -> (StatusCode, Value) {
        read_json(call(app, get(&format!("/api/groups/{group_id}/events?since={since}"))).await).await
    }

    /// Receiving a task: the queued card is there right away, before any pump.
    #[tokio::test]
    async fn task_is_acknowledged_with_queued_card_immediately() {
        let path = temp_db();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "确认", "写周报").await;
        let card = one_status(&events_of(&app, &group_id).await, &task_id).clone();
        assert_eq!(card["state"], "queued");
        assert_eq!(card["label"], "排队");
        assert_eq!(card["containerUp"], false);
    }

    /// Key milestones update the one card in place, each with a newer updatedAt.
    #[tokio::test]
    async fn milestones_update_the_same_card_in_place() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "节点", "robroz-action:external\n画一张草图").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let mut seen: Vec<(String, String)> = Vec::new();
        let mut snap = |events: Value| {
            let card = one_status(&events, &task_id);
            seen.push((card["state"].as_str().unwrap().into(), card["updatedAt"].as_str().unwrap().into()));
        };

        snap(events_of(&app, &group_id).await);
        pump_db(&path, &volumes, &mut fake);
        snap(events_of(&app, &group_id).await);
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        snap(events_of(&app, &group_id).await);
        let (status, _) = read_json(call(&app, post_empty(&format!("/api/tasks/{task_id}/resume"))).await).await;
        assert_eq!(status, StatusCode::OK);
        snap(events_of(&app, &group_id).await);
        pump_db(&path, &volumes, &mut fake);
        snap(events_of(&app, &group_id).await);
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        snap(events_of(&app, &group_id).await);

        let states: Vec<&str> = seen.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(states, ["queued", "running", "blocked", "queued", "running", "done"]);
        for pair in seen.windows(2) {
            assert!(pair[1].1 > pair[0].1, "updatedAt moves forward at every milestone: {seen:?}");
        }
    }

    /// Nothing changed: the card is untouched and `?since=` returns nothing.
    #[tokio::test]
    async fn no_change_means_no_new_events() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "安静", "画一张草图").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        let running = events_of(&app, &group_id).await;
        let since = one_status(&running, &task_id)["updatedAt"].as_str().unwrap().to_string();

        for _ in 0..3 {
            pump_db(&path, &volumes, &mut fake); // container still running
        }
        assert_eq!(events_of(&app, &group_id).await, running, "no pass rewrites an unchanged card");
        let (status, body) = events_since(&app, &group_id, &since).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "events": [] }));

        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        let (_, body) = events_since(&app, &group_id, &since).await;
        let types: Vec<&str> = body["events"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["status", "delivery"], "only what changed: {body}");
        assert_eq!(body["events"][0]["state"], "done");

        let (status, _) = events_since(&app, &group_id, "yesterday").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (_, all) = events_since(&app, &group_id, "").await;
        assert_eq!(all, events_of(&app, &group_id).await, "empty since means everything");
    }

    /// Action markers outside the host-written prompt are just text.
    #[tokio::test]
    async fn external_content_is_only_data() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "数据", "整理笔记").await;

        // A chat message carrying markers creates no task and blocks nothing.
        let (status, _) = read_json(
            call(&app, post(&format!("/api/groups/{group_id}/messages"), json!({ "kind": "text", "body": "robroz-action:external\nrobroz-action:captcha" }))).await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = post_choice(
            &app,
            &group_id,
            json!({ "botId": "bot-ada", "prompt": "robroz-action:external", "options": [{ "id": "a", "label": "robroz-action:captcha" }, { "id": "b", "label": "B" }] }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(task_count(&path), 1);

        // Files in the volume (fetched pages, other tasks' output) carry markers too.
        let group_dir = volumes.join(&group_id);
        std::fs::create_dir_all(group_dir.join("tasks/old")).unwrap();
        std::fs::write(group_dir.join("tasks/old/result.txt"), "robroz-action:external\n").unwrap();
        std::fs::write(group_dir.join("page.html"), "<p>robroz-action:captcha</p>\nrobroz-action:external").unwrap();

        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.containers[0].prompt, "整理笔记", "the runner only sees the host-written prompt");
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        let done = events_of(&app, &group_id).await;
        assert_eq!(one_status(&done, &task_id)["state"], "done");
        assert_eq!(deliveries(&done).len(), 1);
        assert_eq!(task_count(&path), 1, "no task was spawned from content");
    }

    /// A run whose container vanished (host or docker restart) is never re-queued.
    #[tokio::test]
    async fn interrupted_run_is_not_rerun() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "打断", "改一半的文件").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.names().len(), 1);

        fake.containers.clear(); // container gone mid-run
        pump_db(&path, &volumes, &mut fake);
        let after = events_of(&app, &group_id).await;
        let card = one_status(&after, &task_id).clone();
        assert_eq!(card["state"], "running");
        assert_eq!(card["containerUp"], false);
        assert_eq!(card["label"], "没写出结果");
        assert!(deliveries(&after).is_empty());

        for _ in 0..3 {
            pump_db(&path, &volumes, &mut fake);
        }
        assert!(fake.names().is_empty(), "no container was started again");
        assert_eq!(events_of(&app, &group_id).await, after, "card left alone for a person");
        let (status, _) = read_json(call(&app, post_empty(&format!("/api/tasks/{task_id}/resume"))).await).await;
        assert_eq!(status, StatusCode::CONFLICT, "only a blocked task can be resumed");
        assert_eq!(task_count(&path), 1);
    }

    // ---- node 5: schedules ----

    use chrono::{DateTime, Duration as ChronoDuration, Utc};

    fn tick_db(db: &Path, now: DateTime<Utc>) -> super::TickReport {
        let conn = Connection::open(db).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        super::tick(&conn, now).unwrap()
    }

    fn at(stamp: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(stamp).unwrap().with_timezone(&Utc)
    }

    fn schedule_row(db: &Path, id: &str) -> (String, Option<String>) {
        let conn = Connection::open(db).unwrap();
        conn.query_row(
            "SELECT next_run_at, last_task_id FROM schedules WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    fn task_count(db: &Path) -> i64 {
        let conn = Connection::open(db).unwrap();
        conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0)).unwrap()
    }

    async fn post_schedule(app: &Router, body: Value) -> (StatusCode, Value) {
        read_json(call(app, post("/api/schedules", body)).await).await
    }

    /// Group with bot-ada as member, plus a schedule every minute.
    async fn seed_schedule(app: &Router, name: &str, prompt: &str) -> (String, Value) {
        let group_id = create_group(app, name).await["id"].as_str().unwrap().to_string();
        let (status, _) = add_member(app, &group_id, "bot-ada", "Ada").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, schedule) = post_schedule(
            app,
            json!({ "groupId": group_id, "assigneeBotId": "bot-ada", "cron": "* * * * *", "prompt": prompt }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{schedule}");
        (group_id, schedule)
    }

    // ---- routines: list and pause ----

    fn patch(uri: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("PATCH")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn put(uri: &str) -> Request<Body> {
        Request::builder().method("PUT").uri(uri).body(Body::empty()).unwrap()
    }

    async fn set_paused(app: &Router, id: &str, paused: bool) -> (StatusCode, Value) {
        read_json(call(app, patch(&format!("/api/schedules/{id}"), json!({ "paused": paused }))).await).await
    }

    #[tokio::test]
    async fn list_schedules_returns_group_schedules_with_paused() {
        let path = temp_db();
        let app = test_app(&path);
        let (group_id, schedule) = seed_schedule(&app, "例行", "早报").await;
        let other = create_group(&app, "别的群").await["id"].as_str().unwrap().to_string();

        let (status, body) = read_json(call(&app, get(&format!("/api/groups/{group_id}/schedules"))).await).await;
        assert_eq!(status, StatusCode::OK);
        let mut expected = schedule.clone();
        expected["paused"] = json!(false);
        assert_eq!(body, json!({ "schedules": [expected] }));

        let (status, body) = read_json(call(&app, get(&format!("/api/groups/{other}/schedules"))).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "schedules": [] }));

        let (status, _) = read_json(call(&app, get("/api/groups/nope/schedules")).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_schedule_shape_is_unchanged_by_paused() {
        let path = temp_db();
        let app = test_app(&path);
        let (_, schedule) = seed_schedule(&app, "例行", "早报").await;
        assert!(schedule.get("paused").is_none(), "POST /api/schedules keeps the frozen shape");
    }

    #[tokio::test]
    async fn paused_schedule_is_skipped_by_tick_and_resume_does_not_backfill() {
        let path = temp_db();
        let app = test_app(&path);
        let (_, schedule) = seed_schedule(&app, "例行", "早报").await;
        let id = schedule["id"].as_str().unwrap();
        let due = at(schedule["nextRunAt"].as_str().unwrap());

        let (status, body) = set_paused(&app, id, true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["paused"], true);
        assert_eq!(body["id"], id);

        let report = tick_db(&path, due + ChronoDuration::hours(2));
        assert!(report.created.is_empty() && report.skipped.is_empty());
        assert_eq!(task_count(&path), 0);

        // Pausing again is a no-op.
        let (status, body) = set_paused(&app, id, true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["paused"], true);

        let (status, body) = set_paused(&app, id, false).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["paused"], false);
        let next = at(body["nextRunAt"].as_str().unwrap());
        assert!(next > Utc::now() - ChronoDuration::seconds(1), "resume picks a slot after now");
        assert_eq!(schedule_row(&path, id).0, body["nextRunAt"].as_str().unwrap());

        let report = tick_db(&path, next);
        assert_eq!(report.created.len(), 1, "resumed schedule fires once at its next slot");
    }

    #[tokio::test]
    async fn patch_schedule_errors() {
        let path = temp_db();
        let app = test_app(&path);
        let (status, _) = set_paused(&app, "nope", true).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (_, schedule) = seed_schedule(&app, "例行", "早报").await;
        let id = schedule["id"].as_str().unwrap();
        let response = call(&app, patch(&format!("/api/schedules/{id}"), json!({ "paused": "yes" }))).await;
        assert!(response.status().is_client_error());
        let response = call(&app, patch(&format!("/api/schedules/{id}"), json!({}))).await;
        assert!(response.status().is_client_error());
    }

    #[test]
    fn migration_adds_paused_at_to_existing_schedules() {
        let path = temp_db();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE schedules (
                    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, group_id TEXT NOT NULL,
                    assignee_bot_id TEXT NOT NULL, cron TEXT NOT NULL, prompt TEXT NOT NULL,
                    next_run_at TEXT NOT NULL, last_task_id TEXT, created_at TEXT NOT NULL);
                 INSERT INTO schedules VALUES ('s1','local','g1','bot-ada','* * * * *','早报',
                    '2026-10-08T09:00:00.000Z',NULL,'2026-10-01T00:00:00.000Z');",
            )
            .unwrap();
        }
        let _ = test_app(&path);
        let _ = test_app(&path); // second open stays idempotent
        let conn = Connection::open(&path).unwrap();
        let paused: Option<String> = conn
            .query_row("SELECT paused_at FROM schedules WHERE id = 's1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(paused, None);
    }

    // ---- approval cards ----

    async fn post_approval(app: &Router, group_id: &str, body: Value) -> (StatusCode, Value) {
        read_json(call(app, post(&format!("/api/groups/{group_id}/approvals"), body)).await).await
    }

    async fn decide(app: &Router, id: &str, verb: &str) -> (StatusCode, Value) {
        read_json(call(app, post_empty(&format!("/api/approvals/{id}/{verb}"))).await).await
    }

    #[tokio::test]
    async fn approval_card_is_answered_once_by_approve_or_deny() {
        let app = test_app(&temp_db());
        let group_id = create_group(&app, "审批").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group_id, "bot-ada", "Ada").await;

        let (status, card) = post_approval(
            &app,
            &group_id,
            json!({ "botId": "bot-ada", "action": "delete", "summary": " 删掉旧的构建缓存 ", "path": "target/" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{card}");
        assert_eq!(card["type"], "approval");
        assert_eq!(card["botName"], "Ada");
        assert_eq!(card["action"], "delete");
        assert_eq!(card["summary"], "删掉旧的构建缓存");
        assert_eq!(card["path"], "target/");
        assert_eq!(card["decision"], Value::Null);
        assert_eq!(card["answeredAt"], Value::Null);
        let id = card["id"].as_str().unwrap().to_string();

        let since = card["createdAt"].as_str().unwrap().to_string();
        let events = events_of(&app, &group_id).await;
        let open: Vec<_> = events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "approval").collect();
        assert_eq!(open, vec![&card]);

        let (status, answered) = decide(&app, &id, "approve").await;
        assert_eq!(status, StatusCode::OK, "{answered}");
        assert_eq!(answered["decision"], "approved");
        assert!(answered["answeredAt"].is_string());
        let (status, changed) = events_since(&app, &group_id, &since).await;
        assert_eq!(status, StatusCode::OK);
        assert!(changed["events"].as_array().unwrap().contains(&answered), "answer shows up as a change: {changed}");

        for verb in ["approve", "deny"] {
            let (status, body) = decide(&app, &id, verb).await;
            assert_eq!(status, StatusCode::CONFLICT, "{verb}");
            assert_eq!(body, json!({ "error": "已经答过了" }));
        }
        let events = events_of(&app, &group_id).await;
        let cards: Vec<_> = events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "approval").collect();
        assert_eq!(cards, vec![&answered], "first answer stands, card replaced in place");

        let (status, other) = post_approval(&app, &group_id, json!({ "botId": "bot-ada", "action": "spend", "summary": "买域名" })).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(other["path"], Value::Null);
        let (status, denied) = decide(&app, other["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(denied["decision"], "denied");
    }

    #[tokio::test]
    async fn approval_card_validates_and_counts_as_unread() {
        let app = test_app(&temp_db());
        let group_id = create_group(&app, "审批").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group_id, "bot-ada", "Ada").await;

        let (status, _) = post_approval(&app, "nope", json!({ "botId": "bot-ada", "action": "send", "summary": "x" })).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = post_approval(&app, &group_id, json!({ "botId": "bot-bob", "action": "send", "summary": "x" })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "这个 Bot 不在群里" }));
        for summary in ["  ", "两行\n不行"] {
            let (status, _) = post_approval(&app, &group_id, json!({ "botId": "bot-ada", "action": "send", "summary": summary })).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{summary:?}");
        }
        let resp = call(&app, post(&format!("/api/groups/{group_id}/approvals"), json!({ "botId": "bot-ada", "action": "nuke", "summary": "x" }))).await;
        assert!(resp.status().is_client_error(), "unknown action is rejected");
        let (status, _) = decide(&app, "nope", "approve").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = post_approval(&app, &group_id, json!({ "botId": "bot-ada", "action": "send", "summary": "发周报" })).await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, unread) = read_json(call(&app, get(&format!("/api/groups/{group_id}/unread"))).await).await;
        assert_eq!(unread["unread"], 1, "{unread}");
    }

    // ---- choice cards ----

    async fn post_choice(app: &Router, group_id: &str, body: Value) -> (StatusCode, Value) {
        read_json(call(app, post(&format!("/api/groups/{group_id}/choices"), body)).await).await
    }

    async fn answer(app: &Router, id: &str, choice: Value) -> (StatusCode, Value) {
        read_json(call(app, post(&format!("/api/choices/{id}/answer"), json!({ "choice": choice }))).await).await
    }

    fn two_options() -> Value {
        json!([{ "id": "a", "label": "方案 A", "primary": true }, { "id": "b", "label": "方案 B", "primary": false }])
    }

    #[tokio::test]
    async fn choice_card_flow_answer_comes_back_as_choice() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "选项").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group_id, "bot-ada", "Ada").await;

        let (status, card) = post_choice(
            &app,
            &group_id,
            json!({ "botId": "bot-ada", "prompt": " 用哪个？ ", "options": two_options() }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{card}");
        assert_eq!(card["type"], "choice");
        assert_eq!(card["botName"], "Ada");
        assert_eq!(card["prompt"], "用哪个？");
        assert_eq!(card["options"], two_options());
        assert_eq!(card["multi"], false);
        assert_eq!(card["choice"], Value::Null);
        assert_eq!(card["answeredAt"], Value::Null);
        let id = card["id"].as_str().unwrap();

        let events = events_of(&app, &group_id).await;
        let open: Vec<_> = events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "choice").collect();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0], &card);

        let (status, _) = answer(&app, id, json!(["a", "b"])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "single choice takes one value");
        let (status, answered) = answer(&app, id, json!(["b"])).await;
        assert_eq!(status, StatusCode::OK, "{answered}");
        assert_eq!(answered["choice"], json!(["b"]));
        assert!(answered["answeredAt"].is_string());

        let events = events_of(&app, &group_id).await;
        let cards: Vec<_> = events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "choice").collect();
        assert_eq!(cards, vec![&answered], "same card, now answered");

        let (status, _) = answer(&app, id, json!(["a"])).await;
        assert_eq!(status, StatusCode::CONFLICT, "answered once only");
    }

    #[tokio::test]
    async fn multi_choice_takes_several_and_drops_primary() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "多选").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group_id, "bot-ada", "Ada").await;
        let options = json!([
            { "id": "a", "label": "A", "primary": true },
            { "id": "b", "label": "B" },
            { "id": "c", "label": "C" }
        ]);
        let (status, card) = post_choice(&app, &group_id, json!({ "botId": "bot-ada", "prompt": "选几个", "options": options, "multi": true })).await;
        assert_eq!(status, StatusCode::CREATED, "{card}");
        assert_eq!(card["multi"], true);
        assert!(card["options"].as_array().unwrap().iter().all(|o| o["primary"] == false));
        let id = card["id"].as_str().unwrap();
        for bad in [json!([]), json!(["a", "a"]), json!(["a", "z"])] {
            let (status, _) = answer(&app, id, bad.clone()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
        let (status, body) = answer(&app, id, json!(["c", "a"])).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["choice"], json!(["c", "a"]));
    }

    #[tokio::test]
    async fn choice_validation() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "选项").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group_id, "bot-ada", "Ada").await;

        let (status, _) = post_choice(&app, "nope", json!({ "botId": "bot-ada", "prompt": "?", "options": two_options() })).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = post_choice(&app, &group_id, json!({ "botId": "bot-zed", "prompt": "?", "options": two_options() })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = post_choice(&app, &group_id, json!({ "botId": "bot-ada", "prompt": "  ", "options": two_options() })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        for options in [
            json!([{ "id": "a", "label": "A" }]),
            json!([{ "id": "a", "label": "A" }, { "id": " a ", "label": "B" }]),
            json!([{ "id": "a", "label": "A" }, { "id": "", "label": "B" }]),
            json!([{ "id": "a", "label": "A" }, { "id": "b", "label": " " }]),
            json!([{ "id": "a", "label": "A", "primary": true }, { "id": "b", "label": "B", "primary": true }]),
            json!((0..5).map(|i| json!({ "id": i.to_string(), "label": "x" })).collect::<Vec<_>>()),
        ] {
            let (status, body) = post_choice(&app, &group_id, json!({ "botId": "bot-ada", "prompt": "?", "options": options })).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{options} -> {body}");
        }
        let conn = Connection::open(&path).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM choices", [], |row| row.get(0)).unwrap();
        assert_eq!(n, 0, "rejected cards are not stored");
        drop(conn);

        let (_, card) = post_choice(&app, &group_id, json!({ "botId": "bot-ada", "prompt": "?", "options": two_options() })).await;
        let id = card["id"].as_str().unwrap();
        let (status, _) = answer(&app, id, json!(["c"])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = answer(&app, "nope", json!(["a"])).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = answer(&app, id, json!(["a"])).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["choice"], json!(["a"]));
    }

    #[tokio::test]
    async fn create_schedule_validates_and_returns_shape() {
        let path = temp_db();
        let app = test_app(&path);
        let group_id = create_group(&app, "定时").await["id"].as_str().unwrap().to_string();
        let (status, _) = add_member(&app, &group_id, "bot-ada", "Ada").await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, body) = post_schedule(
            &app,
            json!({ "groupId": "missing", "assigneeBotId": "bot-ada", "cron": "* * * * *", "prompt": "x" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, json!({ "error": "群不存在" }));

        let (status, body) = post_schedule(
            &app,
            json!({ "groupId": group_id, "assigneeBotId": "bot-zed", "cron": "* * * * *", "prompt": "x" }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "这个 Bot 不在群里" }));

        let (status, body) = post_schedule(
            &app,
            json!({ "groupId": group_id, "assigneeBotId": "bot-ada", "cron": "* * * * *", "prompt": "  " }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({ "error": "请填写内容" }));

        for bad in ["", "nope", "61 * * * *", "* * * *", "0 0 0 * * * 2026", "@daily", "0 0 30 2 *"] {
            let (status, body) = post_schedule(
                &app,
                json!({ "groupId": group_id, "assigneeBotId": "bot-ada", "cron": bad, "prompt": "x" }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "cron {bad:?}");
            assert_eq!(body, json!({ "error": "cron 格式不对" }), "cron {bad:?}");
        }
        let conn = Connection::open(&path).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM schedules", [], |row| row.get(0)).unwrap();
        assert_eq!(n, 0, "rejected schedules are not stored");
        drop(conn);

        let before = Utc::now();
        let (status, schedule) = post_schedule(
            &app,
            json!({ "groupId": group_id, "assigneeBotId": "bot-ada", "cron": "0 9 * * *", "prompt": "早报" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{schedule}");
        let mut keys: Vec<&str> = schedule.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["assigneeBotId", "cron", "groupId", "id", "lastTaskId", "nextRunAt", "prompt"]
        );
        assert_eq!(schedule["groupId"], group_id);
        assert_eq!(schedule["assigneeBotId"], "bot-ada");
        assert_eq!(schedule["cron"], "0 9 * * *");
        assert_eq!(schedule["prompt"], "早报");
        assert_eq!(schedule["lastTaskId"], Value::Null);
        let next = at(schedule["nextRunAt"].as_str().unwrap());
        assert!(next > before);
        assert_eq!(next.format("%H:%M:%S").to_string(), "09:00:00", "cron is UTC");

        let (status, six) = post_schedule(
            &app,
            json!({ "groupId": group_id, "assigneeBotId": "bot-ada", "cron": "*/30 * * * * *", "prompt": "x" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "6-field cron with seconds: {six}");

        let conn = Connection::open(&path).unwrap();
        let tenant: String = conn
            .query_row("SELECT tenant_id FROM schedules WHERE id = ?1", [schedule["id"].as_str().unwrap()], |row| row.get(0))
            .unwrap();
        assert_eq!(tenant, "local");
        assert_eq!(task_count(&path), 0, "creating a schedule does not create a task");
    }

    #[tokio::test]
    async fn tick_before_due_creates_nothing() {
        let path = temp_db();
        let app = test_app(&path);
        let (_, schedule) = seed_schedule(&app, "未到点", "早报").await;
        let id = schedule["id"].as_str().unwrap();
        let next = at(schedule["nextRunAt"].as_str().unwrap());

        let report = tick_db(&path, next - ChronoDuration::seconds(1));
        assert!(report.created.is_empty());
        assert!(report.skipped.is_empty());
        assert_eq!(task_count(&path), 0);
        assert_eq!(schedule_row(&path, id), (schedule["nextRunAt"].as_str().unwrap().to_string(), None));
    }

    #[tokio::test]
    async fn tick_at_due_creates_one_queued_task_and_advances() {
        let path = temp_db();
        let app = test_app(&path);
        let (group_id, schedule) = seed_schedule(&app, "到点", "早报").await;
        let id = schedule["id"].as_str().unwrap();
        let next = at(schedule["nextRunAt"].as_str().unwrap());

        let report = tick_db(&path, next);
        assert_eq!(report.created.len(), 1);
        let task_id = report.created[0].clone();
        assert_eq!(task_count(&path), 1);

        let (next_run_at, last) = schedule_row(&path, id);
        assert_eq!(last.as_deref(), Some(task_id.as_str()));
        assert_eq!(at(&next_run_at), next + ChronoDuration::minutes(1), "strictly after now");

        let events = events_of(&app, &group_id).await;
        let cards = statuses(&events);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["id"], task_id);
        assert_eq!(cards[0]["botId"], "bot-ada");
        assert_eq!(cards[0]["state"], "queued");
        assert_eq!(cards[0]["label"], "排队");
        assert_eq!(cards[0]["containerUp"], false, "tick does not start a container");
        assert!(deliveries(&events).is_empty());

        let conn = Connection::open(&path).unwrap();
        let (briefing, schedule_id): (i64, String) = conn
            .query_row("SELECT briefing, schedule_id FROM tasks WHERE id = ?1", [&task_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(briefing, 1);
        assert_eq!(schedule_id, id);
        drop(conn);

        // Same instant again: no longer due.
        let again = tick_db(&path, next);
        assert!(again.created.is_empty());
        assert_eq!(task_count(&path), 1);
    }

    #[tokio::test]
    async fn tick_does_not_backfill_missed_runs() {
        let path = temp_db();
        let app = test_app(&path);
        let (_, schedule) = seed_schedule(&app, "补跑", "早报").await;
        let id = schedule["id"].as_str().unwrap();
        let next = at(schedule["nextRunAt"].as_str().unwrap());
        let late = next + ChronoDuration::minutes(10) + ChronoDuration::seconds(5);

        let report = tick_db(&path, late);
        assert_eq!(report.created.len(), 1, "one task, not ten");
        let (next_run_at, _) = schedule_row(&path, id);
        assert!(at(&next_run_at) > late);
        assert_eq!(at(&next_run_at), next + ChronoDuration::minutes(11));
    }

    #[tokio::test]
    async fn scheduled_task_delivers_briefing_and_normal_task_does_not() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = "今日简报";
        let (group_id, schedule) = seed_schedule(&app, "简报群", prompt).await;
        let next = at(schedule["nextRunAt"].as_str().unwrap());

        let scheduled = tick_db(&path, next).created[0].clone();
        let (status, normal) = read_json(
            call(
                &app,
                post(
                    &format!("/api/groups/{group_id}/tasks"),
                    json!({ "assigneeBotId": "bot-ada", "prompt": "普通任务" }),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let normal_id = normal["id"].as_str().unwrap().to_string();

        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.containers.len(), 2);
        let running = events_of(&app, &group_id).await;
        for card in statuses(&running) {
            assert_eq!(card["state"], "running");
            assert_eq!(card["label"], "进行");
            assert_eq!(card["containerUp"], true);
        }

        fake.finish(0);
        let group_dir = fake.containers[0].group_dir.clone();
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty(), "container list is empty afterwards");

        let scheduled_file = group_dir.join(format!("tasks/{scheduled}/result.txt"));
        assert!(group_dir.ends_with(&group_id));
        assert_eq!(std::fs::read(&scheduled_file).unwrap(), prompt.as_bytes());

        let done = events_of(&app, &group_id).await;
        let cards = statuses(&done);
        assert_eq!(cards.len(), 2);
        for card in &cards {
            assert_eq!(card["state"], "done");
            assert_eq!(card["label"], "完成");
            assert_eq!(card["containerUp"], false);
        }
        let delivered = deliveries(&done);
        assert_eq!(delivered.len(), 2);
        let by_path = |task: &str| {
            let rel = format!("tasks/{task}/result.txt");
            delivered.iter().find(|d| d["path"] == rel.as_str()).copied().unwrap()
        };
        assert_eq!(by_path(&scheduled)["briefing"], true);
        assert_eq!(by_path(&scheduled)["summary"], "已写入结果");
        assert_eq!(by_path(&normal_id)["briefing"], false);
        assert_eq!(robroz_card::briefing_badge(), "简报");
    }

    #[tokio::test]
    async fn tick_skips_while_previous_run_is_live_but_advances() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (_, schedule) = seed_schedule(&app, "跳过", "早报").await;
        let id = schedule["id"].as_str().unwrap();
        let first_due = at(schedule["nextRunAt"].as_str().unwrap());

        let first = tick_db(&path, first_due).created[0].clone();

        // Still queued: skip.
        let second_due = first_due + ChronoDuration::minutes(1);
        let report = tick_db(&path, second_due);
        assert!(report.created.is_empty());
        assert_eq!(report.skipped, vec![id.to_string()]);
        assert_eq!(task_count(&path), 1);
        let (next_run_at, last) = schedule_row(&path, id);
        assert_eq!(at(&next_run_at), second_due + ChronoDuration::minutes(1));
        assert_eq!(last.as_deref(), Some(first.as_str()));

        // Running with a live container: skip.
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.names().len(), 1);
        let third_due = at(&next_run_at);
        let report = tick_db(&path, third_due);
        assert!(report.created.is_empty(), "no second task while containerUp is true");
        assert_eq!(task_count(&path), 1);
        let (next_run_at, last) = schedule_row(&path, id);
        assert!(at(&next_run_at) > third_due);
        assert_eq!(at(&next_run_at), third_due + ChronoDuration::minutes(1));
        assert_eq!(last.as_deref(), Some(first.as_str()));
        assert_eq!(fake.names().len(), 1, "tick never starts containers");

        // After it finishes, the next occurrence runs again.
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        let report = tick_db(&path, at(&next_run_at));
        assert_eq!(report.created.len(), 1);
        assert_eq!(task_count(&path), 2);
    }

    #[tokio::test]
    async fn dead_scheduled_run_does_not_hold_the_schedule() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, schedule) = seed_schedule(&app, "死跑", "早报").await;
        let due = at(schedule["nextRunAt"].as_str().unwrap());
        tick_db(&path, due);
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish_without_output(0);
        pump_db(&path, &volumes, &mut fake);
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "running");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert_eq!(statuses(&events)[0]["label"], robroz_card::status_line(robroz_contract::TaskState::Running, false));
        assert!(deliveries(&events).is_empty());

        let report = tick_db(&path, due + ChronoDuration::minutes(1));
        assert_eq!(report.created.len(), 1, "containerUp false is not a live run");
    }

    #[tokio::test]
    async fn blocked_scheduled_task_resumes_and_keeps_briefing() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, schedule) = seed_schedule(&app, "卡住简报", "robroz-action:external\n简报正文").await;
        let due = at(schedule["nextRunAt"].as_str().unwrap());
        let task_id = tick_db(&path, due).created[0].clone();

        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        fake.finish(0);
        let group_dir = fake.containers[0].group_dir.clone();
        pump_db(&path, &volumes, &mut fake);
        let blocked = events_of(&app, &group_id).await;
        assert_eq!(statuses(&blocked)[0]["state"], "blocked");
        assert_eq!(statuses(&blocked)[0]["label"], "卡住");
        assert!(deliveries(&blocked).is_empty());

        let (status, body) = read_json(call(&app, post_empty(&format!("/api/tasks/{task_id}/resume"))).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "id": task_id, "state": "queued" }));
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(fake.containers[0].group_dir, group_dir);
        fake.finish(0);
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());

        let done = events_of(&app, &group_id).await;
        assert_eq!(statuses(&done)[0]["state"], "done");
        let delivered = deliveries(&done);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0]["briefing"], true);
        assert_eq!(delivered[0]["path"], format!("tasks/{task_id}/result.txt"));
        assert!(group_dir.join(format!("tasks/{task_id}/result.txt")).is_file());
    }

    #[tokio::test]
    async fn list_groups_empty() {
        let path = temp_db();
        let app = test_app(&path);
        let (status, body) = read_json(call(&app, get("/api/groups")).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "groups": [] }));
    }

    #[tokio::test]
    async fn list_groups_in_creation_order() {
        let path = temp_db();
        let app = test_app(&path);
        let first = create_group(&app, "设计群").await;
        let second = create_group(&app, "运营群").await;
        let (status, body) = read_json(call(&app, get("/api/groups")).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({
                "groups": [
                    { "id": first["id"], "name": "设计群", "tenantId": "local" },
                    { "id": second["id"], "name": "运营群", "tenantId": "local" }
                ]
            })
        );
    }

    // ---- sandbox user / prompt file / takeover marker ----

    async fn resume(app: &Router, task_id: &str) -> (StatusCode, Value) {
        read_json(call(app, post_empty(&format!("/api/tasks/{task_id}/resume"))).await).await
    }

    fn takeover_file(volumes: &Path, task_id: &str) -> PathBuf {
        control_of(volumes).join(task_id).join("takeover")
    }

    /// Pump, let the fake container run the real runner, pump again.
    fn run_once(db: &Path, volumes: &Path, fake: &mut super::sandbox::FakeSandbox) -> PathBuf {
        pump_db(db, volumes, fake);
        let group_dir = fake.containers.last().expect("container started").group_dir.clone();
        fake.finish(0);
        pump_db(db, volumes, fake);
        assert!(fake.names().is_empty());
        group_dir
    }

    fn stored_intent(db: &Path, task_id: &str) -> (Option<String>, Option<String>, Option<String>) {
        let conn = Connection::open(db).unwrap();
        conn.query_row(
            "SELECT intent_kind, intent_line, intent_hash FROM tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn pump_writes_prompt_into_control_dir_not_volume() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (_, task_id) = seed_task(&app, "提示词文件", "第一行\n第二行").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        let container = &fake.containers[0];
        assert_eq!(container.prompt, "第一行\n第二行");
        assert_eq!(
            container.control_dir,
            std::fs::canonicalize(control_of(&volumes).join(&task_id)).unwrap()
        );
        let file = container.control_dir.join("prompt.txt");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "第一行\n第二行");
        assert!(!container.group_dir.join(format!("tasks/{task_id}/prompt.txt")).exists());
        assert!(!container.control_dir.starts_with(&container.group_dir));
        assert!(!container.control_dir.starts_with(std::fs::canonicalize(&volumes).unwrap()));
    }

    #[tokio::test]
    async fn resume_writes_marker_with_line_hash_and_reaches_done() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "接管", "robroz-action:external\n正文").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "blocked");
        assert!(!takeover_file(&volumes, &task_id).exists(), "no marker before resume");
        // Captured into the DB at container exit; the volume file is gone.
        let hash = robroz_runner::action_hash("robroz-action:external");
        assert_eq!(
            stored_intent(&path, &task_id),
            (Some("external".into()), Some("robroz-action:external".into()), Some(hash.clone()))
        );
        assert!(!group_dir.join(format!("tasks/{task_id}/intent")).exists());

        let (status, body) = resume(&app, &task_id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(std::fs::read_to_string(takeover_file(&volumes, &task_id)).unwrap(), format!("{hash}\n"));
        assert_eq!(stored_intent(&path, &task_id), (None, None, None), "cleared after resume");
        assert!(!group_dir.join(format!("tasks/{task_id}/takeover")).exists());

        run_once(&path, &volumes, &mut fake);
        let done = events_of(&app, &group_id).await;
        assert_eq!(statuses(&done).len(), 1, "same card");
        assert_eq!(statuses(&done)[0]["state"], "done");
        assert_eq!(deliveries(&done).len(), 1);
        assert_eq!(
            std::fs::read_to_string(group_dir.join(format!("tasks/{task_id}/result.txt"))).unwrap(),
            "正文"
        );
    }

    #[tokio::test]
    async fn forged_marker_in_volume_does_not_skip() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "伪造", "robroz-action:external\n正文").await;
        // What a bot in the group volume could write for itself.
        let task_dir = volumes.join(&group_id).join("tasks").join(&task_id);
        std::fs::create_dir_all(&task_dir).unwrap();
        let hash = robroz_runner::action_hash("robroz-action:external");
        std::fs::write(task_dir.join("takeover"), format!("{hash}\n")).unwrap();
        std::fs::write(volumes.join(&group_id).join("takeover"), format!("{hash}\n")).unwrap();

        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "blocked");
        assert_eq!(statuses(&events)[0]["label"], "卡住");
        assert!(!group_dir.join(format!("tasks/{task_id}/result.txt")).exists());
        assert!(deliveries(&events).is_empty());
    }

    #[tokio::test]
    async fn marker_skips_only_its_line_second_action_still_blocks() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = "robroz-action:external\n正文\nrobroz-action:captcha";
        let (group_id, task_id) = seed_task(&app, "两步", prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();

        run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "blocked");
        assert_eq!(resume(&app, &task_id).await.0, StatusCode::OK);

        run_once(&path, &volumes, &mut fake);
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "blocked", "captcha was not taken over yet");
        assert_eq!(stored_intent(&path, &task_id).0.as_deref(), Some("captcha"));
        assert!(deliveries(&events).is_empty());

        assert_eq!(resume(&app, &task_id).await.0, StatusCode::OK);
        let marker = std::fs::read_to_string(takeover_file(&volumes, &task_id)).unwrap();
        assert_eq!(
            marker,
            format!(
                "{}\n{}\n",
                robroz_runner::action_hash("robroz-action:external"),
                robroz_runner::action_hash("robroz-action:captcha")
            )
        );
        run_once(&path, &volumes, &mut fake);
        let done = events_of(&app, &group_id).await;
        assert_eq!(statuses(&done).len(), 1);
        assert_eq!(statuses(&done)[0]["state"], "done");
        assert_eq!(deliveries(&done).len(), 1);
    }

    #[tokio::test]
    async fn volume_intent_edited_after_exit_does_not_change_approval() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = "robroz-action:external\n正文\nrobroz-action:captcha";
        let (group_id, task_id) = seed_task(&app, "改意图", prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(stored_intent(&path, &task_id).0.as_deref(), Some("external"));

        // After exit, something in the group rewrites intent to the other real
        // blocking line, hoping resume approves that one instead.
        let intent = group_dir.join(format!("tasks/{task_id}/intent"));
        std::fs::write(&intent, robroz_runner::format_intent("captcha", "robroz-action:captcha")).unwrap();

        assert_eq!(resume(&app, &task_id).await.0, StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(takeover_file(&volumes, &task_id)).unwrap(),
            format!("{}\n", robroz_runner::action_hash("robroz-action:external")),
            "resume approves only the line captured at exit"
        );

        // The forged file is cleared before the next start; captcha still blocks.
        run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "blocked");
        assert_eq!(stored_intent(&path, &task_id).0.as_deref(), Some("captcha"));
    }

    #[tokio::test]
    async fn resume_without_stored_intent_is_500_and_stays_blocked() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "没记录", "robroz-action:external\n正文").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "blocked");
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE tasks SET intent_kind = NULL, intent_line = NULL, intent_hash = NULL WHERE id = ?1",
                [&task_id],
            )
            .unwrap();
        // A valid-looking intent in the volume is not used as a fallback.
        std::fs::write(
            group_dir.join(format!("tasks/{task_id}/intent")),
            robroz_runner::format_intent("external", "robroz-action:external"),
        )
        .unwrap();
        // Snapshot after the setup wipe: resume must not change the card.
        let blocked = statuses(&events_of(&app, &group_id).await)[0].clone();
        assert_eq!(blocked["state"], "blocked");

        let (status, body) = resume(&app, &task_id).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, json!({ "error": "恢复失败，找不到卡住记录" }));
        let after = statuses(&events_of(&app, &group_id).await)[0].clone();
        assert_eq!(after, blocked, "state and updatedAt unchanged");
        assert!(!takeover_file(&volumes, &task_id).exists(), "no marker written");
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty(), "nothing queued");
    }

    #[tokio::test]
    async fn intent_without_blocking_line_is_dead_run_not_blocked() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "乱写意图", "正文").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        let group_dir = fake.containers[0].group_dir.clone();
        // The container claims it is blocked on a line its prompt never had.
        fake.finish_without_output(0);
        std::fs::create_dir_all(group_dir.join(format!("tasks/{task_id}"))).unwrap();
        std::fs::write(
            group_dir.join(format!("tasks/{task_id}/intent")),
            robroz_runner::format_intent("captcha", "robroz-action:captcha"),
        )
        .unwrap();
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty());
        let events = events_of(&app, &group_id).await;
        let card = statuses(&events)[0];
        assert_eq!(card["state"], "running");
        assert_eq!(card["containerUp"], false);
        assert_eq!(card["label"], "没写出结果");
        assert!(deliveries(&events).is_empty());
        assert_eq!(stored_intent(&path, &task_id), (None, None, None));
        assert!(!group_dir.join(format!("tasks/{task_id}/intent")).exists());
        assert_eq!(resume(&app, &task_id).await.0, StatusCode::CONFLICT);
        assert!(!takeover_file(&volumes, &task_id).exists());
    }

    #[tokio::test]
    async fn intent_after_every_line_taken_over_is_dead_run() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "全接管", "robroz-action:external\n正文").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        assert_eq!(resume(&app, &task_id).await.0, StatusCode::OK);

        // Next run: the only blocking line is approved, yet the container still
        // drops an intent and writes no result.
        pump_db(&path, &volumes, &mut fake);
        let group_dir = fake.containers[0].group_dir.clone();
        fake.finish_without_output(0);
        std::fs::write(group_dir.join(format!("tasks/{task_id}/intent")), "external").unwrap();
        pump_db(&path, &volumes, &mut fake);
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "running");
        assert_eq!(statuses(&events)[0]["containerUp"], false);
        assert_eq!(statuses(&events)[0]["label"], "没写出结果");
        assert_eq!(stored_intent(&path, &task_id), (None, None, None));
    }

    #[tokio::test]
    async fn forged_intent_content_at_exit_has_no_effect() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = "robroz-action:external\n正文\nrobroz-action:captcha";
        let (group_id, task_id) = seed_task(&app, "伪造内容", prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_db(&path, &volumes, &mut fake);
        let group_dir = fake.containers[0].group_dir.clone();
        fake.finish(0);
        // Before the host looks, the bot rewrites intent to the other real
        // blocking line, so that line would be the one approved.
        let intent = group_dir.join(format!("tasks/{task_id}/intent"));
        std::fs::write(&intent, robroz_runner::format_intent("captcha", "robroz-action:captcha")).unwrap();
        pump_db(&path, &volumes, &mut fake);

        let external = robroz_runner::action_hash("robroz-action:external");
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "blocked");
        assert_eq!(
            stored_intent(&path, &task_id),
            (Some("external".into()), Some("robroz-action:external".into()), Some(external.clone())),
            "host computes the first unapproved line; intent content is ignored"
        );
        assert_eq!(resume(&app, &task_id).await.0, StatusCode::OK);
        assert_eq!(std::fs::read_to_string(takeover_file(&volumes, &task_id)).unwrap(), format!("{external}\n"));

        // Still needs a second takeover for captcha.
        run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "blocked");
        assert_eq!(stored_intent(&path, &task_id).0.as_deref(), Some("captcha"));
        assert_eq!(resume(&app, &task_id).await.0, StatusCode::OK);
        run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "done");
    }

    #[tokio::test]
    async fn leftover_intent_is_cleared_before_start() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "旧意图", "正文").await;
        let task_dir = volumes.join(&group_id).join("tasks").join(&task_id);
        std::fs::create_dir_all(&task_dir).unwrap();
        std::fs::write(task_dir.join("intent"), "external").unwrap();
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "done");
    }

    #[tokio::test]
    async fn migration_adds_intent_columns_to_existing_db() {
        let path = temp_db();
        {
            // Tasks table as shipped before this change (node 5 / PR #7).
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE groups (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE bots (id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, name TEXT NOT NULL);
                 CREATE TABLE memberships (group_id TEXT NOT NULL, bot_id TEXT NOT NULL, PRIMARY KEY (group_id, bot_id));
                 CREATE TABLE tasks (
                     id TEXT PRIMARY KEY, group_id TEXT NOT NULL, assignee_bot_id TEXT NOT NULL,
                     prompt TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     container_up INTEGER NOT NULL DEFAULT 0, get_count INTEGER NOT NULL DEFAULT 0,
                     briefing INTEGER NOT NULL DEFAULT 0, schedule_id TEXT);
                 INSERT INTO groups VALUES ('g1', 'local', '旧群', '2026-10-01T00:00:00.000Z');
                 INSERT INTO bots VALUES ('bot-ada', 'local', 'Ada');
                 INSERT INTO memberships VALUES ('g1', 'bot-ada');
                 INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at)
                 VALUES ('t-old', 'g1', 'bot-ada', 'robroz-action:external', 'blocked',
                         '2026-10-01T00:00:01.000Z', '2026-10-01T00:00:02.000Z');",
            )
            .unwrap();
        }
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        // Opening twice must not fail on the existing columns.
        let app = {
            drop(app);
            app_on(&path, &volumes)
        };

        let conn = Connection::open(&path).unwrap();
        let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('tasks')").unwrap();
        let columns: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
        for column in ["intent_kind", "intent_line", "intent_hash"] {
            assert!(columns.iter().any(|c| c == column), "missing {column}: {columns:?}");
        }
        assert_eq!(stored_intent(&path, "t-old"), (None, None, None));

        let events = events_of(&app, "g1").await;
        assert_eq!(statuses(&events)[0]["id"], "t-old");
        assert_eq!(statuses(&events)[0]["state"], "blocked");
        // Blocked before the host stored intents: no approval to give.
        let (status, body) = resume(&app, "t-old").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, json!({ "error": "恢复失败，找不到卡住记录" }));
        assert_eq!(statuses(&events_of(&app, "g1").await)[0]["state"], "blocked");
    }


    /// Wraps the fake and records whether the DB lock was free during each docker call.
    struct LockProbe {
        db: std::sync::Arc<std::sync::Mutex<Connection>>,
        inner: super::sandbox::FakeSandbox,
        calls: usize,
        locked: usize,
    }

    impl LockProbe {
        fn check(&mut self) {
            self.calls += 1;
            if self.db.try_lock().is_err() {
                self.locked += 1;
            }
        }
    }

    impl super::Sandbox for LockProbe {
        fn start(&mut self, req: &super::sandbox::RunRequest<'_>) -> Result<(), super::sandbox::SandboxError> {
            self.check();
            self.inner.start(req)
        }
        fn phase(&mut self, name: &str) -> Result<super::sandbox::ContainerPhase, super::sandbox::SandboxError> {
            self.check();
            self.inner.phase(name)
        }
        fn remove(&mut self, name: &str) -> Result<(), super::sandbox::SandboxError> {
            self.check();
            self.inner.remove(name)
        }
    }

    #[tokio::test]
    async fn pump_once_does_not_hold_db_lock_during_docker_calls() {
        let path = temp_db();
        let volumes = temp_volumes();
        let host = super::Host::open(&path, &volumes, "robroz-runner:local")
            .unwrap()
            .with_control_root(control_of(&volumes));
        let app = host.router();
        let (group_id, _) = seed_task(&app, "锁", "画一张草图").await;
        let mut probe = LockProbe {
            db: host.db.clone(),
            inner: super::sandbox::FakeSandbox::new(),
            calls: 0,
            locked: 0,
        };
        host.pump_once(&mut probe).unwrap();
        probe.inner.finish(0);
        host.pump_once(&mut probe).unwrap();
        assert!(probe.calls >= 3, "start, inspect, remove");
        assert_eq!(probe.locked, 0, "DB lock held during a docker call");
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "done");
    }

    async fn say_in(app: &Router, group_id: &str, body: &str) -> (StatusCode, Value) {
        read_json(call(app, post(&format!("/api/groups/{group_id}/say"), json!({ "body": body }))).await).await
    }

    #[tokio::test]
    async fn say_routes_mentions_and_dms_to_one_task() {
        let app = test_app(&temp_db());
        let group = create_group(&app, "研发").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group, "b1", "bro").await;
        add_member(&app, &group, "b2", "bro 2").await;

        let (status, plain) = say_in(&app, &group, "  大家好 ").await;
        assert_eq!(status, StatusCode::CREATED, "{plain}");
        assert_eq!(plain["message"]["body"], "大家好");
        assert_eq!(plain["message"]["kind"], "text");
        assert!(plain["task"].is_null());

        let (status, to_two) = say_in(&app, &group, "@bro 2 写 README").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(to_two["message"]["body"], "@bro 2 写 README");
        assert_eq!(to_two["task"]["assigneeBotId"], "b2");
        assert_eq!(to_two["task"]["prompt"], "写 README");
        assert_eq!(to_two["task"]["state"], "queued");

        // Not a member, or no prompt: stored only.
        assert!(say_in(&app, &group, "@小明 你好").await.1["task"].is_null());
        assert!(say_in(&app, &group, "@bro").await.1["task"].is_null());

        let events = events_of(&app, &group).await;
        assert_eq!(events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "message").count(), 4);
        let queued = statuses(&events);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0]["botId"], "b2");

        // In a DM everything goes to its bro, no @ needed.
        let (_, bro) = new_bro(&app, Some(json!({ "name": "小明" }))).await;
        let dm = bro["dmGroupId"].as_str().unwrap();
        let (status, said) = say_in(&app, dm, "整理周报").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(said["task"]["assigneeBotId"], bro["id"]);
        assert_eq!(said["task"]["prompt"], "整理周报");
    }

    #[tokio::test]
    async fn say_rejects_empty_and_unknown_group() {
        let app = test_app(&temp_db());
        let group = create_group(&app, "研发").await["id"].as_str().unwrap().to_string();
        let (status, body) = say_in(&app, &group, "   ").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "请填写内容");
        let (status, body) = say_in(&app, "nope", "你好").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "群不存在");
        assert_eq!(events_of(&app, &group).await["events"].as_array().unwrap().len(), 0);
    }

    /// Fake model: answers or fails, and records each call's prompt and memory paragraph.
    struct FakeModel {
        answer: Result<String, String>,
        calls: std::sync::atomic::AtomicUsize,
        seen: std::sync::Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeModel {
        fn new(answer: Result<String, String>) -> Self {
            Self { answer, calls: Default::default(), seen: Default::default() }
        }

        fn memory_sections(&self) -> Vec<Option<String>> {
            self.seen.lock().unwrap().iter().map(|(_, section)| section.clone()).collect()
        }
    }

    impl robroz_model::Model for FakeModel {
        fn reply(&self, prompt: &str, memory_section: Option<&str>) -> Result<String, robroz_model::ModelError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.seen.lock().unwrap().push((prompt.to_string(), memory_section.map(str::to_string)));
            self.answer
                .clone()
                .map(|a| format!("{a}（{prompt}）"))
                .map_err(robroz_model::ModelError)
        }
    }

    fn pump_with_model(db: &Path, volumes: &Path, sandbox: &mut dyn super::Sandbox, model: &FakeModel) {
        let conn = Connection::open(db).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        let control = control_of(volumes);
        super::pump_with(
            &conn,
            sandbox,
            &super::PumpConfig {
                volumes_root: volumes,
                control_root: &control,
                image: "robroz-runner:local",
                get_limit: 8,
                model: Some(model),
                echo_prompt: false,
                tools: crate::tools::ToolRegistry::empty(),
                mcp: None,
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn model_reply_becomes_the_result_once_per_start() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "模型", "写周报").await;
        let model = FakeModel::new(Ok("周报写好了".into()));

        let mut fake = super::sandbox::FakeSandbox::new();
        pump_with_model(&path, &volumes, &mut fake, &model);
        assert_eq!(fake.containers[0].prompt, "写周报", "prompt.txt unchanged");
        let reply = control_of(&volumes).join(&task_id).join("reply.txt");
        assert!(!reply.starts_with(&volumes), "reply lives in the control dir, not the volume");
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &model);
        assert_eq!(model.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "running tasks are not asked again");

        let result = volumes.join(&group_id).join(format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read_to_string(result).unwrap(), "周报写好了（写周报）");
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["state"], "done");
    }

    async fn post_memory(app: &Router, scope: &str, kind: &str, text: &str) {
        let (status, body) = read_json(
            call(app, post("/api/memories", json!({ "scope": scope, "kind": kind, "text": text }))).await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    #[tokio::test]
    async fn kept_memories_are_injected_into_the_system_prompt() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, _) = seed_task(&app, "记忆注入", "先跑一次").await;
        let (status, _) = add_member(&app, &group_id, "bot-bob", "Bob").await;
        assert_eq!(status, StatusCode::CREATED);

        // No memories yet: the system prompt stays the opening only.
        let empty = FakeModel::new(Ok("好".into()));
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_with_model(&path, &volumes, &mut fake, &empty);
        assert_eq!(empty.memory_sections(), [None]);
        assert_eq!(fake.containers[0].prompt, "先跑一次", "prompt.txt is still just the task");
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &empty);
        assert_eq!(empty.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "done tasks are not asked again");

        post_memory(&app, "user", "habit", "共享习惯").await;
        post_memory(&app, "bot:bot-ada", "profile", "Ada 专用旧").await;
        post_memory(&app, "bot:bot-bob", "habit", "Bob 的，Ada 不该看到").await;
        post_memory(&app, "bot:bot-ada", "project", "Ada 专用新").await;
        post_memory(&app, "user", "habit", "忘掉的").await;
        let forgotten = memories_with(&app, "").await.into_iter().find(|m| m["text"] == "忘掉的").unwrap();
        let resp = call(&app, delete_req(&format!("/api/memories/{}", forgotten["id"].as_str().unwrap()))).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        for (id, scope, status, text) in [
            ("pending-row", "user", "pending", "还没点头"),
            ("rejected-row", "bot:bot-ada", "rejected", "被拒绝的"),
        ] {
            conn.execute(
                "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at, forgotten_at)
                 VALUES (?1, ?2, 'habit', ?3, 'user', ?4, '2026-01-01T00:00:09.000Z', '2026-01-01T00:00:09.000Z', NULL)",
                rusqlite::params![id, scope, status, text],
            )
            .unwrap();
        }

        let (_, next) = seed_task_in(&app, &group_id, "再写周报").await;
        let model = FakeModel::new(Ok("周报写好了".into()));
        pump_with_model(&path, &volumes, &mut fake, &model);
        let section = model.memory_sections()[0].clone().expect("kept memories go into the system prompt");
        assert_eq!(
            section,
            "你记得这些：\n[project] Ada 专用新\n[profile] Ada 专用旧\n[habit] 共享习惯"
        );
        assert!(!section.contains("Bob") && !section.contains("还没点头") && !section.contains("被拒绝的") && !section.contains("忘掉的"));
        let started = fake.containers.iter().find(|c| c.task_id == next).expect("second task started");
        assert_eq!(started.prompt, "再写周报", "user prompt unchanged");
    }

    fn status_of<'a>(events: &'a Value, task_id: &str) -> &'a Value {
        statuses(events).into_iter().find(|s| s["taskId"] == task_id).expect("status card")
    }

    #[tokio::test]
    async fn running_status_carries_progress_and_only_changes_send_a_new_one() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "进度", "写周报").await;
        let model = FakeModel::new(Ok("周报写好了".into()));
        let mut fake = super::sandbox::FakeSandbox::new();

        let events = events_of(&app, &group_id).await;
        assert!(status_of(&events, &task_id).get("progress").is_none(), "queued has no progress");

        // Started: claimed, model answered, sandbox up.
        pump_with_model(&path, &volumes, &mut fake, &model);
        let events = events_of(&app, &group_id).await;
        let card = status_of(&events, &task_id).clone();
        assert_eq!(card["state"], "running");
        assert_eq!(card["progress"], "在沙箱里跑");
        let at = card["updatedAt"].as_str().unwrap().to_string();

        // The runner reports a step: same card, new text, newer updatedAt.
        let progress_file = volumes.join(&group_id).join(format!("tasks/{task_id}/progress"));
        std::fs::create_dir_all(progress_file.parent().unwrap()).unwrap();
        std::fs::write(&progress_file, "在查资料").unwrap();
        pump_with_model(&path, &volumes, &mut fake, &model);
        let since = format!("/api/groups/{group_id}/events?since={at}");
        let (_, fresh) = read_json(call(&app, get(&since)).await).await;
        let changed = statuses(&fresh);
        assert_eq!(changed.len(), 1, "one new status for the new text: {fresh}");
        assert_eq!(changed[0]["taskId"], task_id.as_str());
        assert_eq!(changed[0]["progress"], "在查资料");
        let at = changed[0]["updatedAt"].as_str().unwrap().to_string();

        // Same text again: nothing new to send.
        pump_with_model(&path, &volumes, &mut fake, &model);
        pump_with_model(&path, &volumes, &mut fake, &model);
        let since = format!("/api/groups/{group_id}/events?since={at}");
        let (_, fresh) = read_json(call(&app, get(&since)).await).await;
        assert!(statuses(&fresh).is_empty(), "same progress, no new status: {fresh}");

        // Another step: a new status again.
        std::fs::write(&progress_file, "在排版").unwrap();
        pump_with_model(&path, &volumes, &mut fake, &model);
        let (_, fresh) = read_json(call(&app, get(&since)).await).await;
        assert_eq!(statuses(&fresh).len(), 1);
        assert_eq!(statuses(&fresh)[0]["progress"], "在排版");

        // Done: the card no longer carries progress, and the row is cleared.
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &model);
        let events = events_of(&app, &group_id).await;
        let card = status_of(&events, &task_id);
        assert_eq!(card["state"], "done");
        assert!(card.get("progress").is_none(), "{card}");
        let stored: Option<String> = Connection::open(&path)
            .unwrap()
            .query_row("SELECT progress FROM tasks WHERE id = ?1", [&task_id], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn model_progress_line_goes_on_the_card_not_into_the_result() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "进度", "写周报").await;
        let model = FakeModel::new(Ok("robroz-progress:在整理要点\n周报写好了".into()));
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_with_model(&path, &volumes, &mut fake, &model);
        assert_eq!(status_of(&events_of(&app, &group_id).await, &task_id)["progress"], "在整理要点");
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &model);
        let result = volumes.join(&group_id).join(format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read_to_string(result).unwrap(), "周报写好了（写周报）");
        let events = events_of(&app, &group_id).await;
        assert!(status_of(&events, &task_id).get("progress").is_none());
        assert_eq!(deliveries(&events).len(), 1);
    }

    #[tokio::test]
    async fn blocked_and_dead_runs_carry_no_progress() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, blocked) = seed_task(&app, "进度", "登录\nrobroz-action:captcha").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let events = events_of(&app, &group_id).await;
        assert_eq!(status_of(&events, &blocked)["state"], "blocked");
        assert!(status_of(&events, &blocked).get("progress").is_none());

        let (_, dead) = seed_task_in(&app, &group_id, "写周报").await;
        pump_db(&path, &volumes, &mut fake);
        assert_eq!(status_of(&events_of(&app, &group_id).await, &dead)["progress"], "在沙箱里跑");
        fake.finish_without_output(1);
        pump_db(&path, &volumes, &mut fake);
        let events = events_of(&app, &group_id).await;
        let card = status_of(&events, &dead);
        assert_eq!(card["state"], "running");
        assert_eq!(card["containerUp"], false);
        assert!(card.get("progress").is_none(), "dead run: {card}");
    }

    async fn seed_task_in(app: &Router, group_id: &str, prompt: &str) -> (StatusCode, String) {
        let url = format!("/api/groups/{group_id}/tasks");
        let (status, task) = read_json(call(app, post(&url, json!({ "assigneeBotId": "bot-ada", "prompt": prompt }))).await).await;
        (status, task["id"].as_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn model_failure_is_written_as_the_result() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "模型", "写周报").await;
        let model = FakeModel::new(Err("模型服务返回 401".into()));
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_with_model(&path, &volumes, &mut fake, &model);
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &model);
        let result = volumes.join(&group_id).join(format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read_to_string(result).unwrap(), "模型没回：模型服务返回 401");
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["state"], "done");
        assert_eq!(deliveries(&events)[0]["failure"], "模型没回：模型服务返回 401", "card can say it was not done");
        assert_eq!(statuses(&events)[0]["failure"], "模型没回：模型服务返回 401", "status card too");

        // The next task with a working model gets a normal delivery: no failure key at all.
        let ok = FakeModel::new(Ok("周报写好了".into()));
        let (_, next) = seed_task_in(&app, &group_id, "再写一次").await;
        pump_with_model(&path, &volumes, &mut fake, &ok);
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &ok);
        let events = events_of(&app, &group_id).await;
        let delivered = deliveries(&events);
        assert_eq!(delivered.len(), 2, "{next}");
        assert!(delivered.iter().filter(|d| d.get("failure").is_some()).count() == 1);
        assert!(statuses(&events).iter().filter(|t| t.get("failure").is_some()).count() == 1);
    }

    #[tokio::test]
    async fn answered_by_is_stored_when_sent_and_null_otherwise() {
        let app = test_app(&temp_db());
        let group_id = create_group(&app, "谁答的").await["id"].as_str().unwrap().to_string();
        add_member(&app, &group_id, "bot-ada", "Ada").await;
        let card = |summary: &str| json!({ "botId": "bot-ada", "action": "send", "summary": summary });

        // Old request (no body): answeredBy null.
        let (_, a1) = post_approval(&app, &group_id, card("发周报")).await;
        assert_eq!(a1["answeredBy"], Value::Null);
        let (status, done) = decide(&app, a1["id"].as_str().unwrap(), "approve").await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["answeredBy"], Value::Null);

        // With a body: trimmed and stored; the 409 for a second device keeps the first answer.
        let (_, a2) = post_approval(&app, &group_id, card("发邮件")).await;
        let id = a2["id"].as_str().unwrap();
        let url = format!("/api/approvals/{id}/deny");
        let (status, done) = read_json(call(&app, post(&url, json!({ "answeredBy": " Panda 的手机 " }))).await).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["answeredBy"], "Panda 的手机");
        let approve = format!("/api/approvals/{id}/approve");
        let (status, _) = read_json(call(&app, post(&approve, json!({ "answeredBy": "Mac" }))).await).await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Bad answeredBy: 400, card stays open.
        let (_, a3) = post_approval(&app, &group_id, card("付款")).await;
        let url = format!("/api/approvals/{}/approve", a3["id"].as_str().unwrap());
        let (status, body) = read_json(call(&app, post(&url, json!({ "answeredBy": "a\nb" }))).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, _) = read_json(call(&app, post(&url, json!({ "answeredBy": 3 }))).await).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = decide(&app, a3["id"].as_str().unwrap(), "approve").await;
        assert_eq!(status, StatusCode::OK, "still open after bad requests");

        // Choice cards take it next to `choice`.
        let (_, c) = post_choice(&app, &group_id, json!({ "botId": "bot-ada", "prompt": "哪个？", "options": two_options() })).await;
        let url = format!("/api/choices/{}/answer", c["id"].as_str().unwrap());
        let (status, picked) = read_json(call(&app, post(&url, json!({ "choice": ["a"], "answeredBy": "Mac" }))).await).await;
        assert_eq!(status, StatusCode::OK, "{picked}");
        assert_eq!(picked["answeredBy"], "Mac");

        let events = events_of(&app, &group_id).await;
        let by: Vec<_> = events["events"].as_array().unwrap().iter().map(|e| e["answeredBy"].clone()).collect();
        assert!(by.contains(&json!("Panda 的手机")) && by.contains(&json!("Mac")));
    }

    #[test]
    fn answered_by_column_is_added_to_old_databases() {
        let path = temp_db();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE approvals (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, bot_id TEXT NOT NULL, action TEXT NOT NULL,
               summary TEXT NOT NULL, path TEXT, decision TEXT, created_at TEXT NOT NULL, answered_at TEXT);
             CREATE TABLE choices (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, bot_id TEXT NOT NULL, prompt TEXT NOT NULL,
               options TEXT NOT NULL, multi INTEGER NOT NULL DEFAULT 0, choice TEXT, created_at TEXT NOT NULL, answered_at TEXT);",
        )
        .unwrap();
        drop(conn);
        let _app = test_app(&path);
        let _again = test_app(&path);
        let conn = Connection::open(&path).unwrap();
        for table in ["approvals", "choices"] {
            conn.query_row(&format!("SELECT answered_by FROM {table} LIMIT 1"), [], |_| Ok(())).or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(()),
                e => Err(e),
            }).unwrap();
        }
    }

    fn approvals_in(events: &Value) -> Vec<Value> {
        events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "approval").cloned().collect()
    }

    fn task_state(db: &Path, task_id: &str) -> String {
        Connection::open(db).unwrap().query_row("SELECT state FROM tasks WHERE id = ?1", [task_id], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn external_write_raises_an_approval_card_and_approve_resumes() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "审批闸", "给客户发周报\nrobroz-action:external").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &task_id), "blocked");

        let cards = approvals_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1, "one card per blocking line");
        assert_eq!(cards[0]["action"], "send");
        assert_eq!(cards[0]["summary"], "要对外写入：给客户发周报");
        assert_eq!(cards[0]["decision"], Value::Null);
        let assignee = statuses(&events_of(&app, &group_id).await)[0]["botId"].clone();
        assert_eq!(cards[0]["botId"], assignee);

        let (status, answered) = decide(&app, cards[0]["id"].as_str().unwrap(), "approve").await;
        assert_eq!(status, StatusCode::OK, "{answered}");
        assert_eq!(task_state(&path, &task_id), "queued");
        let hash = robroz_policy::action_hash("robroz-action:external");
        assert!(std::fs::read_to_string(takeover_file(&volumes, &task_id)).unwrap().contains(&hash));

        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &task_id), "done");
        assert!(statuses(&events_of(&app, &group_id).await)[0].get("failure").is_none(), "approved run is a normal done");
        assert_eq!(std::fs::read_to_string(group_dir.join(format!("tasks/{task_id}/result.txt"))).unwrap(), "给客户发周报");
        assert_eq!(approvals_in(&events_of(&app, &group_id).await).len(), 1, "no second card after approval");
        let (status, _) = decide(&app, cards[0]["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn deny_ends_the_task_without_a_result() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "审批闸", "删远端分支\nrobroz-action:external").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let card = approvals_in(&events_of(&app, &group_id).await)[0].clone();
        let (status, _) = decide(&app, card["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(task_state(&path, &task_id), "done");
        assert_eq!(statuses(&events_of(&app, &group_id).await)[0]["failure"], "你拒绝了：删远端分支");
        assert!(!takeover_file(&volumes, &task_id).exists(), "deny writes no marker");
        pump_db(&path, &volumes, &mut fake);
        assert!(fake.names().is_empty(), "denied task never starts again");
        assert!(deliveries(&events_of(&app, &group_id).await).is_empty());
        let (status, _) = resume(&app, &task_id).await;
        assert_eq!(status, StatusCode::CONFLICT, "nothing left to resume");
    }

    #[tokio::test]
    async fn blocked_status_says_whether_it_is_waiting_for_approval_or_takeover() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, external) = seed_task(&app, "等什么", "发周报\nrobroz-action:external").await;
        let mut fake = super::sandbox::FakeSandbox::new();

        let events = events_of(&app, &group_id).await;
        assert!(statuses(&events)[0].get("waitingFor").is_none(), "queued has no waitingFor");

        run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &external), "blocked");
        let events = events_of(&app, &group_id).await;
        assert_eq!(statuses(&events)[0]["waitingFor"], "approval");

        let (_, captcha) = seed_task_in(&app, &group_id, "登录\nrobroz-action:captcha").await;
        run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &captcha), "blocked");
        let events = events_of(&app, &group_id).await;
        let st = statuses(&events);
        let of = |id: &str| st.iter().find(|s| s["taskId"] == id).copied().unwrap();
        assert_eq!(of(&external)["waitingFor"], "approval");
        assert_eq!(of(&captcha)["waitingFor"], "takeover");

        let cards = approvals_in(&events);
        let (status, _) = decide(&app, cards[0]["id"].as_str().unwrap(), "approve").await;
        assert_eq!(status, StatusCode::OK);
        run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &external), "done");
        let events = events_of(&app, &group_id).await;
        let st = statuses(&events);
        let of = |id: &str| st.iter().find(|s| s["taskId"] == id).copied().unwrap();
        assert!(of(&external).get("waitingFor").is_none(), "done has no waitingFor");
        assert_eq!(of(&captcha)["waitingFor"], "takeover");
    }

    #[tokio::test]
    async fn captcha_keeps_takeover_and_resume_closes_an_open_card() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, captcha) = seed_task(&app, "审批闸", "登录\nrobroz-action:captcha").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &captcha), "blocked");
        assert!(approvals_in(&events_of(&app, &group_id).await).is_empty(), "captcha needs a person, not a card");

        let (_, external) = seed_task_in(&app, &group_id, "发帖\nrobroz-action:external").await;
        run_once(&path, &volumes, &mut fake);
        let (status, _) = resume(&app, &external).await;
        assert_eq!(status, StatusCode::OK);
        let cards = approvals_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["decision"], "approved", "taking over answers the card");
        let (status, _) = decide(&app, cards[0]["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(task_state(&path, &external), "queued");
    }

    // ---- bro proposals and confirm cards ----

    fn confirms_in(events: &Value) -> Vec<Value> {
        events["events"].as_array().unwrap().iter().filter(|e| e["type"] == "confirm").cloned().collect()
    }

    fn memory_line(kind: &str, text: &str) -> String {
        format!("robroz-memory:{}", json!({ "kind": kind, "text": text }))
    }

    async fn memories_with(app: &Router, query: &str) -> Vec<Value> {
        let (status, body) = read_json(call(app, get(&format!("/api/memories{query}"))).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["memories"].as_array().unwrap().clone()
    }

    fn delete_req(uri: &str) -> Request<Body> {
        Request::builder().method("DELETE").uri(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn proposal_raises_a_confirm_card_and_approve_keeps_it() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = format!("{}\n写周报", memory_line("habit", "周报用中文"));
        let (group_id, task_id) = seed_task(&app, "记忆", &prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &task_id), "done");
        assert_eq!(std::fs::read_to_string(group_dir.join(format!("tasks/{task_id}/result.txt"))).unwrap(), "写周报");
        assert!(!group_dir.join(format!("tasks/{task_id}/proposals.jsonl")).exists(), "read once, then removed");

        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1, "{cards:?}");
        let card = &cards[0];
        assert_eq!(card["subject"], "memory");
        assert_eq!(card["kind"], "habit");
        assert_eq!(card["text"], "周报用中文");
        assert_eq!(card["scope"], "bot:bot-ada", "defaults to the bro that ran it");
        assert_eq!(card["taskId"], task_id.as_str());
        assert_eq!(card["botId"], "bot-ada");
        assert_eq!(card["botName"], "Ada");
        assert_eq!(card["decision"], Value::Null);
        assert_eq!(card["answeredAt"], Value::Null);
        assert!(card.get("slug").is_none() && card.get("title").is_none());
        assert!(approvals_in(&events_of(&app, &group_id).await).is_empty(), "not an approval card");
        let card_id = card["id"].as_str().unwrap().to_string();

        let pending = memories_with(&app, "?status=pending").await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["source"], "bot");
        assert_eq!(pending[0]["id"], card["memoryId"]);
        assert_eq!(pending[0]["confirmId"], card_id.as_str(), "the memory page answers through this card");
        let memory_id = pending[0]["id"].as_str().unwrap().to_string();

        // Editing while pending shows on the card, and the card is sent again.
        let before = card["createdAt"].as_str().unwrap().to_string();
        let resp = call(&app, patch(&format!("/api/memories/{memory_id}"), json!({ "text": "周报一律用中文" }))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let since = format!("/api/groups/{group_id}/events?since={before}");
        let (_, fresh) = read_json(call(&app, get(&since)).await).await;
        assert_eq!(confirms_in(&fresh)[0]["text"], "周报一律用中文");

        let (status, answered) =
            read_json(call(&app, post(&format!("/api/approvals/{card_id}/approve"), json!({ "answeredBy": "Mac" }))).await).await;
        assert_eq!(status, StatusCode::OK, "{answered}");
        assert_eq!(answered["type"], "confirm");
        assert_eq!(answered["decision"], "approved");
        assert_eq!(answered["answeredBy"], "Mac");
        let kept = memories_with(&app, "?status=kept").await;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["text"], "周报一律用中文");
        assert!(kept[0].get("confirmId").is_none(), "answered: nothing left to confirm");

        for verb in ["approve", "deny"] {
            let (status, _) = decide(&app, &card_id, verb).await;
            assert_eq!(status, StatusCode::CONFLICT, "only one answer");
        }
        assert_eq!(memories_with(&app, "?status=kept").await.len(), 1, "still kept");
        assert_eq!(confirms_in(&events_of(&app, &group_id).await)[0]["decision"], "approved");
    }

    #[tokio::test]
    async fn deny_rejects_and_a_repeat_raises_no_new_card() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = format!("{}\n{}\n写周报", memory_line("habit", "周报用中文"), memory_line("project", "robroz 用 Rust"));
        let (group_id, _) = seed_task(&app, "记忆", &prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 2);
        let card = |text: &str| cards.iter().find(|c| c["text"] == text).unwrap()["id"].as_str().unwrap().to_string();
        let (status, denied) = decide(&app, &card("周报用中文"), "deny").await;
        assert_eq!(status, StatusCode::OK, "{denied}");
        assert_eq!(denied["decision"], "denied");
        let (status, _) = decide(&app, &card("robroz 用 Rust"), "approve").await;
        assert_eq!(status, StatusCode::OK);

        let rejected = memories_with(&app, "?status=rejected").await;
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0]["text"], "周报用中文");
        assert!(memories_with(&app, "").await.iter().all(|m| m["text"] != "周报用中文"), "rejected is not in use");

        // Same scope, kind and text (spaces aside), rejected or kept: no new card.
        let again = format!("{}\n{}\n又写周报", memory_line("habit", "  周报用中文 "), memory_line("project", "robroz  用 Rust"));
        let (_, second) = seed_task_in(&app, &group_id, &again).await;
        run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &second), "done");
        assert_eq!(confirms_in(&events_of(&app, &group_id).await).len(), 2, "no new card");
        // Another kind is another memory.
        let other = format!("{}\n再写", memory_line("profile", "周报用中文"));
        seed_task_in(&app, &group_id, &other).await;
        run_once(&path, &volumes, &mut fake);
        assert_eq!(confirms_in(&events_of(&app, &group_id).await).len(), 3);
    }

    #[tokio::test]
    async fn bad_proposal_lines_are_skipped_and_one_task_is_capped() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let lines = [
            "robroz-memory:不是 json".to_string(),
            r#"robroz-memory:{"kind":"mood","text":"x"}"#.to_string(),
            r#"robroz-memory:{"kind":"habit","text":"   "}"#.to_string(),
            r#"robroz-memory:{"kind":"habit"}"#.to_string(),
            r#"robroz-memory:{"kind":"habit","text":"x","scope":"bot:nobody"}"#.to_string(),
            r#"robroz-memory:{"kind":"habit","text":"x","scope":"group:g1"}"#.to_string(),
            format!(r#"robroz-memory:{{"kind":"habit","text":"{}"}}"#, "长".repeat(2001)),
            r#"robroz-memory:{"kind":"profile","text":"叫 Panda","scope":"user"}"#.to_string(),
            "正文".to_string(),
        ];
        let (group_id, task_id) = seed_task(&app, "记忆", &lines.join("\n")).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &task_id), "done", "bad lines never fail the task");
        assert_eq!(std::fs::read_to_string(group_dir.join(format!("tasks/{task_id}/result.txt"))).unwrap(), "正文");
        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1, "{cards:?}");
        assert_eq!(cards[0]["scope"], "user");

        let many: Vec<String> = (0..25).map(|i| memory_line("project", &format!("第 {i} 条"))).collect();
        seed_task_in(&app, &group_id, &format!("{}\n正文", many.join("\n"))).await;
        run_once(&path, &volumes, &mut fake);
        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1 + robroz_contract::PROPOSALS_PER_TASK_MAX);
        assert!(cards.iter().any(|c| c["text"] == "第 19 条") && !cards.iter().any(|c| c["text"] == "第 20 条"), "first 20 in order");
    }

    #[tokio::test]
    async fn model_reply_memory_line_raises_a_card_and_stays_out_of_the_result() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, task_id) = seed_task(&app, "记忆", "写周报").await;
        let model = FakeModel::new(Ok(format!("{}\n周报写好了", memory_line("habit", "周五交周报"))));
        let mut fake = super::sandbox::FakeSandbox::new();
        pump_with_model(&path, &volumes, &mut fake, &model);
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &model);
        let result = volumes.join(&group_id).join(format!("tasks/{task_id}/result.txt"));
        assert_eq!(std::fs::read_to_string(result).unwrap(), "周报写好了（写周报）");
        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["text"], "周五交周报");
        assert_eq!(cards[0]["taskId"], task_id.as_str());

        // A run that did not get done proposes nothing.
        let failing = FakeModel::new(Err("模型服务返回 500".into()));
        seed_task_in(&app, &group_id, &format!("{}\n再写", memory_line("habit", "失败也想记"))).await;
        pump_with_model(&path, &volumes, &mut fake, &failing);
        fake.finish(0);
        pump_with_model(&path, &volumes, &mut fake, &failing);
        assert_eq!(confirms_in(&events_of(&app, &group_id).await).len(), 1, "failed run: no card");
    }

    #[tokio::test]
    async fn forgetting_a_pending_memory_closes_its_card_and_patch_cannot_settle() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = format!("{}\n{}\n写", memory_line("habit", "A"), memory_line("habit", "B"));
        let (group_id, _) = seed_task(&app, "记忆", &prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let pending = memories_with(&app, "?status=pending").await;
        assert_eq!(pending.len(), 2);
        let a = pending.iter().find(|m| m["text"] == "A").unwrap().clone();
        let b = pending.iter().find(|m| m["text"] == "B").unwrap().clone();

        // PATCH has no status: the body is "nothing to change", status stays pending.
        let resp = call(&app, patch(&format!("/api/memories/{}", b["id"].as_str().unwrap()), json!({ "status": "kept" }))).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = call(&app, patch(&format!("/api/memories/{}", b["id"].as_str().unwrap()), json!({ "status": "kept", "text": "B2" }))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let b_now = memories_with(&app, "?status=pending").await.into_iter().find(|m| m["id"] == b["id"]).unwrap();
        assert_eq!((b_now["status"].clone(), b_now["text"].clone()), (json!("pending"), json!("B2")));
        assert_eq!(b_now["confirmId"], b["confirmId"]);

        let resp = call(&app, delete_req(&format!("/api/memories/{}", a["id"].as_str().unwrap()))).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let cards = confirms_in(&events_of(&app, &group_id).await);
        let closed = cards.iter().find(|c| c["memoryId"] == a["id"]).unwrap();
        assert_eq!(closed["decision"], "denied", "forgotten: closed as denied");
        assert_eq!(closed["answeredBy"], Value::Null);
        let (status, _) = decide(&app, a["confirmId"].as_str().unwrap(), "approve").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(memories_with(&app, "?status=kept").await.is_empty(), "a forgotten one is never kept");
        let open = cards.iter().find(|c| c["memoryId"] == b["id"]).unwrap();
        assert_eq!(open["decision"], Value::Null, "the other card is untouched");

        let (status, _) = decide(&app, "no-such-card", "approve").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn approval_cards_carry_the_task_id_when_a_task_raised_them() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, task_id) = seed_task(&app, "审批闸", "发周报\nrobroz-action:external").await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let (status, posted) = post_approval(&app, &group_id, json!({ "botId": "bot-ada", "action": "send", "summary": "发邮件" })).await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(posted.get("taskId").is_none(), "a bot's own card has no task");
        let cards = approvals_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 2);
        let gated = cards.iter().find(|c| c["id"] != posted["id"]).unwrap();
        assert_eq!(gated["taskId"], task_id.as_str());
        let (_, answered) = decide(&app, gated["id"].as_str().unwrap(), "approve").await;
        assert_eq!(answered["taskId"], task_id.as_str());
    }

    #[tokio::test]
    async fn stream_pushes_confirm_cards_and_their_answer() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let (group_id, _) = seed_task(&app, "记忆", &format!("{}\n写", memory_line("habit", "周报用中文"))).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);

        let wait = std::time::Duration::from_secs(3);
        let mut body = open_stream(&app, &group_id, None).await.into_body();
        let backlog = sse_take(&mut body, 3, wait).await;
        assert_events_once(&backlog, &["status", "delivery", "confirm"]);
        let (_, _, card) = backlog.iter().find(|(name, _, _)| name == "confirm").unwrap();
        assert_eq!(card["text"], "周报用中文");
        assert!(card["taskId"].is_string());

        let (status, _) = decide(&app, card["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::OK);
        let pushed = sse_take(&mut body, 1, wait).await;
        assert_events_once(&pushed, &["confirm"]);
        assert_eq!(pushed[0].2["decision"], "denied");
    }

    fn skill_line(slug: &str, title: &str, instruction: &str) -> String {
        format!("robroz-skill:{}", json!({ "slug": slug, "title": title, "instruction": instruction }))
    }

    async fn skills_with(app: &Router, query: &str) -> Vec<Value> {
        let (status, body) = read_json(call(app, get(&format!("/api/skills{query}"))).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["skills"].as_array().unwrap().clone()
    }

    async fn post_skill(app: &Router, scope: &str, slug: &str, title: &str, instruction: &str) -> Value {
        let (status, body) = read_json(
            call(app, post("/api/skills", json!({ "scope": scope, "slug": slug, "title": title, "instruction": instruction }))).await,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    #[tokio::test]
    async fn skill_proposal_raises_a_card_and_approve_keeps_it() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = format!("{}\n写周报", skill_line("weekly-report", "写周报", "先列三点"));
        let (group_id, task_id) = seed_task(&app, "技能", &prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        let group_dir = run_once(&path, &volumes, &mut fake);
        assert_eq!(task_state(&path, &task_id), "done");
        assert_eq!(std::fs::read_to_string(group_dir.join(format!("tasks/{task_id}/result.txt"))).unwrap(), "写周报");
        assert!(!group_dir.join(format!("tasks/{task_id}/proposals.jsonl")).exists());

        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 1, "{cards:?}");
        let card = &cards[0];
        assert_eq!(card["subject"], "skill");
        assert_eq!(card["slug"], "weekly-report");
        assert_eq!(card["title"], "写周报");
        assert_eq!(card["instruction"], "先列三点");
        assert_eq!(card["scope"], "bot:bot-ada");
        assert_eq!(card["taskId"], task_id.as_str());
        assert!(card.get("memoryId").is_none());
        let pending = skills_with(&app, "?status=pending").await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["confirmId"], card["id"]);

        let (status, answered) = decide(&app, card["id"].as_str().unwrap(), "approve").await;
        assert_eq!(status, StatusCode::OK, "{answered}");
        assert_eq!(answered["decision"], "approved");
        assert_eq!(skills_with(&app, "?status=kept").await[0]["slug"], "weekly-report");
        let (status, _) = decide(&app, card["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Same slug is not proposed again, even after keep.
        seed_task_in(&app, &group_id, &format!("{}\n再写", skill_line("weekly-report", "写周报", "先列三点"))).await;
        run_once(&path, &volumes, &mut fake);
        assert_eq!(confirms_in(&events_of(&app, &group_id).await).len(), 1, "duplicate slug dropped");
    }

    #[tokio::test]
    async fn skill_deny_and_forgetting_pending_close_the_card() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let prompt = format!("{}\n{}\n写", skill_line("keep-me", "留下", "留下说明"), skill_line("drop-me", "丢掉", "丢掉说明"));
        let (group_id, _) = seed_task(&app, "技能", &prompt).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), 2);
        let drop = cards.iter().find(|c| c["slug"] == "drop-me").unwrap();
                let (status, denied) = decide(&app, drop["id"].as_str().unwrap(), "deny").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(denied["decision"], "denied");
        assert_eq!(skills_with(&app, "?status=rejected").await[0]["slug"], "drop-me");

        let resp = call(&app, delete_req("/api/skills/keep-me")).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let closed = confirms_in(&events_of(&app, &group_id).await).into_iter().find(|c| c["slug"] == "keep-me").unwrap();
        assert_eq!(closed["decision"], "denied");
        assert!(closed["answeredBy"].is_null());
        assert!(skills_with(&app, "").await.iter().all(|s| s["slug"] != "keep-me"));
    }

    #[tokio::test]
    async fn skill_and_memory_proposals_share_the_per_task_cap() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = app_on(&path, &volumes);
        let mut lines: Vec<String> = (0..12).map(|i| memory_line("habit", &format!("记 {i}"))).collect();
        lines.extend((0..12).map(|i| skill_line(&format!("s-{i}"), &format!("技 {i}"), "说明")));
        lines.push("正文".into());
        let (group_id, _) = seed_task(&app, "技能", &lines.join("\n")).await;
        let mut fake = super::sandbox::FakeSandbox::new();
        run_once(&path, &volumes, &mut fake);
        let cards = confirms_in(&events_of(&app, &group_id).await);
        assert_eq!(cards.len(), robroz_contract::PROPOSALS_PER_TASK_MAX);
        assert!(cards.iter().any(|c| c["subject"] == "memory") && cards.iter().any(|c| c["subject"] == "skill"));
        assert!(!cards.iter().any(|c| c["slug"] == "s-8"), "memories first, then skills, cap 20");
    }

    #[tokio::test]
    async fn kept_enabled_skills_are_injected_into_the_system_prompt() {
        let path = temp_db();
        let volumes = temp_volumes();
        let app = test_app(&path);
        let (group_id, _) = seed_task(&app, "技能注入", "先跑一次").await;
        let (status, _) = add_member(&app, &group_id, "bot-bob", "Bob").await;
        assert_eq!(status, StatusCode::CREATED);
        post_skill(&app, "user", "weekly-report", "写周报", "先列三点").await;
        post_skill(&app, "bot:bot-ada", "ada-notes", "Ada 笔记", "只给 Ada").await;
        post_skill(&app, "bot:bot-bob", "bob-notes", "Bob 笔记", "Ada 不该看到").await;
        post_skill(&app, "user", "unused", "未启用", "不该出现").await;

        let resp = call(&app, put(&format!("/api/groups/{group_id}/skills/weekly-report"))).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let resp = call(&app, put(&format!("/api/groups/{group_id}/skills/ada-notes"))).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let resp = call(&app, put(&format!("/api/groups/{group_id}/skills/bob-notes"))).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        // unused stays disabled

        let mut fake = super::sandbox::FakeSandbox::new();
        let (_, next) = seed_task_in(&app, &group_id, "再写周报").await;
        let model = FakeModel::new(Ok("周报写好了".into()));
        pump_with_model(&path, &volumes, &mut fake, &model);
        // Two queued tasks: "先跑一次" (older) and "再写周报". Pump starts both if
        // both are queued. Check the call for 再写周报.
        let seen = model.seen.lock().unwrap().clone();
        let section = seen
            .iter()
            .find(|(prompt, _)| prompt == "再写周报")
            .map(|(_, s)| s.clone())
            .or_else(|| seen.last().map(|(_, s)| s.clone()))
            .flatten()
            .expect("kept enabled skills go into the system prompt");
        assert!(section.contains("你会这些技能："), "{section}");
        assert!(section.contains("### 写周报 (weekly-report)\n先列三点"), "{section}");
        assert!(section.contains("### Ada 笔记 (ada-notes)\n只给 Ada"), "{section}");
        assert!(!section.contains("Bob") && !section.contains("未启用") && !section.contains("unused"), "{section}");
        let started = fake.containers.iter().find(|c| c.task_id == next).expect("second task started");
        assert_eq!(started.prompt, "再写周报");
    }

    #[test]
    fn confirm_columns_are_added_once() {
        let path = temp_db();
        let _app = test_app(&path);
        let _again = test_app(&path);
        let conn = Connection::open(&path).unwrap();
        conn.prepare("SELECT subject_kind, subject_id FROM approvals").unwrap();
        conn.prepare("SELECT slug, title, instruction FROM skills").unwrap();
    }
}
