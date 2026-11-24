//! Allow rules for auto-review.
//!
//! - `GET    /api/settings/review-rules`       confirmed rules only, oldest first
//! - `POST   /api/settings/review-rules`       `{text}` → 202 `{approvalId}`, a `rule_add` card
//! - `DELETE /api/settings/review-rules/{id}`  → 202 `{approvalId}`, a `rule_delete` card
//!
//! `POST` and `DELETE` never change the list. They raise a confirm card in
//! the owner's DM (the oldest bro's private chat), answered with the existing
//! `POST /api/approvals/{id}/approve` or `/deny`. Approve applies the change
//! ([`apply_confirmed`]); deny or expiry discards it. The pending text and
//! the rule id are stored on the approval row (`summary`, `rule_id`).

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{CreateReviewRule, ReviewRule, ReviewRuleCreated, ReviewRules, REVIEW_RULE_MAX};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, error_response, lock_db, new_id, now_rfc3339, AppState, TENANT_ID};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS review_rules (
        id TEXT PRIMARY KEY,
        text TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
";

pub(crate) fn count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM review_rules", [], |r| r.get(0))
}

/// The original text of every confirmed rule, oldest first.
pub(crate) fn texts(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    Ok(list(conn)?.into_iter().map(|r| r.text).collect())
}

fn list(conn: &Connection) -> rusqlite::Result<Vec<ReviewRule>> {
    let mut stmt =
        conn.prepare("SELECT id, text, created_at FROM review_rules ORDER BY created_at, rowid")?;
    let rows = stmt.query_map([], |r| {
        Ok(ReviewRule {
            id: r.get(0)?,
            text: r.get(1)?,
            created_at: r.get(2)?,
        })
    })?;
    rows.collect()
}

/// Trimmed, non-empty, one line, at most [`REVIEW_RULE_MAX`] characters.
pub fn validate(text: &str) -> Result<String, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("请填写规则");
    }
    if text.contains('\n') || text.contains('\r') || text.chars().count() > REVIEW_RULE_MAX {
        return Err("规则只能一行，最多 200 字");
    }
    Ok(text.to_string())
}

/// The owner's DM for confirm cards: the oldest bro's private chat.
/// `None` when there is no bro yet.
fn owner_dm(conn: &Connection) -> rusqlite::Result<Option<(String, String)>> {
    let Some((bot_id, name)) = conn
        .query_row(
            "SELECT id, name FROM bots WHERE tenant_id = ?1 ORDER BY rowid LIMIT 1",
            [TENANT_ID],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    crate::bros::ensure_dm(conn, &bot_id, &name)?;
    let group_id: String =
        conn.query_row("SELECT id FROM groups WHERE dm_bot_id = ?1", [&bot_id], |r| r.get(0))?;
    Ok(Some((group_id, bot_id)))
}

/// Insert a `rule_add` / `rule_delete` card. `None`: no bro to show it to.
fn raise(
    conn: &Connection,
    action: &str,
    text: &str,
    rule_id: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let tx = conn.unchecked_transaction()?;
    let Some((group_id, bot_id)) = owner_dm(&tx)? else {
        return Ok(None);
    };
    let id = new_id();
    let now = now_rfc3339();
    tx.execute(
        "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at, rule_id, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, NULL, ?7, ?8)",
        params![
            id,
            group_id,
            bot_id,
            action,
            text,
            now,
            rule_id,
            super::expires_at(&now, super::approval_ttl())
        ],
    )?;
    tx.commit()?;
    Ok(Some(id))
}

const NO_BRO: &str = "还没有 bro，没法发确认卡";

fn accepted(id: String) -> Response {
    (StatusCode::ACCEPTED, Json(ReviewRuleCreated { approval_id: id })).into_response()
}

pub(crate) async fn list_rules(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    match list(&conn) {
        Ok(rules) => (StatusCode::OK, Json(ReviewRules { rules })).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn create_rule(
    State(state): State<AppState>,
    Json(body): Json<CreateReviewRule>,
) -> Response {
    let text = match validate(&body.text) {
        Ok(text) => text,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    let conn = lock_db(&state);
    match raise(&conn, "rule_add", &text, None) {
        Ok(Some(id)) => accepted(id),
        Ok(None) => error_response(StatusCode::CONFLICT, NO_BRO),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn delete_rule(
    State(state): State<AppState>,
    AxumPath(rule_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let text: Option<String> = conn
            .query_row("SELECT text FROM review_rules WHERE id = ?1", [&rule_id], |r| r.get(0))
            .optional()?;
        let Some(text) = text else { return Ok(Err(error_response(StatusCode::NOT_FOUND, "规则不存在"))) };
        Ok(match raise(&conn, "rule_delete", &text, Some(&rule_id))? {
            Some(id) => Ok(id),
            None => Err(error_response(StatusCode::CONFLICT, NO_BRO)),
        })
    })();
    match result {
        Ok(Ok(id)) => accepted(id),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

/// Called inside the approve transaction: a `rule_add` card adds its rule,
/// a `rule_delete` card removes its rule. Any other card: nothing.
pub(crate) fn apply_confirmed(conn: &Connection, approval_id: &str, now: &str) -> rusqlite::Result<()> {
    let row: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT action, summary, rule_id FROM approvals WHERE id = ?1",
            [approval_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    match row {
        Some((action, text, _)) if action == "rule_add" => {
            conn.execute(
                "INSERT INTO review_rules (id, text, created_at) VALUES (?1, ?2, ?3)",
                params![new_id(), text, now],
            )?;
        }
        Some((action, _, Some(rule_id))) if action == "rule_delete" => {
            conn.execute("DELETE FROM review_rules WHERE id = ?1", [rule_id])?;
        }
        _ => {}
    }
    Ok(())
}
