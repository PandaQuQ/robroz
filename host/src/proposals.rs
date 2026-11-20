//! Bro proposals and the `confirm` cards that settle them.
//!
//! A run leaves `tasks/{taskId}/proposals.jsonl` in the group volume (the
//! runner writes it from `robroz-memory:{json}` lines). When the task is done
//! without a failure, [`ingest`] stores each good line as a `pending` memory
//! (`source = bot`) and raises one `confirm` card for it.
//!
//! A `confirm` card is a row in `approvals` with `action = 'confirm'` and
//! `subject_kind` / `subject_id` pointing at what it asks about. It is answered
//! through the approval endpoints (`POST /api/approvals/{id}/approve` keeps,
//! `/deny` rejects), once. [`settle`] is the only place that moves a proposed
//! memory out of `pending`; forgetting a pending memory closes its open card
//! as denied ([`close_for_forgotten`]).
//!
//! Skills (later) add a `subject_kind` and a [`Proposal`] variant here.

use std::path::Path;

use axum::http::StatusCode;
use robroz_contract::{
    ApprovalDecision, ConfirmSubject, Event, MemoryKind, MemoryScope, MemorySource, MemoryStatus, Proposal,
    PROPOSALS_PER_TASK_MAX,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::approvals::{decision_text, parse_decision};
use crate::{memories, new_id, TimedEvent};

/// `approvals.action` of a confirm card. Approval queries skip these rows.
pub(crate) const ACTION: &str = "confirm";

/// Columns added to `approvals`; ignore "duplicate column" on databases that have them.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    for column in ["subject_kind", "subject_id"] {
        let _ = conn.execute(&format!("ALTER TABLE approvals ADD COLUMN {column} TEXT"), []);
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS approvals_subject ON approvals(subject_kind, subject_id);")
}

/// Good lines of the run's `proposals.jsonl`, in order. Missing file: none.
/// Bad lines and unknown types are skipped.
pub(crate) fn read(group_dir: &Path, task_id: &str) -> Vec<Proposal> {
    let Ok(text) = std::fs::read_to_string(robroz_runner::proposals_path(group_dir, task_id)) else {
        return Vec::new();
    };
    text.lines().filter_map(Proposal::parse_line).collect()
}

pub(crate) fn remove(group_dir: &Path, task_id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(robroz_runner::proposals_path(group_dir, task_id)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Store a finished task's proposals: a `pending` memory and a `confirm` card
/// each. Skipped: a task that did not get done (`failure`), bad text or scope,
/// and anything already remembered (same scope, kind and text, not forgotten,
/// whatever its status, `rejected` included). At most
/// [`PROPOSALS_PER_TASK_MAX`] are taken. Returns how many were stored.
pub(crate) fn ingest(conn: &Connection, task_id: &str, proposals: &[Proposal], now: &str) -> rusqlite::Result<usize> {
    if proposals.is_empty() {
        return Ok(0);
    }
    let task: Option<(String, String, Option<String>)> = conn
        .query_row("SELECT group_id, assignee_bot_id, failure FROM tasks WHERE id = ?1", [task_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((group_id, bot_id, None)) = task else { return Ok(0) };
    let mut stored = 0;
    for proposal in proposals {
        if stored == PROPOSALS_PER_TASK_MAX {
            break;
        }
        let Proposal::Memory(memory) = proposal;
        let Ok(text) = memories::clean_text(&memory.text) else { continue };
        let scope = match memory.scope.as_deref() {
            None => MemoryScope::for_bot(&bot_id).to_string(),
            Some(raw) => match memories::checked_scope(conn, raw)? {
                Ok(scope) => scope,
                Err(_) => continue,
            },
        };
        if memories::has_live_duplicate(conn, &scope, memory.kind, &text)? {
            continue;
        }
        let memory_id = new_id();
        conn.execute(
            "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                memory_id,
                scope,
                memory.kind.as_str(),
                text,
                MemorySource::Bot.as_str(),
                MemoryStatus::Pending.as_str(),
                now
            ],
        )?;
        conn.execute(
            "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at,
                                    task_id, intent_hash, subject_kind, subject_id)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, NULL, ?7, NULL, ?8, ?9)",
            params![new_id(), group_id, bot_id, ACTION, text, now, task_id, ConfirmSubject::Memory.as_str(), memory_id],
        )?;
        stored += 1;
    }
    Ok(stored)
}

/// Whether `id` is a confirm card (answered or not).
pub(crate) fn is_confirm(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row("SELECT subject_kind IS NOT NULL FROM approvals WHERE id = ?1", [id], |r| r.get::<_, bool>(0))
        .optional()?
        .unwrap_or(false))
}

/// Answer a confirm card once and move what it points at, in one step:
/// approve keeps a memory, deny rejects it (the record stays, never used).
/// `false` when the card is not an open confirm card; nothing changes then.
/// The caller holds the transaction.
pub(crate) fn settle(
    conn: &Connection,
    card_id: &str,
    decision: ApprovalDecision,
    by: Option<&str>,
    now: &str,
) -> rusqlite::Result<bool> {
    let Some((subject, subject_id)) = conn
        .query_row(
            "SELECT subject_kind, subject_id FROM approvals
             WHERE id = ?1 AND subject_kind IS NOT NULL AND decision IS NULL",
            [card_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let changed = conn.execute(
        "UPDATE approvals SET decision = ?2, answered_at = ?3, answered_by = ?4 WHERE id = ?1 AND decision IS NULL",
        params![card_id, decision_text(decision), now, by],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    if ConfirmSubject::parse(&subject) == Some(ConfirmSubject::Memory) {
        let status = match decision {
            ApprovalDecision::Approved => MemoryStatus::Kept,
            ApprovalDecision::Denied => MemoryStatus::Rejected,
        };
        conn.execute(
            "UPDATE memories SET status = ?2, updated_at = ?3
             WHERE id = ?1 AND status = 'pending' AND forgotten_at IS NULL",
            params![subject_id, status.as_str(), now],
        )?;
    }
    Ok(true)
}

/// Forgetting a memory closes its open confirm card as denied (no
/// `answeredBy`), so no card is left asking about something gone.
pub(crate) fn close_for_forgotten(conn: &Connection, memory_id: &str, now: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE approvals SET decision = 'denied', answered_at = ?2
         WHERE subject_kind = 'memory' AND subject_id = ?1 AND decision IS NULL",
        params![memory_id, now],
    )?;
    Ok(())
}

/// Answer through `POST /api/approvals/{id}/approve|deny`. `Err` is the
/// status and message to send back.
pub(crate) fn answer(
    conn: &mut Connection,
    id: &str,
    decision: ApprovalDecision,
    by: Option<&str>,
) -> rusqlite::Result<Result<Event, (StatusCode, &'static str)>> {
    let tx = conn.transaction()?;
    if !settle(&tx, id, decision, by, &crate::now_rfc3339())? {
        return Ok(Err(if load(&tx, id)?.is_some() {
            (StatusCode::CONFLICT, "已经答过了")
        } else {
            (StatusCode::NOT_FOUND, "确认卡不存在")
        }));
    }
    tx.commit()?;
    Ok(Ok(load(conn, id)?.expect("confirm card exists")))
}

const SELECT: &str = "SELECT a.id, a.bot_id, b.name, a.subject_kind, a.subject_id, a.task_id, a.decision,
            a.created_at, a.answered_at, a.answered_by, m.kind, m.text, m.scope, m.updated_at
     FROM approvals a
     JOIN bots b ON b.id = a.bot_id
     LEFT JOIN memories m ON a.subject_kind = 'memory' AND m.id = a.subject_id";

fn bad_column(column: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, format!("unknown value {text:?}").into())
}

/// The card and when it last changed (answered, or its memory edited).
/// `None` for a subject this host does not know.
fn timed_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<TimedEvent>> {
    let subject: String = row.get(3)?;
    let Some(subject) = ConfirmSubject::parse(&subject) else { return Ok(None) };
    let id: String = row.get(0)?;
    let created_at: String = row.get(7)?;
    let answered_at: Option<String> = row.get(8)?;
    let memory_updated: Option<String> = row.get(13)?;
    let decision: Option<String> = row.get(6)?;
    let kind: Option<String> = row.get(10)?;
    let kind = kind.map(|k| MemoryKind::parse(&k).ok_or_else(|| bad_column(10, &k))).transpose()?;
    let changed_at = [Some(answered_at.clone().unwrap_or_else(|| created_at.clone())), memory_updated]
        .into_iter()
        .flatten()
        .max()
        .expect("one time at least");
    let memory = subject == ConfirmSubject::Memory;
    let event = Event::Confirm {
        id: id.clone(),
        bot_id: row.get(1)?,
        bot_name: row.get(2)?,
        subject,
        task_id: row.get(5)?,
        memory_id: if memory { row.get(4)? } else { None },
        kind,
        text: row.get(11)?,
        scope: row.get(12)?,
        slug: None,
        title: None,
        instruction: None,
        decision: decision.map(parse_decision).transpose()?,
        created_at: created_at.clone(),
        answered_at,
        answered_by: row.get(9)?,
    };
    Ok(Some(TimedEvent { created_at, changed_at, id, event }))
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    Ok(conn
        .query_row(&format!("{SELECT} WHERE a.id = ?1 AND a.subject_kind IS NOT NULL"), [id], timed_from_row)
        .optional()?
        .flatten()
        .map(|t| t.event))
}

/// Confirm cards of one group, for `/events` and the live stream.
pub(crate) fn timed_events(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<TimedEvent>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE a.group_id = ?1 AND a.subject_kind IS NOT NULL"))?;
    let rows = stmt.query_map([group_id], timed_from_row)?;
    rows.filter_map(|row| row.transpose()).collect()
}
