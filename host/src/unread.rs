//! Unread counts: one read position per group (single owner for now).
//!
//! - `GET  /api/unread`               all groups, most recently active first
//! - `GET  /api/groups/{id}/unread`   one group
//! - `POST /api/groups/{id}/read`     `{readAt?}`, default now, forward only
//!
//! What counts is defined once in [`UNREAD_SQL`]; see `robroz_contract::Unread`.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{MarkRead, Unread, Unreads};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    db_error, error_response, group_exists, lock_db, now_rfc3339, parse_since, AppState, TENANT_ID,
};

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS group_reads (
    group_id TEXT PRIMARY KEY,
    read_at TEXT NOT NULL
);";

/// ?1 group id, ?2 read position ('' when never read, which sorts before any time).
const UNREAD_SQL: &str = "SELECT
    (SELECT COUNT(*) FROM deliveries WHERE group_id = ?1 AND created_at > ?2)
  + (SELECT COUNT(*) FROM choices WHERE group_id = ?1 AND created_at > ?2)
  + (SELECT COUNT(*) FROM approvals WHERE group_id = ?1 AND created_at > ?2
       AND (review_state IS NULL OR review_state = 'blocked'))
  + (SELECT COUNT(*) FROM tasks WHERE group_id = ?1 AND state = 'blocked' AND updated_at > ?2)";

const LAST_ACTIVITY_SQL: &str = "SELECT MAX(at) FROM (
    SELECT MAX(created_at) AS at FROM messages WHERE group_id = ?1
    UNION ALL SELECT MAX(updated_at) FROM tasks WHERE group_id = ?1
    UNION ALL SELECT MAX(created_at) FROM deliveries WHERE group_id = ?1
    UNION ALL SELECT MAX(COALESCE(answered_at, created_at)) FROM choices WHERE group_id = ?1
    UNION ALL SELECT MAX(COALESCE(answered_at, created_at)) FROM approvals
      WHERE (group_id = ?1 OR origin_group_id = ?1) AND (review_state IS NULL OR review_state = 'blocked')
)";

fn read_at(conn: &Connection, group_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT read_at FROM group_reads WHERE group_id = ?1",
        [group_id],
        |r| r.get(0),
    )
    .optional()
}

fn unread_of(conn: &Connection, group_id: &str) -> rusqlite::Result<Unread> {
    let read_at = read_at(conn, group_id)?;
    let floor = read_at.clone().unwrap_or_default();
    let unread: i64 = conn.query_row(UNREAD_SQL, params![group_id, floor], |r| r.get(0))?;
    let last_activity_at: Option<String> =
        conn.query_row(LAST_ACTIVITY_SQL, [group_id], |r| r.get(0))?;
    Ok(Unread {
        group_id: group_id.to_string(),
        read_at,
        unread: unread as u64,
        last_activity_at,
    })
}

pub(crate) async fn list_unread(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let mut stmt =
            conn.prepare("SELECT id FROM groups WHERE tenant_id = ?1 ORDER BY created_at, rowid")?;
        let ids = stmt
            .query_map([TENANT_ID], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut groups = ids
            .iter()
            .map(|id| unread_of(&conn, id))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Newest activity first; empty groups last, in creation order (stable sort).
        groups.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
        Ok::<_, rusqlite::Error>(Unreads { groups })
    })();
    drop(conn);
    match result {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn group_unread(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(None);
        }
        unread_of(&conn, &group_id).map(Some)
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(body)) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn mark_read(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    body: Option<Json<MarkRead>>,
) -> Response {
    let requested = body.and_then(|Json(b)| b.read_at);
    let at = match parse_since(requested.as_deref()) {
        Ok(Some(at)) => at,
        Ok(None) => now_rfc3339(),
        Err(()) => return error_response(StatusCode::BAD_REQUEST, "readAt 不是合法时间"),
    };
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(None);
        }
        conn.execute(
            "INSERT INTO group_reads (group_id, read_at) VALUES (?1, ?2)
             ON CONFLICT(group_id) DO UPDATE SET read_at = MAX(read_at, excluded.read_at)",
            params![group_id, at],
        )?;
        unread_of(&conn, &group_id).map(Some)
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(body)) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}
