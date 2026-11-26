//! Allow rules for auto-review.
//!
//! - `GET    /api/settings/review-rules`       confirmed rules only, oldest first
//! - `POST   /api/settings/review-rules`       `{text}` → 202 `{approvalId}`, a `rule_add` card
//! - `DELETE /api/settings/review-rules/{id}`  → 202 `{approvalId}`, a `rule_delete` card
//!
//! `POST` and `DELETE` never change the list. They raise a confirm card that
//! is shown on the settings page itself, not in any chat: the client fetches
//! it with `GET /api/approvals/{id}` and answers it with the existing
//! `POST /api/approvals/{id}/approve` or `/deny`. No bro is needed. Approve applies the change
//! ([`apply_confirmed`]); deny or expiry discards it. The pending text and
//! the rule id are stored on the approval row (`summary`, `rule_id`).

use crate::extract::Json;
use crate::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use robroz_contract::{
    CreateReviewRule, ReviewRule, ReviewRuleCreated, ReviewRules, REVIEW_RULE_MAX,
    SETTINGS_APPROVAL_BOT_ID,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, error_response, lock_db, new_id, now_rfc3339, AppState};

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
        return Err(robroz_contract::host_text::NEED_RULE);
    }
    if text.contains('\n') || text.contains('\r') || text.chars().count() > REVIEW_RULE_MAX {
        return Err(robroz_contract::host_text::RULE_ONE_LINE);
    }
    Ok(text.to_string())
}

/// `approvals.group_id` of a settings-page rule card. Group ids are UUIDs,
/// so no group's events or stream ever match it.
pub(crate) const SETTINGS_GROUP: &str = SETTINGS_APPROVAL_BOT_ID;

/// Insert a `rule_add` / `rule_delete` card raised from the settings page.
/// It belongs to no bro and no chat (`botId` is `SETTINGS_APPROVAL_BOT_ID`).
fn raise(
    conn: &Connection,
    action: &str,
    text: &str,
    rule_id: Option<&str>,
) -> rusqlite::Result<String> {
    let id = new_id();
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO approvals (id, group_id, bot_id, action, summary, path, decision, created_at, answered_at, rule_id, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, NULL, ?7, ?8)",
        params![
            id,
            SETTINGS_GROUP,
            SETTINGS_APPROVAL_BOT_ID,
            action,
            text,
            now,
            rule_id,
            super::expires_at(&now, super::approval_ttl())
        ],
    )?;
    Ok(id)
}

fn accepted(id: String) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(ReviewRuleCreated { approval_id: id }),
    )
        .into_response()
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
        Ok(id) => accepted(id),
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
            .query_row(
                "SELECT text FROM review_rules WHERE id = ?1",
                [&rule_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(text) = text else {
            return Ok(Err(error_response(
                StatusCode::NOT_FOUND,
                robroz_contract::host_text::RULE_NOT_FOUND,
            )));
        };
        Ok(Ok(raise(&conn, "rule_delete", &text, Some(&rule_id))?))
    })();
    match result {
        Ok(Ok(id)) => accepted(id),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

/// Called inside the approve transaction: a `rule_add` card adds its rule,
/// a `rule_delete` card removes its rule. Any other card: nothing.
pub(crate) fn apply_confirmed(
    conn: &Connection,
    approval_id: &str,
    now: &str,
) -> rusqlite::Result<()> {
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
