//! Users (v0.2): the minimum needed to tell who owns a bro and who may
//! answer its cards. No user or token management API yet.
//!
//! - `users`: `id`, `name`, `role` (`admin` / `member`), `created_at`.
//! - The migration creates one bootstrap admin ([`BOOTSTRAP_ADMIN_ID`]).
//! - Every request is that admin unless something already put a
//!   [`CurrentUser`] on it: the env token (`ROBROZ_TOKEN` /
//!   `ROBROZ_TOKEN_FILE`) and loopback without a token both resolve to the
//!   bootstrap admin, exactly as v0.1 behaved for its single person.
//!   Request extensions are never read from the wire, so only in-process
//!   code (tests today, per-user tokens later) can set another user.
//! - `GET /api/me` returns the caller.

use crate::extract::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use robroz_contract::{Me, UserRole, BOOTSTRAP_ADMIN_ID, BOOTSTRAP_ADMIN_NAME};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, lock_db, now_rfc3339, AppState};

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member')),
    created_at TEXT NOT NULL
);";

/// Creates `users` and the bootstrap admin. Idempotent.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT OR IGNORE INTO users (id, name, role, created_at) VALUES (?1, ?2, 'admin', ?3)",
        params![BOOTSTRAP_ADMIN_ID, BOOTSTRAP_ADMIN_NAME, now_rfc3339()],
    )?;
    Ok(())
}

/// The authenticated user of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser(pub String);

impl CurrentUser {
    pub fn bootstrap_admin() -> Self {
        Self(BOOTSTRAP_ADMIN_ID.to_string())
    }
}

/// Puts the caller on the request. Runs after the bearer check, so with a
/// token set only a request with that token gets here.
pub(crate) async fn identify(mut request: Request, next: Next) -> Response {
    if request.extensions().get::<CurrentUser>().is_none() {
        request
            .extensions_mut()
            .insert(CurrentUser::bootstrap_admin());
    }
    next.run(request).await
}

/// `(name, role)` of a user, `None` if unknown.
pub(crate) fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<(String, UserRole)>> {
    conn.query_row("SELECT name, role FROM users WHERE id = ?1", [id], |r| {
        let role: String = r.get(1)?;
        Ok((
            r.get(0)?,
            if role == "admin" {
                UserRole::Admin
            } else {
                UserRole::Member
            },
        ))
    })
    .optional()
}

pub(crate) fn is_admin(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(matches!(load(conn, id)?, Some((_, UserRole::Admin))))
}

/// SQL for a bro's owner; rows written before v0.2 (or by tests straight
/// into `bots`) without an owner belong to the bootstrap admin.
pub(crate) const OWNER_SQL: &str = "COALESCE(b.owner_user_id, 'local-admin')";

/// The owner of bro `bot_id`, `None` if the bro is unknown.
pub(crate) fn bot_owner(conn: &Connection, bot_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        &format!("SELECT {OWNER_SQL} FROM bots b WHERE b.id = ?1"),
        [bot_id],
        |r| r.get(0),
    )
    .optional()
}

/// `(ownerUserId, ownerName)` of bro `bot_id`, both `None` if unknown.
/// `ownerName` is `None` when the user row is missing.
pub(crate) fn owner_of(
    conn: &Connection,
    bot_id: &str,
) -> rusqlite::Result<(Option<String>, Option<String>)> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {OWNER_SQL}, u.name FROM bots b
                 LEFT JOIN users u ON u.id = {OWNER_SQL}
                 WHERE b.id = ?1"
            ),
            [bot_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or((None, None)))
}

/// Test hook: add a user straight into the database.
#[cfg(test)]
pub(crate) fn insert_for_test(conn: &Connection, id: &str, name: &str, role: &str) {
    conn.execute(
        "INSERT INTO users (id, name, role, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, role, now_rfc3339()],
    )
    .expect("insert test user");
}

pub(crate) async fn me(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let conn = lock_db(&state);
    match load(&conn, &user.0) {
        Ok(Some((name, role))) => (
            StatusCode::OK,
            Json(Me {
                id: user.0,
                name,
                role,
            }),
        )
            .into_response(),
        Ok(None) => crate::forbidden(robroz_contract::NOT_BRO_OWNER, "unknown_user"),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn owner_sql_names_the_bootstrap_admin() {
        assert!(super::OWNER_SQL.contains(&format!("'{}'", robroz_contract::BOOTSTRAP_ADMIN_ID)));
    }
}
