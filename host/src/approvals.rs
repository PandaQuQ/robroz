//! Approval cards: a bot in the group asks before it deletes, sends or spends.
//! The person approves or denies once, each through its own endpoint.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::body::Bytes;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{
    clean_answered_by, Answer, ApprovalAction, ApprovalDecision, CreateApproval, Event, APPROVAL_SUMMARY_MAX,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, error_response, group_exists, lock_db, new_id, now_rfc3339, AppState, TimedEvent};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS approvals (
        id TEXT PRIMARY KEY,
        group_id TEXT NOT NULL,
        bot_id TEXT NOT NULL,
        action TEXT NOT NULL,
        summary TEXT NOT NULL,
        path TEXT,
        decision TEXT,
        created_at TEXT NOT NULL,
        answered_at TEXT
    );
";

const SELECT: &str = "SELECT a.id, a.bot_id, b.name, a.action, a.summary, a.path, a.decision, a.created_at, a.answered_at, a.answered_by
     FROM approvals a JOIN bots b ON b.id = a.bot_id";

fn action_text(action: ApprovalAction) -> &'static str {
    match action {
        ApprovalAction::Delete => "delete",
        ApprovalAction::Send => "send",
        ApprovalAction::Spend => "spend",
    }
}

fn decision_text(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::Denied => "denied",
    }
}

fn bad_column(column: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, format!("unknown value {text:?}").into())
}

fn parse_action(text: String) -> rusqlite::Result<ApprovalAction> {
    [ApprovalAction::Delete, ApprovalAction::Send, ApprovalAction::Spend]
        .into_iter()
        .find(|a| action_text(*a) == text)
        .ok_or_else(|| bad_column(3, &text))
}

fn parse_decision(text: String) -> rusqlite::Result<ApprovalDecision> {
    [ApprovalDecision::Approved, ApprovalDecision::Denied]
        .into_iter()
        .find(|d| decision_text(*d) == text)
        .ok_or_else(|| bad_column(6, &text))
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let action = parse_action(row.get(3)?)?;
    let decision: Option<String> = row.get(6)?;
    let decision = decision.map(parse_decision).transpose()?;
    Ok(Event::Approval {
        id: row.get(0)?,
        bot_id: row.get(1)?,
        bot_name: row.get(2)?,
        action,
        summary: row.get(4)?,
        path: row.get(5)?,
        decision,
        created_at: row.get(7)?,
        answered_at: row.get(8)?,
        answered_by: row.get(9)?,
    })
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    conn.query_row(&format!("{SELECT} WHERE a.id = ?1"), [id], event_from_row).optional()
}

/// Approval events for one group, for the events stream.
pub(crate) fn timed_events(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<TimedEvent>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE a.group_id = ?1"))?;
    let rows = stmt.query_map([group_id], |row| {
        let id: String = row.get(0)?;
        let created_at: String = row.get(7)?;
        let answered_at: Option<String> = row.get(8)?;
        let changed_at = answered_at.unwrap_or_else(|| created_at.clone());
        Ok(TimedEvent { created_at, changed_at, id, event: event_from_row(row)? })
    })?;
    rows.collect()
}

/// Trims summary and path; summary is one non-empty line, path is optional.
pub fn validate(summary: &str, path: Option<&str>) -> Result<(String, Option<String>), &'static str> {
    let summary = summary.trim();
    if summary.is_empty() {
        return Err("请填写要做什么");
    }
    if summary.chars().count() > APPROVAL_SUMMARY_MAX || summary.contains('\n') {
        return Err("说明只能一行，最多 200 字");
    }
    let path = path.map(str::trim).filter(|p| !p.is_empty()).map(String::from);
    Ok((summary.to_string(), path))
}

pub(crate) async fn create_approval(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(body): Json<CreateApproval>,
) -> Response {
    let bot_id = body.bot_id.trim().to_string();
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err(error_response(StatusCode::NOT_FOUND, "群不存在")));
        }
        let member: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memberships WHERE group_id = ?1 AND bot_id = ?2",
            params![group_id, bot_id],
            |row| row.get(0),
        )?;
        if member == 0 {
            return Ok(Err(error_response(StatusCode::BAD_REQUEST, "这个 Bot 不在群里")));
        }
        let (summary, path) = match validate(&body.summary, body.path.as_deref()) {
            Ok(clean) => clean,
            Err(message) => return Ok(Err(error_response(StatusCode::BAD_REQUEST, message))),
        };
        let id = new_id();
        conn.execute(
            "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL)",
            params![id, group_id, bot_id, action_text(body.action), summary, path, now_rfc3339()],
        )?;
        Ok(Ok(load(&conn, &id)?.expect("approval just inserted")))
    })();
    drop(conn);
    match result {
        Ok(Ok(event)) => (StatusCode::CREATED, Json(event)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

/// One answer per card: the update only matches an unanswered row, so a
/// second click (or a race) gets 409 and the first answer stands.
fn decide(state: &AppState, id: &str, decision: ApprovalDecision, body: &[u8]) -> Response {
    let by = match answered_by(body) {
        Ok(by) => by,
        Err((status, message)) => return error_response(status, message),
    };
    let conn = lock_db(state);
    let result = (|| {
        if load(&conn, id)?.is_none() {
            return Ok(Err(error_response(StatusCode::NOT_FOUND, "审批卡不存在")));
        }
        let changed = conn.execute(
            "UPDATE approvals SET decision = ?1, answered_at = ?2, answered_by = ?4 WHERE id = ?3 AND decision IS NULL",
            params![decision_text(decision), now_rfc3339(), id, by],
        )?;
        if changed == 0 {
            return Ok(Err(error_response(StatusCode::CONFLICT, "已经答过了")));
        }
        Ok(Ok(load(&conn, id)?.expect("approval exists")))
    })();
    drop(conn);
    match result {
        Ok(Ok(event)) => (StatusCode::OK, Json(event)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

/// The optional `{"answeredBy"}` body. Empty body is the old request.
/// Errors are `(status, message)`.
fn answered_by(body: &[u8]) -> Result<Option<String>, (StatusCode, &'static str)> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let answer: Answer =
        serde_json::from_slice(body).map_err(|_| (StatusCode::UNPROCESSABLE_ENTITY, "请求格式不正确"))?;
    clean_answered_by(answer.answered_by.as_deref()).map_err(|m| (StatusCode::BAD_REQUEST, m))
}

pub(crate) async fn approve(State(state): State<AppState>, AxumPath(id): AxumPath<String>, body: Bytes) -> Response {
    decide(&state, &id, ApprovalDecision::Approved, &body)
}

pub(crate) async fn deny(State(state): State<AppState>, AxumPath(id): AxumPath<String>, body: Bytes) -> Response {
    decide(&state, &id, ApprovalDecision::Denied, &body)
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn summary_is_one_trimmed_line_and_path_is_optional() {
        assert_eq!(validate("  删缓存 ", Some(" target/ ")), Ok(("删缓存".into(), Some("target/".into()))));
        assert_eq!(validate("删缓存", Some("  ")), Ok(("删缓存".into(), None)));
        assert!(validate("   ", None).is_err());
        assert!(validate("两行\n不行", None).is_err());
        assert!(validate(&"长".repeat(201), None).is_err());
        assert!(validate(&"长".repeat(200), None).is_ok());
    }
}
