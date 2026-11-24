//! Memories: what bros remember about the person, their habits and projects.
//!
//! - `GET    /api/memories?kind=&scope=&status=`  not forgotten; no `status` means
//!   `kept` + `pending` (`rejected` only when asked for). Newest `updatedAt` first.
//! - `POST   /api/memories`       `{scope, kind, text}` written by the person: `source=user`, `status=kept`
//! - `PATCH  /api/memories/{id}`  `{text?, kind?, scope?}`; forgotten or unknown → 404
//! - `DELETE /api/memories/{id}`  forget: soft delete (`forgotten_at`). Forgetting again is 204;
//!   an id that never existed is 404.
//!
//! `scope` is `user` (every bro) or `bot:<botId>` (one existing bro); parsing lives in
//! [`MemoryScope::parse`].
//!
//! A bro's proposals arrive as `pending` (`source=bot`) with a `confirm` card
//! (see [`crate::proposals`]); `confirmId` names that card while it is open.
//! Only answering the card moves a memory out of `pending`: `PATCH` never
//! changes `status`. Forgetting a pending memory closes its card as denied.
//!
//! When a task starts and a model is configured, [`kept_for`] plus
//! [`prompt_section`] append a short paragraph to the system prompt.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{
    CreateMemory, Memories, Memory, MemoryKind, MemoryScope, MemorySource, MemoryStatus,
    UpdateMemory, MEMORY_TEXT_MAX,
};
use rusqlite::{params, Connection, OptionalExtension, ToSql};

use crate::{db_error, error_response, lock_db, new_id, now_rfc3339, AppState, TENANT_ID};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS memories (
        id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,
        kind TEXT NOT NULL,
        text TEXT NOT NULL,
        source TEXT NOT NULL,
        status TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        forgotten_at TEXT
    );
    CREATE INDEX IF NOT EXISTS memories_scope ON memories(scope);
";

/// The last column is the open confirm card of a pending memory, if any.
const SELECT: &str = "SELECT id, scope, kind, text, source, status, created_at, updated_at,
            (SELECT a.id FROM approvals a
             WHERE a.subject_kind = 'memory' AND a.subject_id = memories.id AND a.decision IS NULL
             ORDER BY a.created_at LIMIT 1)
     FROM memories";
/// Newest change first; rowid breaks ties within the same millisecond.
const ORDER: &str = "ORDER BY updated_at DESC, rowid DESC";

const BAD_SCOPE: &str = robroz_contract::host_text::BAD_SCOPE;
const BAD_KIND: &str = robroz_contract::host_text::BAD_MEMORY_KIND;
const BAD_STATUS: &str = robroz_contract::host_text::BAD_STATUS;
const UNKNOWN_BOT: &str = robroz_contract::host_text::UNKNOWN_BRO;
const NOT_FOUND: &str = robroz_contract::host_text::MEMORY_NOT_FOUND;

fn bad_column(column: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("unknown value {text:?}").into(),
    )
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let kind: String = row.get(2)?;
    let source: String = row.get(4)?;
    let status: String = row.get(5)?;
    Ok(Memory {
        id: row.get(0)?,
        scope: row.get(1)?,
        kind: MemoryKind::parse(&kind).ok_or_else(|| bad_column(2, &kind))?,
        text: row.get(3)?,
        source: MemorySource::parse(&source).ok_or_else(|| bad_column(4, &source))?,
        status: MemoryStatus::parse(&status).ok_or_else(|| bad_column(5, &status))?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        confirm_id: if status == MemoryStatus::Pending.as_str() {
            row.get(8)?
        } else {
            None
        },
    })
}

/// A live (not forgotten) memory.
fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Memory>> {
    conn.query_row(
        &format!("{SELECT} WHERE id = ?1 AND forgotten_at IS NULL"),
        [id],
        from_row,
    )
    .optional()
}

/// Kept, not forgotten memories a bro may use: everyone's (`user`) plus its own
/// (`bot:<id>`), newest `updatedAt` first. For building the model prompt.
pub(crate) fn kept_for(conn: &Connection, bot_id: &str) -> rusqlite::Result<Vec<Memory>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE scope IN ('user', ?1) AND status = 'kept' AND forgotten_at IS NULL {ORDER}"
    ))?;
    let own = MemoryScope::for_bot(bot_id).to_string();
    let rows = stmt.query_map([own], from_row)?;
    rows.collect()
}

/// Max length of the memory paragraph in the system prompt, in Unicode scalar
/// values. Over this, oldest whole memories are dropped; a line is never cut.
pub(crate) const MEMORY_PROMPT_MAX: usize = 4000;

const MEMORY_PROMPT_HEADER: &str = "You remember:";

/// One line per memory. Newlines in the text are flattened so the paragraph
/// stays one memory per line.
fn prompt_line(memory: &Memory) -> String {
    let text = memory.text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("[{}] {}", memory.kind.as_str(), text)
}

/// The paragraph to append to the system prompt, or `None` when there is
/// nothing to add. `memories` must already be newest-first, as [`kept_for`]
/// returns them. Entries that would go past `max_chars` are dropped from the
/// oldest end.
pub(crate) fn prompt_section(memories: &[Memory]) -> Option<String> {
    prompt_section_with(memories, MEMORY_PROMPT_MAX)
}

pub(crate) fn prompt_section_with(memories: &[Memory], max_chars: usize) -> Option<String> {
    if memories.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    let mut total = MEMORY_PROMPT_HEADER.chars().count();
    for memory in memories {
        let line = prompt_line(memory);
        let add = 1 + line.chars().count();
        if total + add > max_chars {
            break;
        }
        lines.push(line);
        total += add;
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!("{MEMORY_PROMPT_HEADER}\n{}", lines.join("\n")))
}

pub(crate) fn clean_text(raw: &str) -> Result<String, &'static str> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(robroz_contract::host_text::NEED_TEXT);
    }
    if text.chars().count() > MEMORY_TEXT_MAX {
        return Err(robroz_contract::host_text::MEMORY_TOO_LONG);
    }
    Ok(text.to_string())
}

/// Whitespace runs as one space, for telling two texts apart.
fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a memory with this scope, kind and (normalized) text exists and is
/// not forgotten, whatever its status. Proposals that match are not stored again.
pub(crate) fn has_live_duplicate(
    conn: &Connection,
    scope: &str,
    kind: MemoryKind,
    text: &str,
) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT text FROM memories WHERE scope = ?1 AND kind = ?2 AND forgotten_at IS NULL",
    )?;
    let wanted = normalized(text);
    let mut rows = stmt.query(params![scope, kind.as_str()])?;
    while let Some(row) = rows.next()? {
        if normalized(&row.get::<_, String>(0)?) == wanted {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Parse a scope and make sure a `bot:` scope names a bro that exists.
/// `Ok(Err(msg))` is a 400.
pub(crate) fn checked_scope(
    conn: &Connection,
    raw: &str,
) -> rusqlite::Result<Result<String, &'static str>> {
    let Some(scope) = MemoryScope::parse(raw) else {
        return Ok(Err(BAD_SCOPE));
    };
    if let MemoryScope::Bot(id) = &scope {
        let exists = conn
            .query_row(
                "SELECT 1 FROM bots WHERE id = ?1 AND tenant_id = ?2",
                params![id, TENANT_ID],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(Err(UNKNOWN_BOT));
        }
    }
    Ok(Ok(scope.to_string()))
}

/// Bad JSON, a missing field or an unknown `kind` are all 400 here.
fn body_error(rejection: JsonRejection) -> Response {
    let message = if rejection.body_text().contains("unknown variant") {
        BAD_KIND
    } else {
        robroz_contract::host_text::BAD_REQUEST_FORMAT
    };
    error_response(StatusCode::BAD_REQUEST, message)
}

/// `?kind=&scope=&status=`; an empty value means no filter, unknown keys are ignored.
type ListQuery = std::collections::HashMap<String, String>;

pub(crate) async fn list_memories(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let param = |key: &str| query.get(key).map(String::as_str).filter(|v| !v.is_empty());
    let mut filters = vec!["forgotten_at IS NULL".to_string()];
    let mut values: Vec<String> = Vec::new();
    if let Some(kind) = param("kind") {
        let Some(kind) = MemoryKind::parse(kind) else {
            return error_response(StatusCode::BAD_REQUEST, BAD_KIND);
        };
        values.push(kind.as_str().into());
        filters.push(format!("kind = ?{}", values.len()));
    }
    if let Some(scope) = param("scope") {
        // Format only: a forgotten bro's memories can still be listed.
        let Some(scope) = MemoryScope::parse(scope) else {
            return error_response(StatusCode::BAD_REQUEST, BAD_SCOPE);
        };
        values.push(scope.to_string());
        filters.push(format!("scope = ?{}", values.len()));
    }
    match param("status") {
        Some(status) => {
            let Some(status) = MemoryStatus::parse(status) else {
                return error_response(StatusCode::BAD_REQUEST, BAD_STATUS);
            };
            values.push(status.as_str().into());
            filters.push(format!("status = ?{}", values.len()));
        }
        None => filters.push("status IN ('kept', 'pending')".into()),
    }
    let sql = format!("{SELECT} WHERE {} {ORDER}", filters.join(" AND "));
    let conn = lock_db(&state);
    let result = (|| {
        let mut stmt = conn.prepare(&sql)?;
        let args: Vec<&dyn ToSql> = values.iter().map(|v| v as &dyn ToSql).collect();
        let memories = stmt
            .query_map(args.as_slice(), from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok::<_, rusqlite::Error>(Memories { memories })
    })();
    drop(conn);
    match result {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn create_memory(
    State(state): State<AppState>,
    body: Result<Json<CreateMemory>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return body_error(rejection),
    };
    let text = match clean_text(&body.text) {
        Ok(text) => text,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let conn = lock_db(&state);
    let result = (|| {
        let scope = match checked_scope(&conn, &body.scope)? {
            Ok(scope) => scope,
            Err(msg) => return Ok(Err(msg)),
        };
        let id = new_id();
        let now = now_rfc3339();
        conn.execute(
            "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                scope,
                body.kind.as_str(),
                text,
                MemorySource::User.as_str(),
                MemoryStatus::Kept.as_str(),
                now
            ],
        )?;
        Ok::<_, rusqlite::Error>(Ok(load(&conn, &id)?.expect("just inserted")))
    })();
    drop(conn);
    match result {
        Ok(Ok(memory)) => (StatusCode::CREATED, Json(memory)).into_response(),
        Ok(Err(msg)) => error_response(StatusCode::BAD_REQUEST, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn update_memory(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Result<Json<UpdateMemory>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return body_error(rejection),
    };
    if body.text.is_none() && body.kind.is_none() && body.scope.is_none() {
        return error_response(
            StatusCode::BAD_REQUEST,
            robroz_contract::host_text::NOTHING_TO_CHANGE,
        );
    }
    let text = match body.text.as_deref().map(clean_text).transpose() {
        Ok(text) => text,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let conn = lock_db(&state);
    let result = (|| {
        let Some(current) = load(&conn, &id)? else {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        };
        let scope = match body.scope.as_deref() {
            None => current.scope,
            Some(raw) => match checked_scope(&conn, raw)? {
                Ok(scope) => scope,
                Err(msg) => return Ok(Err((StatusCode::BAD_REQUEST, msg))),
            },
        };
        let kind = body.kind.unwrap_or(current.kind);
        let text = text.unwrap_or(current.text);
        conn.execute(
            "UPDATE memories SET scope = ?2, kind = ?3, text = ?4, updated_at = ?5 WHERE id = ?1 AND forgotten_at IS NULL",
            params![id, scope, kind.as_str(), text, now_rfc3339()],
        )?;
        Ok::<_, rusqlite::Error>(Ok(load(&conn, &id)?.expect("still there")))
    })();
    drop(conn);
    match result {
        Ok(Ok(memory)) => (StatusCode::OK, Json(memory)).into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn forget_memory(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let mut conn = lock_db(&state);
    let result = (|| {
        let conn = conn.transaction()?;
        // The first forget time stands; forgetting again changes nothing.
        let forgotten_at: Option<Option<String>> = conn
            .query_row(
                "SELECT forgotten_at FROM memories WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        match forgotten_at {
            None => Ok(false),
            Some(Some(_)) => Ok(true),
            Some(None) => {
                let now = now_rfc3339();
                conn.execute(
                    "UPDATE memories SET forgotten_at = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
                // A pending one may still have a card asking about it: close it.
                crate::proposals::close_for_forgotten(
                    &conn,
                    robroz_contract::ConfirmSubject::Memory,
                    &id,
                    &now,
                )?;
                conn.commit()?;
                Ok::<_, rusqlite::Error>(true)
            }
        }
    })();
    drop(conn);
    match result {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, NOT_FOUND),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::{kept_for, prompt_section, prompt_section_with, SCHEMA};

    fn temp_path(ext: &str) -> PathBuf {
        std::env::temp_dir().join(format!("robroz-mem-test-{}.{ext}", uuid::Uuid::new_v4()))
    }

    fn app() -> (Router, PathBuf) {
        let db = temp_path("db");
        let volumes = temp_path("volumes");
        let router = crate::Host::open(&db, &volumes, "robroz-runner:local")
            .unwrap()
            .with_control_root(volumes.with_extension("control"))
            .router();
        (router, db)
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    async fn new_bro(app: &Router, name: &str) -> String {
        let (status, bro) = send(app, "POST", "/api/bros", Some(json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::CREATED);
        bro["id"].as_str().unwrap().to_string()
    }

    async fn remember(app: &Router, scope: &str, kind: &str, text: &str) -> Value {
        let (status, memory) = send(
            app,
            "POST",
            "/api/memories",
            Some(json!({ "scope": scope, "kind": kind, "text": text })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{memory}");
        memory
    }

    async fn texts(app: &Router, query: &str) -> Vec<String> {
        let (status, body) = send(app, "GET", &format!("/api/memories{query}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["memories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["text"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn memories_crud_and_soft_forget() {
        let (app, _) = app();
        let ada = new_bro(&app, "Ada").await;

        let first = remember(&app, "user", "profile", "  我叫 Panda，在伦敦 ").await;
        assert_eq!(first["text"], "我叫 Panda，在伦敦");
        assert_eq!(first["scope"], "user");
        assert_eq!(first["kind"], "profile");
        assert_eq!(first["source"], "user");
        assert_eq!(first["status"], "kept");
        assert_eq!(first["createdAt"], first["updatedAt"]);
        assert!(first.get("forgottenAt").is_none());
        let second = remember(&app, &format!("bot:{ada}"), "habit", "周报用中文").await;
        assert_eq!(second["scope"], format!("bot:{ada}"));

        assert_eq!(
            texts(&app, "").await,
            ["周报用中文", "我叫 Panda，在伦敦"],
            "newest first"
        );

        let id = first["id"].as_str().unwrap();
        let (status, changed) = send(
            &app,
            "PATCH",
            &format!("/api/memories/{id}"),
            Some(json!({ "text": "我叫 Panda", "kind": "habit" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["text"], "我叫 Panda");
        assert_eq!(changed["kind"], "habit");
        assert_eq!(changed["scope"], "user", "untouched field stays");
        assert_eq!(changed["createdAt"], first["createdAt"]);
        assert!(changed["updatedAt"].as_str().unwrap() >= first["updatedAt"].as_str().unwrap());
        assert_eq!(
            texts(&app, "").await[0],
            "我叫 Panda",
            "edited one moves to the top"
        );

        let (status, moved) = send(
            &app,
            "PATCH",
            &format!("/api/memories/{id}"),
            Some(json!({ "scope": format!("bot:{ada}") })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(moved["scope"], format!("bot:{ada}"));

        let (status, body) = send(&app, "DELETE", &format!("/api/memories/{id}"), None).await;
        assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
        let (status, _) = send(&app, "DELETE", &format!("/api/memories/{id}"), None).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "forgetting again is a no-op"
        );
        assert_eq!(texts(&app, "").await, ["周报用中文"]);
        for status_filter in ["kept", "pending", "rejected"] {
            assert!(!texts(&app, &format!("?status={status_filter}"))
                .await
                .contains(&"我叫 Panda".to_string()));
        }
        let (status, _) = send(
            &app,
            "PATCH",
            &format!("/api/memories/{id}"),
            Some(json!({ "text": "x" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "forgotten can't be edited");
        let (status, _) = send(
            &app,
            "PATCH",
            "/api/memories/nope",
            Some(json!({ "text": "x" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = send(&app, "DELETE", "/api/memories/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn memories_filter_and_rejected_hidden_by_default() {
        let (app, db) = app();
        let ada = new_bro(&app, "Ada").await;
        let bob = new_bro(&app, "Bob").await;
        remember(&app, "user", "profile", "u-profile").await;
        remember(&app, &format!("bot:{ada}"), "project", "ada-project").await;
        remember(&app, &format!("bot:{bob}"), "habit", "bob-habit").await;
        {
            // Bro proposals arrive in PR ②; seed them directly.
            let conn = Connection::open(&db).unwrap();
            for (id, status, at) in [
                ("p1", "pending", "2000-01-01T00:00:00.000Z"),
                ("r1", "rejected", "2000-01-01T00:00:00.000Z"),
            ] {
                conn.execute(
                    "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at)
                     VALUES (?1, 'user', 'habit', ?1, 'bot', ?2, ?3, ?3)",
                    rusqlite::params![id, status, at],
                )
                .unwrap();
            }
        }
        assert_eq!(
            texts(&app, "").await,
            ["bob-habit", "ada-project", "u-profile", "p1"]
        );
        assert_eq!(texts(&app, "?status=rejected").await, ["r1"]);
        assert_eq!(texts(&app, "?status=pending").await, ["p1"]);
        assert_eq!(texts(&app, "?kind=habit").await, ["bob-habit", "p1"]);
        assert_eq!(
            texts(&app, &format!("?scope=bot:{ada}")).await,
            ["ada-project"]
        );
        assert_eq!(
            texts(&app, "?scope=user&kind=habit&status=kept").await,
            Vec::<String>::new()
        );
        assert_eq!(
            texts(&app, "?kind=&scope=&status=").await.len(),
            4,
            "empty params mean no filter"
        );

        for query in [
            "?kind=mood",
            "?scope=group:g1",
            "?scope=bot:",
            "?status=forgotten",
        ] {
            let (status, body) = send(&app, "GET", &format!("/api/memories{query}"), None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
        }
    }

    #[tokio::test]
    async fn memories_validate_scope_kind_and_text() {
        let (app, _) = app();
        let ada = new_bro(&app, "Ada").await;
        let cases = [
            (
                json!({ "scope": "group:g1", "kind": "habit", "text": "x" }),
                "scope 只能是 user 或 bot:<botId>",
            ),
            (
                json!({ "scope": "everyone", "kind": "habit", "text": "x" }),
                "scope 只能是 user 或 bot:<botId>",
            ),
            (
                json!({ "scope": "bot:ghost", "kind": "habit", "text": "x" }),
                "这个 bro 不存在",
            ),
            (
                json!({ "scope": "user", "kind": "mood", "text": "x" }),
                "kind 只能是 profile、habit 或 project",
            ),
            (
                json!({ "scope": "user", "kind": "habit", "text": "   " }),
                "请填写内容",
            ),
            (
                json!({ "scope": "user", "kind": "habit", "text": "长".repeat(2001) }),
                "内容太长，最多 2000 个字",
            ),
            (json!({ "scope": "user", "text": "x" }), "请求格式不正确"),
        ];
        for (body, message) in cases {
            let (status, error) = send(&app, "POST", "/api/memories", Some(body.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(error, json!({ "error": message }), "{body}");
        }
        remember(&app, "user", "habit", &"长".repeat(2000)).await;

        let memory = remember(&app, &format!("bot:{ada}"), "habit", "x").await;
        let uri = format!("/api/memories/{}", memory["id"].as_str().unwrap());
        for body in [
            json!({}),
            json!({ "scope": "bot:ghost" }),
            json!({ "scope": "group:g1" }),
            json!({ "kind": "mood" }),
            json!({ "text": " " }),
        ] {
            let (status, _) = send(&app, "PATCH", &uri, Some(body.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        let (_, same) = send(&app, "GET", &format!("/api/memories?scope=bot:{ada}"), None).await;
        assert_eq!(
            same["memories"],
            json!([memory]),
            "failed edits change nothing"
        );
    }

    #[tokio::test]
    async fn memories_require_the_api_token() {
        let db = temp_path("db");
        let token = crate::auth::ApiToken::parse("0123456789abcdef-good").unwrap();
        let app = crate::Host::open(&db, temp_path("volumes"), "robroz-runner:local")
            .unwrap()
            .with_token(token)
            .router();
        for (method, uri) in [
            ("GET", "/api/memories"),
            ("POST", "/api/memories"),
            ("PATCH", "/api/memories/x"),
            ("DELETE", "/api/memories/x"),
        ] {
            let (status, _) = send(&app, method, uri, Some(json!({}))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
    }

    #[test]
    fn kept_for_gives_each_bro_only_its_own_and_shared_kept() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(SCHEMA).expect("migration is idempotent");
        conn.execute_batch(crate::approvals::SCHEMA).unwrap();
        conn.execute("ALTER TABLE approvals ADD COLUMN task_id TEXT", [])
            .unwrap();
        crate::proposals::migrate(&conn).unwrap();
        let rows = [
            ("shared", "user", "kept", None, "2026-01-01T00:00:01.000Z"),
            (
                "ada-old",
                "bot:ada",
                "kept",
                None,
                "2026-01-01T00:00:00.000Z",
            ),
            (
                "ada-new",
                "bot:ada",
                "kept",
                None,
                "2026-01-01T00:00:02.000Z",
            ),
            ("bob", "bot:bob", "kept", None, "2026-01-01T00:00:03.000Z"),
            (
                "pending",
                "user",
                "pending",
                None,
                "2026-01-01T00:00:04.000Z",
            ),
            (
                "rejected",
                "bot:ada",
                "rejected",
                None,
                "2026-01-01T00:00:05.000Z",
            ),
            (
                "forgotten",
                "user",
                "kept",
                Some("2026-01-02T00:00:00.000Z"),
                "2026-01-01T00:00:06.000Z",
            ),
        ];
        for (id, scope, status, forgotten, at) in rows {
            conn.execute(
                "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at, forgotten_at)
                 VALUES (?1, ?2, 'habit', ?1, 'user', ?3, ?5, ?5, ?4)",
                rusqlite::params![id, scope, status, forgotten, at],
            )
            .unwrap();
        }
        let ids = |bot: &str| {
            kept_for(&conn, bot)
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("ada"), ["ada-new", "shared", "ada-old"]);
        assert_eq!(ids("bob"), ["bob", "shared"]);
        assert_eq!(ids("carol"), ["shared"]);
        assert_eq!(ids("ad"), ["shared"], "no prefix matching");
    }

    fn sample(kind: robroz_contract::MemoryKind, text: &str) -> robroz_contract::Memory {
        use robroz_contract::{Memory, MemorySource, MemoryStatus};
        Memory {
            id: text.to_string(),
            scope: "user".into(),
            kind,
            text: text.into(),
            source: MemorySource::User,
            status: MemoryStatus::Kept,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            updated_at: "2026-01-01T00:00:00.000Z".into(),
            confirm_id: None,
        }
    }

    #[test]
    fn prompt_section_is_newest_first_and_empty_adds_nothing() {
        use robroz_contract::MemoryKind;
        assert_eq!(prompt_section(&[]), None);
        let section = prompt_section(&[
            sample(MemoryKind::Project, "新项目"),
            sample(MemoryKind::Habit, "周五交周报"),
            sample(MemoryKind::Profile, "叫 Panda"),
        ])
        .unwrap();
        assert_eq!(
            section,
            "You remember:\n[project] 新项目\n[habit] 周五交周报\n[profile] 叫 Panda"
        );
        let wrapped = sample(MemoryKind::Habit, "第一行\n  第二行");
        assert_eq!(
            prompt_section(&[wrapped]).as_deref(),
            Some("You remember:\n[habit] 第一行 第二行")
        );
    }

    #[test]
    fn prompt_section_drops_oldest_whole_entries_at_the_budget() {
        use robroz_contract::MemoryKind;
        let newer = sample(MemoryKind::Habit, "新");
        let older = sample(MemoryKind::Habit, "旧");
        let line_len = "[habit] 新".chars().count();
        let header_len = "You remember:".chars().count();
        let one = header_len + 1 + line_len;
        let two = one + 1 + "[habit] 旧".chars().count();
        assert_eq!(
            prompt_section_with(&[newer.clone(), older.clone()], two).as_deref(),
            Some("You remember:\n[habit] 新\n[habit] 旧")
        );
        assert_eq!(
            prompt_section_with(&[newer.clone(), older.clone()], two - 1).as_deref(),
            Some("You remember:\n[habit] 新")
        );
        assert_eq!(prompt_section_with(&[newer, older], one - 1), None);
        assert!(
            prompt_section(&[sample(MemoryKind::Habit, "短")])
                .unwrap()
                .chars()
                .count()
                <= super::MEMORY_PROMPT_MAX
        );
    }

    #[test]
    fn prompt_section_from_kept_for_skips_other_bros_and_unusable_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(crate::approvals::SCHEMA).unwrap();
        conn.execute("ALTER TABLE approvals ADD COLUMN task_id TEXT", [])
            .unwrap();
        crate::proposals::migrate(&conn).unwrap();
        let rows = [
            ("shared", "user", "kept", None, "2026-01-01T00:00:01.000Z"),
            (
                "ada-old",
                "bot:ada",
                "kept",
                None,
                "2026-01-01T00:00:00.000Z",
            ),
            (
                "ada-new",
                "bot:ada",
                "kept",
                None,
                "2026-01-01T00:00:02.000Z",
            ),
            ("bob", "bot:bob", "kept", None, "2026-01-01T00:00:03.000Z"),
            (
                "pending",
                "user",
                "pending",
                None,
                "2026-01-01T00:00:04.000Z",
            ),
            (
                "rejected",
                "bot:ada",
                "rejected",
                None,
                "2026-01-01T00:00:05.000Z",
            ),
            (
                "forgotten",
                "user",
                "kept",
                Some("2026-01-02T00:00:00.000Z"),
                "2026-01-01T00:00:06.000Z",
            ),
        ];
        for (id, scope, status, forgotten, at) in rows {
            conn.execute(
                "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at, forgotten_at)
                 VALUES (?1, ?2, 'habit', ?1, 'user', ?3, ?5, ?5, ?4)",
                rusqlite::params![id, scope, status, forgotten, at],
            )
            .unwrap();
        }
        let section = prompt_section(&kept_for(&conn, "ada").unwrap()).unwrap();
        assert_eq!(
            section,
            "You remember:\n[habit] ada-new\n[habit] shared\n[habit] ada-old"
        );
        assert!(
            !section.contains("bob")
                && !section.contains("pending")
                && !section.contains("rejected")
                && !section.contains("forgotten")
        );
        assert_eq!(
            prompt_section(&kept_for(&conn, "carol").unwrap()).as_deref(),
            Some("You remember:\n[habit] shared")
        );
    }
}
