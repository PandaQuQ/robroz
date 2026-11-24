//! Auto-review: the host-side gate in front of risky actions, and approval
//! card expiry. See `docs/design/auto-review.md`.
//!
//! - [`rules`]: the owner's allow rules (`/api/settings/review-rules`). Adding
//!   or deleting one only raises a `rule_add` / `rule_delete` card; the list
//!   changes when the owner approves it.
//! - [`reviewer`]: the review model. It reads every confirmed rule and the
//!   pending action, and answers allow or block with a reason. An error is
//!   retried once, then blocks. Review never default-allows.
//! - This module: how a task's approval card is opened ([`open_gate`]), the
//!   review pass that settles cards waiting for review ([`run_pending`]), and
//!   the expiry sweep ([`expire_due`]).
//!
//! A card waiting for review has `review_state = 'pending'` and is hidden from
//! events and from the answer endpoints. Allowed: the card stays hidden
//! (`review_state = 'allowed'`, `decision = 'approved'`, the model's reason in
//! `reason`), and the task continues exactly as after a manual approve.
//! Blocked: `review_state = 'blocked'`, the card shows with `reason` and
//! `expiresAt`. Cards opened while there are no rules are blocked at once,
//! with no review call and no `reason`.

pub(crate) mod reviewer;
pub(crate) mod rules;

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use robroz_contract::{ApprovalDecision, APPROVAL_EXPIRED_REASON};
use rusqlite::{params, Connection};

use crate::approvals::{load_any, settle, Settlement};
pub use reviewer::{ModelReviewer, Reviewer, Verdict};

/// Env var for how long an approval card stays open, in seconds.
pub const ENV_APPROVAL_TTL: &str = "ROBROZ_APPROVAL_TTL_SECS";
/// Default approval card lifetime: 24 hours.
pub const DEFAULT_APPROVAL_TTL_SECS: u64 = 24 * 60 * 60;

/// `reason` when there are rules but no model to check them with.
pub const NO_MODEL_REASON: &str = robroz_contract::host_text::REVIEW_NO_MODEL_REASON;

pub(crate) const PENDING: &str = "pending";
pub(crate) const ALLOWED: &str = "allowed";
pub(crate) const BLOCKED: &str = "blocked";

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(rules::SCHEMA)?;
    // Ignore "duplicate column" on databases that have them.
    // `origin_group_id`: the group a bro posted a card in when the host moved
    // it to the owner's DM; the group still shows it, without buttons.
    for column in [
        "reason",
        "expires_at",
        "review_state",
        "rule_id",
        "origin_group_id",
    ] {
        let _ = conn.execute(
            &format!("ALTER TABLE approvals ADD COLUMN {column} TEXT"),
            [],
        );
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS approvals_open_expiry ON approvals(expires_at) WHERE decision IS NULL;
         CREATE INDEX IF NOT EXISTS approvals_review_state ON approvals(review_state) WHERE decision IS NULL;",
    )
}

/// Approval card lifetime from `ROBROZ_APPROVAL_TTL_SECS`. Unset, empty,
/// zero or not a number: 24 hours.
pub fn approval_ttl() -> Duration {
    ttl_from(std::env::var(ENV_APPROVAL_TTL).ok().as_deref())
}

fn ttl_from(value: Option<&str>) -> Duration {
    let secs = value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_APPROVAL_TTL_SECS)
        .min(i64::MAX as u64 / 1000);
    Duration::seconds(secs as i64)
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `now + ttl` in the same style as `createdAt`. A `now` that is not ours
/// falls back to the current time.
pub(crate) fn expires_at(now: &str, ttl: Duration) -> String {
    let base = DateTime::parse_from_rfc3339(now)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    rfc3339(base + ttl)
}

/// How a new task or MCP approval card starts.
pub(crate) struct Gate {
    pub review_state: &'static str,
    /// Set when the card shows right away.
    pub expires_at: Option<String>,
}

/// With confirmed rules the card waits for review (hidden). Without any,
/// nothing could allow it, so it is shown at once and starts expiring.
pub(crate) fn open_gate(conn: &Connection, now: &str) -> rusqlite::Result<Gate> {
    if rules::count(conn)? > 0 {
        Ok(Gate {
            review_state: PENDING,
            expires_at: None,
        })
    } else {
        Ok(Gate {
            review_state: BLOCKED,
            expires_at: Some(expires_at(now, approval_ttl())),
        })
    }
}

/// The reviewer to use: a test override, else the host's fixed model, else
/// the model configured in settings or env. `None` without any model.
pub(crate) fn reviewer_for(
    conn: &Connection,
    reviewer: Option<&Arc<dyn Reviewer>>,
    model: Option<&Arc<dyn robroz_model::Model>>,
) -> Option<Arc<dyn Reviewer>> {
    if let Some(r) = reviewer {
        return Some(r.clone());
    }
    if let Some(m) = model {
        return Some(Arc::new(ModelReviewer::new(m.clone())));
    }
    match crate::model_settings::live_model(conn) {
        Ok(Some(cfg)) => Some(Arc::new(ModelReviewer::new(Arc::new(
            robroz_model::OpenAiCompat::new(cfg),
        )))),
        Ok(None) => None,
        Err(err) => {
            eprintln!("review: model config: {err}");
            None
        }
    }
}

/// One card waiting for review.
struct Pending {
    id: String,
    action: String,
}

/// A bro-posted card is reviewed inside its `POST`; this pass only picks it
/// up if that request did not finish (for example the client went away).
pub(crate) const BRO_CARD_GRACE_SECS: i64 = 60;

fn pending(conn: &Connection, now: DateTime<Utc>) -> rusqlite::Result<Vec<Pending>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.action, a.summary, a.mcp_server, a.mcp_tool, t.intent_line
         FROM approvals a LEFT JOIN tasks t ON t.id = a.task_id
         WHERE a.review_state = 'pending' AND a.decision IS NULL AND a.subject_kind IS NULL
           AND (a.task_id IS NOT NULL OR a.created_at <= ?1)
         ORDER BY a.created_at, a.rowid",
    )?;
    let cutoff = rfc3339(now - Duration::seconds(BRO_CARD_GRACE_SECS));
    let rows = stmt.query_map([cutoff], |r| {
        let action: String = r.get(1)?;
        let summary: String = r.get(2)?;
        let server: Option<String> = r.get(3)?;
        let tool: Option<String> = r.get(4)?;
        let line: Option<String> = r.get(5)?;
        Ok(Pending {
            id: r.get(0)?,
            action: describe(
                &action,
                &summary,
                server.as_deref(),
                tool.as_deref(),
                line.as_deref(),
            ),
        })
    })?;
    rows.collect()
}

/// What the review model is shown about the action.
pub(crate) fn describe(
    action: &str,
    summary: &str,
    server: Option<&str>,
    tool: Option<&str>,
    line: Option<&str>,
) -> String {
    let mut text = format!("Action type: {action}\nDescription: {summary}");
    if let (Some(server), Some(tool)) = (server, tool) {
        text.push_str(&format!("\nMCP tool: {server}.{tool}"));
    }
    if let Some(line) = line.filter(|l| !l.trim().is_empty()) {
        text.push_str(&format!("\nBlocking line: {}", line.trim()));
    }
    text
}

/// Review every card waiting for review. The DB lock is only held for SQL,
/// never across a model call. `reviewer` is `None` without a model: each
/// card is blocked with [`NO_MODEL_REASON`]. Returns how many were settled.
pub fn run_pending(
    db: &Mutex<Connection>,
    control_root: &Path,
    reviewer: Option<&dyn Reviewer>,
    ttl: Duration,
    now: DateTime<Utc>,
) -> rusqlite::Result<usize> {
    let (items, rules) = {
        let conn = db.lock().expect("sqlite connection lock poisoned");
        (pending(&conn, now)?, rules::texts(&conn)?)
    };
    let mut settled = 0;
    for item in items {
        let verdict = match reviewer {
            Some(r) => reviewer::review_with_retry(r, &item.action, &rules),
            None => Verdict::Block(NO_MODEL_REASON.into()),
        };
        let mut conn = db.lock().expect("sqlite connection lock poisoned");
        if apply(&mut conn, control_root, &item.id, &verdict, ttl, now)? {
            settled += 1;
        }
    }
    Ok(settled)
}

/// Write one verdict. `false` when the card is no longer waiting for review.
pub(crate) fn apply(
    conn: &mut Connection,
    control_root: &Path,
    id: &str,
    verdict: &Verdict,
    ttl: Duration,
    now: DateTime<Utc>,
) -> rusqlite::Result<bool> {
    match verdict {
        Verdict::Allow(reason) => {
            let Some(card) = load_any(conn, id)? else {
                return Ok(false);
            };
            let settled = settle(
                conn,
                control_root,
                id,
                &card,
                &Settlement {
                    decision: ApprovalDecision::Approved,
                    reason: Some(reason),
                    from_review: Some(PENDING),
                    to_review: Some(ALLOWED),
                    ..Settlement::default()
                },
            )?;
            match settled {
                Ok(()) => {
                    eprintln!("review: {id} allowed by a rule: {reason}");
                    Ok(true)
                }
                Err((status, _)) if status == axum::http::StatusCode::CONFLICT => Ok(false),
                // The continue marker could not be written: ask the owner instead.
                Err((_, message)) => block(conn, id, message, ttl, now),
            }
        }
        Verdict::Block(reason) => block(conn, id, reason, ttl, now),
    }
}

/// Show the card to the owner with `reason`. `createdAt` becomes the time it
/// is shown, so live streams pick it up.
fn block(
    conn: &Connection,
    id: &str,
    reason: &str,
    ttl: Duration,
    now: DateTime<Utc>,
) -> rusqlite::Result<bool> {
    let at = rfc3339(now);
    let changed = conn.execute(
        "UPDATE approvals SET review_state = 'blocked', reason = ?2, created_at = ?3, expires_at = ?4
         WHERE id = ?1 AND review_state = 'pending' AND decision IS NULL",
        params![id, reason, at, rfc3339(now + ttl)],
    )?;
    if changed > 0 {
        eprintln!("review: {id} blocked: {reason}");
    }
    Ok(changed > 0)
}

/// Settle every open approval card whose `expiresAt` is at or before `now`
/// as `denied` with `reason` [`APPROVAL_EXPIRED_REASON`] and no
/// `answeredBy`. A task it gated ends `done` with `failure` 「审批已过期」.
/// Confirm cards (memory, skill) and takeover never expire. Returns the
/// expired card ids.
pub fn expire_due(
    conn: &mut Connection,
    control_root: &Path,
    now: DateTime<Utc>,
) -> rusqlite::Result<Vec<String>> {
    let due: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM approvals
             WHERE decision IS NULL AND subject_kind IS NULL AND expires_at IS NOT NULL AND expires_at <= ?1
               AND (review_state IS NULL OR review_state = 'blocked')
             ORDER BY expires_at, rowid",
        )?;
        let rows = stmt.query_map([rfc3339(now)], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut expired = Vec::new();
    for id in due {
        let Some(card) = load_any(conn, &id)? else {
            continue;
        };
        let settled = settle(
            conn,
            control_root,
            &id,
            &card,
            &Settlement {
                decision: ApprovalDecision::Denied,
                reason: Some(APPROVAL_EXPIRED_REASON),
                failure: Some(APPROVAL_EXPIRED_REASON),
                ..Settlement::default()
            },
        )?;
        if settled.is_ok() {
            expired.push(id);
        }
    }
    Ok(expired)
}

#[cfg(test)]
mod tests;
