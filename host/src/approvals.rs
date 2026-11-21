//! Approval cards: a bot in the group asks before it deletes, sends or spends.
//! The person approves or denies once, each through its own endpoint.

use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{
    clean_answered_by, Answer, ApprovalAction, ApprovalDecision, CreateApproval, Event,
    APPROVAL_SUMMARY_MAX,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    db_error, error_response, group_exists, lock_db, new_id, now_rfc3339, AppState, TimedEvent,
};

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

/// Approval cards only: confirm cards (`subject_kind` set) live in [`crate::proposals`].
const SELECT: &str = "SELECT a.id, a.bot_id, b.name, a.action, a.summary, a.path, a.decision, a.created_at, a.answered_at, a.answered_by,
            a.task_id, a.mcp_server, a.mcp_tool
     FROM approvals a JOIN bots b ON b.id = a.bot_id";
const APPROVALS_ONLY: &str = "a.subject_kind IS NULL";

fn action_text(action: ApprovalAction) -> &'static str {
    match action {
        ApprovalAction::Delete => "delete",
        ApprovalAction::Send => "send",
        ApprovalAction::Spend => "spend",
        ApprovalAction::Call => "call",
    }
}

pub(crate) fn decision_text(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::Denied => "denied",
    }
}

fn bad_column(column: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("unknown value {text:?}").into(),
    )
}

fn parse_action(text: String) -> rusqlite::Result<ApprovalAction> {
    [
        ApprovalAction::Delete,
        ApprovalAction::Send,
        ApprovalAction::Spend,
        ApprovalAction::Call,
    ]
    .into_iter()
    .find(|a| action_text(*a) == text)
    .ok_or_else(|| bad_column(3, &text))
}

pub(crate) fn parse_decision(text: String) -> rusqlite::Result<ApprovalDecision> {
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
        task_id: row.get(10)?,
        mcp_server: row.get(11)?,
        mcp_tool: row.get(12)?,
    })
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    conn.query_row(
        &format!("{SELECT} WHERE a.id = ?1 AND {APPROVALS_ONLY}"),
        [id],
        event_from_row,
    )
    .optional()
}

/// Approval events for one group, for the events stream.
pub(crate) fn timed_events(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<TimedEvent>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE a.group_id = ?1 AND {APPROVALS_ONLY}"
    ))?;
    let rows = stmt.query_map([group_id], |row| {
        let id: String = row.get(0)?;
        let created_at: String = row.get(7)?;
        let answered_at: Option<String> = row.get(8)?;
        let changed_at = answered_at.unwrap_or_else(|| created_at.clone());
        Ok(TimedEvent {
            created_at,
            changed_at,
            id,
            event: event_from_row(row)?,
        })
    })?;
    rows.collect()
}

/// Trims summary and path; summary is one non-empty line, path is optional.
pub fn validate(
    summary: &str,
    path: Option<&str>,
) -> Result<(String, Option<String>), &'static str> {
    let summary = summary.trim();
    if summary.is_empty() {
        return Err("请填写要做什么");
    }
    if summary.chars().count() > APPROVAL_SUMMARY_MAX || summary.contains('\n') {
        return Err("说明只能一行，最多 200 字");
    }
    let path = path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from);
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
            return Ok(Err(error_response(
                StatusCode::BAD_REQUEST,
                "这个 Bot 不在群里",
            )));
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

/// Blocking kinds that raise an approval card instead of waiting for a
/// takeover. A captcha still needs the person to type, so it is not one.
pub(crate) fn needs_approval(kind: &str) -> bool {
    kind == "external"
}

/// Card text for a task: its first plain line, shortened.
pub(crate) fn gate_summary(prompt: &str) -> String {
    format!("{GATE_PREFIX}{}", gate_line(prompt))
}

const GATE_PREFIX: &str = "要对外写入：";

/// The task's first plain line, at most 60 characters.
fn gate_line(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && robroz_policy::blocking_kind(l).is_none())
        .unwrap_or("这个任务");
    crate::text::clip(line, 60)
}

/// A blocked task asks before writing outside. The card names the task and
/// the blocking line; answering it resumes or ends that task.
pub(crate) fn gate_task(
    conn: &Connection,
    task_id: &str,
    prompt: &str,
    hash: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at, task_id, intent_hash)
         SELECT ?1, group_id, assignee_bot_id, 'send', ?2, NULL, NULL, ?3, NULL, id, ?4 FROM tasks WHERE id = ?5",
        params![new_id(), gate_summary(prompt), now, hash, task_id],
    )?;
    Ok(())
}

/// What a gated card points at, if it was raised by a task.
/// An MCP tool call waiting for approval. `action` is `call`; the card
/// carries `mcp_server` and `mcp_tool`.
pub(crate) fn gate_mcp_task(
    conn: &Connection,
    task_id: &str,
    summary: &str,
    hash: &str,
    now: &str,
    server: &str,
    tool: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at, task_id, intent_hash, mcp_server, mcp_tool)
         SELECT ?1, group_id, assignee_bot_id, 'call', ?2, NULL, NULL, ?3, NULL, id, ?4, ?5, ?6 FROM tasks WHERE id = ?7",
        params![new_id(), summary, now, hash, server, tool, task_id],
    )?;
    Ok(())
}

fn is_mcp_card(event: &Event) -> bool {
    matches!(
        event,
        Event::Approval {
            action: ApprovalAction::Call,
            ..
        }
    )
}

fn gate_of(conn: &Connection, id: &str) -> rusqlite::Result<Option<(String, String)>> {
    Ok(conn
        .query_row(
            "SELECT task_id, intent_hash FROM approvals WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()?
        .and_then(|(task, hash)| task.zip(hash)))
}

/// Apply the answer to the task, only if it is still blocked on that line
/// (a takeover through `/resume` may have moved it on already).
fn settle_task(
    conn: &Connection,
    task_id: &str,
    hash: &str,
    decision: ApprovalDecision,
) -> rusqlite::Result<()> {
    // Denied is done without the work: say so, so the card is not a green "done".
    let (next, failure) = match decision {
        ApprovalDecision::Approved => ("queued", None),
        ApprovalDecision::Denied => {
            let prompt: String =
                conn.query_row("SELECT prompt FROM tasks WHERE id = ?1", [task_id], |r| {
                    r.get(0)
                })?;
            ("done", Some(format!("你拒绝了：{}", gate_line(&prompt))))
        }
    };
    conn.execute(
        "UPDATE tasks SET state = ?1, container_up = 0, updated_at = ?2, failure = ?5,
                          intent_kind = NULL, intent_line = NULL, intent_hash = NULL
         WHERE id = ?3 AND state = 'blocked' AND intent_hash = ?4",
        params![next, now_rfc3339(), task_id, hash, failure],
    )?;
    Ok(())
}

/// One answer per card: the update only matches an unanswered row, so a
/// second click (or a race) gets 409 and the first answer stands.
///
/// A card raised by a blocked task also moves that task: approve writes the
/// takeover marker and queues it again, deny ends it as done (no result).
fn decide(state: &AppState, id: &str, decision: ApprovalDecision, body: &[u8]) -> Response {
    let by = match answered_by(body) {
        Ok(by) => by,
        Err((status, message)) => return error_response(status, message),
    };
    let mut conn = lock_db(state);
    let result = (|| {
        // A confirm card answers the same way but settles what a bro proposed.
        if crate::proposals::is_confirm(&conn, id)? {
            return Ok(
                crate::proposals::answer(&mut conn, id, decision, by.as_deref())?
                    .map_err(|(status, message)| error_response(status, message)),
            );
        }
        let Some(card) = load(&conn, id)? else {
            return Ok(Err(error_response(StatusCode::NOT_FOUND, "审批卡不存在")));
        };
        if matches!(
            card,
            Event::Approval {
                decision: Some(_),
                ..
            }
        ) {
            return Ok(Err(error_response(StatusCode::CONFLICT, "已经答过了")));
        }
        let gate = gate_of(&conn, id)?;
        let mcp = is_mcp_card(&card);
        // Marker first: if it can't be written nothing changes and the card stays open.
        if let (Some((task_id, _hash)), ApprovalDecision::Approved) = (&gate, decision) {
            if mcp {
                if let Err(err) = crate::mcp_tools::mark_approved(&state.control_root.join(task_id))
                {
                    eprintln!("approve {id}: mcp pending for {task_id}: {err}");
                    return Ok(Err(error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "批准失败，无法写入继续标记",
                    )));
                }
            } else if let Err(err) =
                robroz_runner::append_takeover(&state.control_root.join(task_id), _hash)
            {
                eprintln!("approve {id}: takeover for {task_id}: {err}");
                return Ok(Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "批准失败，无法写入接管标记",
                )));
            }
        }
        let now = now_rfc3339();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE approvals SET decision = ?1, answered_at = ?2, answered_by = ?4 WHERE id = ?3 AND decision IS NULL",
            params![decision_text(decision), now, id, by],
        )?;
        if changed == 0 {
            return Ok(Err(error_response(StatusCode::CONFLICT, "已经答过了")));
        }
        if let Some((task_id, hash)) = &gate {
            if mcp {
                match decision {
                    ApprovalDecision::Approved => {
                        crate::mcp_tools::queue_approved(&tx, task_id, hash, &now)?;
                    }
                    ApprovalDecision::Denied => {
                        let line = match &card {
                            Event::Approval {
                                mcp_server: Some(s),
                                mcp_tool: Some(tool),
                                ..
                            } => {
                                format!("{s}.{tool}")
                            }
                            Event::Approval { summary, .. } => summary.clone(),
                            _ => "MCP 工具".into(),
                        };
                        let failure = format!("你拒绝了：{line}");
                        crate::mcp_tools::finish_denied(&tx, task_id, hash, &failure, &now)?;
                        let _ = crate::mcp_tools::remove_pending(&state.control_root.join(task_id));
                    }
                }
            } else {
                settle_task(&tx, task_id, hash, decision)?;
            }
        }
        tx.commit()?;
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
    let answer: Answer = serde_json::from_slice(body)
        .map_err(|_| (StatusCode::UNPROCESSABLE_ENTITY, "请求格式不正确"))?;
    clean_answered_by(answer.answered_by.as_deref()).map_err(|m| (StatusCode::BAD_REQUEST, m))
}

pub(crate) async fn approve(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> Response {
    decide(&state, &id, ApprovalDecision::Approved, &body)
}

pub(crate) async fn deny(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body: Bytes,
) -> Response {
    decide(&state, &id, ApprovalDecision::Denied, &body)
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn summary_is_one_trimmed_line_and_path_is_optional() {
        assert_eq!(
            validate("  删缓存 ", Some(" target/ ")),
            Ok(("删缓存".into(), Some("target/".into())))
        );
        assert_eq!(validate("删缓存", Some("  ")), Ok(("删缓存".into(), None)));
        assert!(validate("   ", None).is_err());
        assert!(validate("两行\n不行", None).is_err());
        assert!(validate(&"长".repeat(201), None).is_err());
        assert!(validate(&"长".repeat(200), None).is_ok());
    }
}
