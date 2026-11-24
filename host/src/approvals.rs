//! Approval cards: a bot in the group asks before it deletes, sends or spends.
//! The person approves or denies once, each through its own endpoint.

use std::path::Path;

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
/// Settings-page rule cards have no bro: `botId` / `botName` are the
/// contract's `SETTINGS_APPROVAL_BOT_ID` / `SETTINGS_APPROVAL_BOT_NAME`.
const SELECT: &str = "SELECT a.id, a.bot_id, COALESCE(b.name, CASE WHEN a.bot_id = 'settings' THEN '设置' END), a.action, a.summary, a.path, a.decision, a.created_at, a.answered_at, a.answered_by,
            a.task_id, a.mcp_server, a.mcp_tool, a.reason, a.expires_at
     FROM approvals a LEFT JOIN bots b ON b.id = a.bot_id
     WHERE (b.id IS NOT NULL OR a.bot_id = 'settings')";
/// Approval cards the owner can see: not a confirm card, and not hidden by
/// auto-review (still being reviewed, or allowed without a card).
const APPROVALS_ONLY: &str =
    "a.subject_kind IS NULL AND (a.review_state IS NULL OR a.review_state = 'blocked')";

fn action_text(action: ApprovalAction) -> &'static str {
    match action {
        ApprovalAction::Delete => "delete",
        ApprovalAction::Send => "send",
        ApprovalAction::Spend => "spend",
        ApprovalAction::Call => "call",
        ApprovalAction::RuleAdd => "rule_add",
        ApprovalAction::RuleDelete => "rule_delete",
        ApprovalAction::Unknown => {
            debug_assert!(false, "host must never emit ApprovalAction::Unknown");
            "unknown"
        }
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
        ApprovalAction::RuleAdd,
        ApprovalAction::RuleDelete,
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
        reason: row.get(13)?,
        expires_at: row.get(14)?,
    })
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    conn.query_row(
        &format!("{SELECT} AND a.id = ?1 AND {APPROVALS_ONLY}"),
        [id],
        event_from_row,
    )
    .optional()
}

/// `GET /api/approvals/{id}`: one approval card the owner can see, in the
/// same shape as in events. Settings-page rule cards are only reachable here.
pub(crate) async fn get_approval(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    match load_settled_or_open(&conn, &id) {
        Ok(Some(event)) => (StatusCode::OK, Json(event)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            robroz_contract::host_text::APPROVAL_NOT_FOUND,
        ),
        Err(err) => db_error(err),
    }
}

/// A card by id, unless it is still under review. Cards auto-review allowed
/// (never shown) are included, so `GET` reads them and answering them is 409.
pub(crate) fn load_settled_or_open(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    let pending = conn
        .query_row(
            "SELECT review_state = 'pending' FROM approvals WHERE id = ?1",
            [id],
            |r| r.get::<_, Option<bool>>(0),
        )
        .optional()?;
    if pending == Some(Some(true)) {
        return Ok(None);
    }
    load_any(conn, id)
}

/// Any approval card, including ones auto-review has hidden.
pub(crate) fn load_any(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    conn.query_row(
        &format!("{SELECT} AND a.id = ?1 AND a.subject_kind IS NULL"),
        [id],
        event_from_row,
    )
    .optional()
}

/// Approval events for one group, for the events stream.
pub(crate) fn timed_events(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<TimedEvent>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} AND (a.group_id = ?1 OR a.origin_group_id = ?1) AND {APPROVALS_ONLY}"
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
        return Err(robroz_contract::host_text::NEED_ACTION);
    }
    if summary.chars().count() > APPROVAL_SUMMARY_MAX || summary.contains('\n') {
        return Err(robroz_contract::host_text::SUMMARY_ONE_LINE);
    }
    let path = path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from);
    Ok((summary.to_string(), path))
}

/// Where a bro-posted card goes: the owner's DM with that bro. A card posted
/// in a DM stays there. Returns `(card group, origin group if moved)`.
fn bro_card_group(
    conn: &Connection,
    group_id: &str,
    bot_id: &str,
) -> rusqlite::Result<(String, Option<String>)> {
    if crate::bros::dm_owner(conn, group_id)?.is_some() {
        return Ok((group_id.to_string(), None));
    }
    Ok(match crate::bros::owner_dm_for(conn, bot_id)? {
        Some(dm) => (dm, Some(group_id.to_string())),
        None => (group_id.to_string(), None),
    })
}

/// What a bro-posted card needs reviewed after the insert, if anything.
struct ToReview {
    id: String,
    action: String,
    rules: Vec<String>,
    reviewer: Option<std::sync::Arc<dyn crate::review::Reviewer>>,
}

/// A bro asks before it deletes, sends or spends. The card goes through
/// auto-review like a task card: with confirmed rules it is reviewed right
/// here (outside the DB lock), allow settles it as `approved` with the
/// reviewer's `reason` and shows no card, block shows it with `reason`.
/// Without rules it is shown at once. Shown cards live in the owner's DM with
/// the bro; the group where it was posted also lists it (without buttons).
/// The 201 body is the card as stored after review.
pub(crate) async fn create_approval(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(body): Json<CreateApproval>,
) -> Response {
    let result = insert_bro_card(&state, &group_id, &body);
    let (id, review) = match result {
        Ok(Ok(v)) => v,
        Ok(Err(response)) => return response,
        Err(err) => return db_error(err),
    };
    if let Some(review) = review {
        let ToReview {
            id: rid,
            action,
            rules,
            reviewer,
        } = review;
        let verdict = match reviewer {
            Some(r) => tokio::task::spawn_blocking(move || {
                crate::review::reviewer::review_with_retry(&*r, &action, &rules)
            })
            .await
            .unwrap_or_else(|_| {
                crate::review::Verdict::Block(crate::review::reviewer::REVIEW_FAILED_REASON.into())
            }),
            None => crate::review::Verdict::Block(crate::review::NO_MODEL_REASON.into()),
        };
        let mut conn = lock_db(&state);
        if let Err(err) = crate::review::apply(
            &mut conn,
            &state.control_root,
            &rid,
            &verdict,
            crate::review::approval_ttl(),
            chrono::Utc::now(),
        ) {
            return db_error(err);
        }
    }
    let conn = lock_db(&state);
    match load_any(&conn, &id) {
        Ok(Some(event)) => (StatusCode::CREATED, Json(event)).into_response(),
        Ok(None) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            robroz_contract::host_text::APPROVAL_VANISHED,
        ),
        Err(err) => db_error(err),
    }
}

/// Validate and store a bro-posted card; returns what still needs review.
#[allow(clippy::type_complexity)]
fn insert_bro_card(
    state: &AppState,
    group_id: &str,
    body: &CreateApproval,
) -> rusqlite::Result<Result<(String, Option<ToReview>), Response>> {
    let bot_id = body.bot_id.trim().to_string();
    let conn = lock_db(state);
    {
        if !group_exists(&conn, group_id)? {
            return Ok(Err(error_response(
                StatusCode::NOT_FOUND,
                robroz_contract::host_text::GROUP_NOT_FOUND,
            )));
        }
        let member: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memberships WHERE group_id = ?1 AND bot_id = ?2",
            params![group_id, bot_id],
            |row| row.get(0),
        )?;
        if member == 0 {
            return Ok(Err(error_response(
                StatusCode::BAD_REQUEST,
                robroz_contract::host_text::BOT_NOT_IN_GROUP,
            )));
        }
        if matches!(body.action, ApprovalAction::Unknown) {
            return Ok(Err(error_response(
                StatusCode::BAD_REQUEST,
                robroz_contract::host_text::ACTION_NOT_SUPPORTED,
            )));
        }
        if matches!(
            body.action,
            ApprovalAction::Call | ApprovalAction::RuleAdd | ApprovalAction::RuleDelete
        ) {
            return Ok(Err(error_response(
                StatusCode::BAD_REQUEST,
                robroz_contract::host_text::ACTION_HOST_ONLY,
            )));
        }
        let (summary, path) = match validate(&body.summary, body.path.as_deref()) {
            Ok(clean) => clean,
            Err(message) => return Ok(Err(error_response(StatusCode::BAD_REQUEST, message))),
        };
        let id = new_id();
        let now = now_rfc3339();
        let tx = conn.unchecked_transaction()?;
        let (card_group, origin) = bro_card_group(&tx, group_id, &bot_id)?;
        let gate = crate::review::open_gate(&tx, &now)?;
        tx.execute(
            "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at,
                                    review_state, expires_at, origin_group_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL, ?8, ?9, ?10)",
            params![
                id,
                card_group,
                bot_id,
                action_text(body.action),
                summary,
                path,
                now,
                gate.review_state,
                gate.expires_at,
                origin
            ],
        )?;
        tx.commit()?;
        if gate.review_state != crate::review::PENDING {
            return Ok(Ok((id, None)));
        }
        let mut action =
            crate::review::describe(action_text(body.action), &summary, None, None, None);
        if let Some(p) = &path {
            action.push_str(&format!("\nPath: {p}"));
        }
        let review = ToReview {
            action,
            rules: crate::review::rules::texts(&conn)?,
            reviewer: crate::review::reviewer_for(
                &conn,
                state.reviewer.as_ref(),
                state.model.as_ref(),
            ),
            id: id.clone(),
        };
        Ok(Ok((id, Some(review))))
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

const GATE_PREFIX: &str = robroz_contract::host_text::GATE_PREFIX;

/// The task's first plain line, at most 60 characters.
fn gate_line(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && robroz_policy::blocking_kind(l).is_none())
        .unwrap_or(robroz_contract::host_text::GATE_FALLBACK_LINE);
    crate::text::clip(line, 60)
}

/// Where a task's approval card goes: the owner's DM with the task's bro.
/// A task that already runs in a DM keeps its card there. If the bro is
/// unknown (no `bots` row), the card stays in the task's group.
fn card_group(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<String>> {
    let Some((group_id, bot_id)) = conn
        .query_row(
            "SELECT group_id, assignee_bot_id FROM tasks WHERE id = ?1",
            [task_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    if crate::bros::dm_owner(conn, &group_id)?.is_some() {
        return Ok(Some(group_id));
    }
    Ok(Some(
        crate::bros::owner_dm_for(conn, &bot_id)?.unwrap_or(group_id),
    ))
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
    let Some(group) = card_group(conn, task_id)? else {
        return Ok(());
    };
    let gate = crate::review::open_gate(conn, now)?;
    conn.execute(
        "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at, task_id, intent_hash,
                               review_state, expires_at)
         SELECT ?1, ?8, assignee_bot_id, 'send', ?2, NULL, NULL, ?3, NULL, id, ?4, ?6, ?7 FROM tasks WHERE id = ?5",
        params![new_id(), gate_summary(prompt), now, hash, task_id, gate.review_state, gate.expires_at, group],
    )?;
    Ok(())
}

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
    let Some(group) = card_group(conn, task_id)? else {
        return Ok(());
    };
    let gate = crate::review::open_gate(conn, now)?;
    conn.execute(
        "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at, task_id, intent_hash, mcp_server, mcp_tool,
                               review_state, expires_at)
         SELECT ?1, ?10, assignee_bot_id, 'call', ?2, NULL, NULL, ?3, NULL, id, ?4, ?5, ?6, ?8, ?9 FROM tasks WHERE id = ?7",
        params![new_id(), summary, now, hash, server, tool, task_id, gate.review_state, gate.expires_at, group],
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
    failure: Option<&str>,
) -> rusqlite::Result<()> {
    // Denied is done without the work: say so, so the card is not a green "done".
    // A task split by the Router ends whole: 「没写出结果」.
    let routed = decision == ApprovalDecision::Denied && crate::router::clear(conn, task_id)?;
    let (next, failure) = match decision {
        ApprovalDecision::Approved => ("queued", None),
        ApprovalDecision::Denied if routed => ("done", Some(robroz_card::NO_RESULT.to_string())),
        ApprovalDecision::Denied if failure.is_some() => ("done", failure.map(String::from)),
        ApprovalDecision::Denied => {
            let prompt: String =
                conn.query_row("SELECT prompt FROM tasks WHERE id = ?1", [task_id], |r| {
                    r.get(0)
                })?;
            (
                "done",
                Some(format!(
                    "{}{}",
                    robroz_contract::host_text::DENIED_PREFIX,
                    gate_line(&prompt)
                )),
            )
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

/// How [`settle`] answers a card. `Default` is a plain answer from the owner.
pub(crate) struct Settlement<'a> {
    pub decision: ApprovalDecision,
    /// The client-reported `answeredBy`. `None` for auto-review and expiry.
    pub by: Option<&'a str>,
    /// Written to the card's `reason` when set.
    pub reason: Option<&'a str>,
    /// The task's `failure` on deny. `None`: 「你拒绝了：…」.
    pub failure: Option<&'a str>,
    /// Only settle a card in this review state (`pending` for auto-review).
    /// `None`: only a card the owner can see.
    pub from_review: Option<&'a str>,
    /// New review state, when set.
    pub to_review: Option<&'a str>,
}

impl Default for Settlement<'_> {
    fn default() -> Self {
        Self {
            decision: ApprovalDecision::Denied,
            by: None,
            reason: None,
            failure: None,
            from_review: None,
            to_review: None,
        }
    }
}

/// Answer one approval card and move what it gates: the task (marker, then
/// `queued`, or `done` with `failure`), the MCP call, or the rule list.
/// The owner's approve and deny, auto-review's allow, and expiry all go
/// through here. `Err` is `(status, message)` and leaves the card open.
pub(crate) fn settle(
    conn: &mut Connection,
    control_root: &Path,
    id: &str,
    card: &Event,
    how: &Settlement<'_>,
) -> rusqlite::Result<Result<(), (StatusCode, &'static str)>> {
    let decision = how.decision;
    let gate = gate_of(conn, id)?;
    let mcp = is_mcp_card(card);
    // Marker first: if it can't be written nothing changes and the card stays open.
    if let (Some((task_id, hash)), ApprovalDecision::Approved) = (&gate, decision) {
        if mcp {
            if let Err(err) = crate::mcp_tools::mark_approved(&control_root.join(task_id)) {
                eprintln!("approve {id}: mcp pending for {task_id}: {err}");
                return Ok(Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    robroz_contract::host_text::APPROVE_CONTINUE_MARKER_FAILED,
                )));
            }
        } else if let Err(err) = robroz_runner::append_takeover(&control_root.join(task_id), hash) {
            eprintln!("approve {id}: takeover for {task_id}: {err}");
            return Ok(Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                robroz_contract::host_text::APPROVE_TAKEOVER_MARKER_FAILED,
            )));
        }
    }
    let now = now_rfc3339();
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE approvals SET decision = ?1, answered_at = ?2, answered_by = ?4,
                              reason = COALESCE(?5, reason), review_state = COALESCE(?7, review_state)
         WHERE id = ?3 AND decision IS NULL
           AND ((?6 IS NULL AND (review_state IS NULL OR review_state = 'blocked')) OR review_state = ?6)",
        params![
            decision_text(decision),
            now,
            id,
            how.by,
            how.reason,
            how.from_review,
            how.to_review
        ],
    )?;
    if changed == 0 {
        return Ok(Err((
            StatusCode::CONFLICT,
            robroz_contract::host_text::ALREADY_ANSWERED,
        )));
    }
    if let Some((task_id, hash)) = &gate {
        if mcp {
            match decision {
                ApprovalDecision::Approved => {
                    crate::mcp_tools::queue_approved(&tx, task_id, hash, &now)?;
                }
                ApprovalDecision::Denied if crate::router::clear(&tx, task_id)? => {
                    // A split task ends whole: 「没写出结果」, no 「未执行」 delivery.
                    crate::router::end_denied(&tx, task_id, hash, &now)?;
                    let _ = crate::mcp_tools::remove_pending(&control_root.join(task_id));
                }
                ApprovalDecision::Denied => {
                    let failure = match how.failure {
                        Some(f) => f.to_string(),
                        None => {
                            let line = match card {
                                Event::Approval {
                                    mcp_server: Some(s),
                                    mcp_tool: Some(tool),
                                    ..
                                } => format!("{s}.{tool}"),
                                Event::Approval { summary, .. } => summary.clone(),
                                _ => robroz_contract::host_text::DENIED_MCP_FALLBACK.into(),
                            };
                            format!("{}{line}", robroz_contract::host_text::DENIED_PREFIX)
                        }
                    };
                    crate::mcp_tools::finish_denied(&tx, task_id, hash, &failure, &now)?;
                    let _ = crate::mcp_tools::remove_pending(&control_root.join(task_id));
                }
            }
        } else {
            settle_task(&tx, task_id, hash, decision, how.failure)?;
        }
    }
    if decision == ApprovalDecision::Approved {
        crate::review::rules::apply_confirmed(&tx, id, &now)?;
    }
    tx.commit()?;
    Ok(Ok(()))
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
        let Some(card) = load_settled_or_open(&conn, id)? else {
            return Ok(Err(error_response(
                StatusCode::NOT_FOUND,
                robroz_contract::host_text::APPROVAL_NOT_FOUND,
            )));
        };
        if matches!(
            card,
            Event::Approval {
                decision: Some(_),
                ..
            }
        ) {
            return Ok(Err(error_response(
                StatusCode::CONFLICT,
                robroz_contract::host_text::ALREADY_ANSWERED,
            )));
        }
        let settled = settle(
            &mut conn,
            &state.control_root,
            id,
            &card,
            &Settlement {
                decision,
                by: by.as_deref(),
                ..Settlement::default()
            },
        )?;
        if let Err((status, message)) = settled {
            return Ok(Err(error_response(status, message)));
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
    let answer: Answer = serde_json::from_slice(body).map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            robroz_contract::host_text::BAD_REQUEST_FORMAT,
        )
    })?;
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

    /// `SELECT` spells the settings card's bot name in SQL; keep it the contract's.
    #[test]
    fn select_names_settings_cards_with_the_contract_text() {
        assert!(super::SELECT.contains(&format!(
            "a.bot_id = '{}' THEN '{}'",
            robroz_contract::SETTINGS_APPROVAL_BOT_ID,
            robroz_contract::SETTINGS_APPROVAL_BOT_NAME
        )));
    }
}
