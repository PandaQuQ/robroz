//! Bros and private chats.
//!
//! A bro is a `bots` row. A private chat (DM) is an ordinary group whose only
//! member is that bro, marked by `groups.dm_bot_id`, so messages, tasks,
//! events, the stream and unread all work on it unchanged. No new concept.
//!
//! - `POST  /api/bros`          `{name?}` → new bro + its DM. Default `bro`, `bro 2`…
//!   Any user may create one; the caller becomes `owner_user_id`.
//! - `GET   /api/bros`          the caller's bros with their DM group id
//! - `PATCH /api/bros/{id}`     `{name}` rename; the DM's name follows. Owner only (403).
//! - `POST  /api/bros/{id}/dm`  get or create the owner's DM. Owner only (403).
//!
//! A DM keeps exactly one member: adding another bro to it is refused (409).
//! A DM is between one user and one bro (`groups.dm_user_id`, `dm_bot_id`),
//! unique per pair, and the user is always that bro's owner.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{next_bro_name, Bro, Bros, CreateBro, RenameBro, BRO_NAME_MAX};
use rusqlite::{params, Connection, OptionalExtension};

use crate::users::{CurrentUser, OWNER_SQL};
use crate::{db_error, error_response, lock_db, new_id, now_rfc3339, AppState, TENANT_ID};
use axum::Extension;
use robroz_contract::{NotBroOwner, BOOTSTRAP_ADMIN_ID, FORBIDDEN_NOT_BRO_OWNER, NOT_BRO_OWNER};

/// Fresh databases get the columns from here too; duplicate-column errors
/// are ignored. Runs after [`crate::users::migrate`]. Additive: existing
/// bros and DMs are backfilled to the bootstrap admin, and the DM index
/// moves from `dm_bot_id` alone to `(dm_user_id, dm_bot_id)`.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE groups ADD COLUMN dm_bot_id TEXT", []);
    let _ = conn.execute("ALTER TABLE groups ADD COLUMN dm_user_id TEXT", []);
    let _ = conn.execute("ALTER TABLE bots ADD COLUMN owner_user_id TEXT", []);
    // Every v0.1 message was typed by the one person: the bootstrap admin.
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN author_user_id TEXT", []);
    conn.execute(
        "UPDATE messages SET author_user_id = ?1 WHERE author_user_id IS NULL",
        [BOOTSTRAP_ADMIN_ID],
    )?;
    conn.execute(
        "UPDATE bots SET owner_user_id = ?1 WHERE owner_user_id IS NULL",
        [BOOTSTRAP_ADMIN_ID],
    )?;
    conn.execute(
        &format!(
            "UPDATE groups SET dm_user_id = (SELECT {OWNER_SQL} FROM bots b WHERE b.id = groups.dm_bot_id)
             WHERE dm_bot_id IS NOT NULL AND dm_user_id IS NULL"
        ),
        [],
    )?;
    conn.execute(
        "UPDATE groups SET dm_user_id = ?1 WHERE dm_bot_id IS NOT NULL AND dm_user_id IS NULL",
        [BOOTSTRAP_ADMIN_ID],
    )?;
    conn.execute_batch(
        "DROP INDEX IF EXISTS groups_dm_bot;
         CREATE UNIQUE INDEX IF NOT EXISTS groups_dm_user_bot ON groups(dm_user_id, dm_bot_id)
           WHERE dm_bot_id IS NOT NULL;",
    )
}

/// `Some(bot)` when `group_id` is a DM.
pub(crate) fn dm_bot(conn: &Connection, group_id: &str) -> rusqlite::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT dm_bot_id FROM groups WHERE id = ?1",
            [group_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// The owner's DM with `bot_id`, created if the bro has none yet (bots added
/// straight into a group by `botId` have no DM). `None` if the bot is unknown.
pub(crate) fn owner_dm_for(conn: &Connection, bot_id: &str) -> rusqlite::Result<Option<String>> {
    let Some(name) = conn
        .query_row("SELECT name FROM bots WHERE id = ?1", [bot_id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    else {
        return Ok(None);
    };
    ensure_dm(conn, bot_id, &name)?;
    owner_dm(conn, bot_id)
}

/// The DM between `bot_id` and its owner, if there is one. Never creates.
fn owner_dm(conn: &Connection, bot_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        &format!(
            "SELECT g.id FROM groups g JOIN bots b ON b.id = g.dm_bot_id
             WHERE g.dm_bot_id = ?1 AND g.dm_user_id = {OWNER_SQL}"
        ),
        [bot_id],
        |r| r.get(0),
    )
    .optional()
}

/// 403 for a caller who is not the bro's owner. `owner` is
/// `(ownerUserId, ownerName)` so the client can say who to ask.
pub(crate) fn not_owner(owner: (Option<String>, Option<String>)) -> Response {
    let (owner_user_id, owner_name) = owner;
    (
        StatusCode::FORBIDDEN,
        Json(NotBroOwner {
            error: NOT_BRO_OWNER.to_string(),
            code: FORBIDDEN_NOT_BRO_OWNER.to_string(),
            owner_user_id,
            owner_name,
        }),
    )
        .into_response()
}

/// `Ok(None)` if the bro is unknown, `Ok(Some(true))` if `user` owns it.
pub(crate) fn owned_by(
    conn: &Connection,
    bot_id: &str,
    user: &CurrentUser,
) -> rusqlite::Result<Option<bool>> {
    Ok(crate::users::bot_owner(conn, bot_id)?.map(|owner| owner == user.0))
}

fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(robroz_contract::host_text::NEED_NAME);
    }
    if name.chars().count() > BRO_NAME_MAX {
        return Err(robroz_contract::host_text::NAME_TOO_LONG);
    }
    Ok(name.to_string())
}

/// `SELECT` for a [`Bro`]: id, name, the owner's DM, owner.
fn bro_select() -> String {
    format!(
        "SELECT b.id, b.name, g.id, {OWNER_SQL} FROM bots b
         LEFT JOIN groups g ON g.dm_bot_id = b.id AND g.dm_user_id = {OWNER_SQL}"
    )
}

fn bro_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Bro> {
    Ok(Bro {
        id: r.get(0)?,
        name: r.get(1)?,
        dm_group_id: r.get(2)?,
        owner_user_id: r.get(3)?,
    })
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Bro>> {
    conn.query_row(
        &format!("{} WHERE b.id = ?1", bro_select()),
        [id],
        bro_from_row,
    )
    .optional()
}

/// Creates the DM between `bot_id` and its owner if it does not exist yet.
fn ensure_dm(conn: &Connection, bot_id: &str, name: &str) -> rusqlite::Result<bool> {
    if owner_dm(conn, bot_id)?.is_some() {
        return Ok(false);
    }
    let owner =
        crate::users::bot_owner(conn, bot_id)?.unwrap_or_else(|| BOOTSTRAP_ADMIN_ID.to_string());
    let group_id = new_id();
    conn.execute(
        "INSERT INTO groups (id, tenant_id, name, created_at, dm_bot_id, dm_user_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![group_id, TENANT_ID, name, now_rfc3339(), bot_id, owner],
    )?;
    conn.execute(
        "INSERT INTO memberships (group_id, bot_id) VALUES (?1, ?2)",
        params![group_id, bot_id],
    )?;
    Ok(true)
}

pub(crate) async fn create_bro(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    body: Option<Json<CreateBro>>,
) -> Response {
    let requested = body.and_then(|Json(b)| b.name);
    let requested = match requested.as_deref().map(clean_name) {
        None => None,
        Some(Ok(name)) => Some(name),
        Some(Err(msg)) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let conn = lock_db(&state);
    let result = (|| {
        let tx = conn.unchecked_transaction()?;
        let name = match requested {
            Some(name) => name,
            None => {
                let mut stmt = tx.prepare("SELECT name FROM bots WHERE tenant_id = ?1")?;
                let taken = stmt
                    .query_map([TENANT_ID], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                next_bro_name(taken.iter().map(String::as_str))
            }
        };
        let id = new_id();
        tx.execute(
            "INSERT INTO bots (id, tenant_id, name, owner_user_id) VALUES (?1, ?2, ?3, ?4)",
            params![id, TENANT_ID, name, user.0],
        )?;
        ensure_dm(&tx, &id, &name)?;
        let bro = load(&tx, &id)?.expect("just inserted");
        tx.commit()?;
        Ok::<_, rusqlite::Error>(bro)
    })();
    drop(conn);
    match result {
        Ok(bro) => (StatusCode::CREATED, Json(bro)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn list_bros(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let mut stmt = conn.prepare(&format!(
            "{} WHERE b.tenant_id = ?1 AND {OWNER_SQL} = ?2 ORDER BY b.rowid",
            bro_select()
        ))?;
        let bros = stmt
            .query_map(params![TENANT_ID, user.0], bro_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok::<_, rusqlite::Error>(Bros { bros })
    })();
    drop(conn);
    match result {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn rename_bro(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<RenameBro>,
) -> Response {
    let name = match clean_name(&body.name) {
        Ok(name) => name,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let conn = lock_db(&state);
    let result = (|| {
        let tx = conn.unchecked_transaction()?;
        match owned_by(&tx, &id, &user)? {
            None => return Ok(None),
            Some(false) => return Ok(Some(Err(crate::users::owner_of(&tx, &id)?))),
            Some(true) => {}
        }
        tx.execute("UPDATE bots SET name = ?2 WHERE id = ?1", params![id, name])?;
        tx.execute(
            "UPDATE groups SET name = ?2 WHERE dm_bot_id = ?1",
            params![id, name],
        )?;
        let bro = load(&tx, &id)?.expect("exists");
        tx.commit()?;
        Ok::<_, rusqlite::Error>(Some(Ok(bro)))
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            robroz_contract::host_text::BRO_NOT_FOUND,
        ),
        Ok(Some(Err(owner))) => not_owner(owner),
        Ok(Some(Ok(bro))) => (StatusCode::OK, Json(bro)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn open_dm(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let tx = conn.unchecked_transaction()?;
        let Some(bro) = load(&tx, &id)? else {
            return Ok(None);
        };
        if bro.owner_user_id.as_deref() != Some(user.0.as_str()) {
            return Ok(Some(Err(crate::users::owner_of(&tx, &id)?)));
        }
        let created = ensure_dm(&tx, &bro.id, &bro.name)?;
        let bro = load(&tx, &id)?.expect("exists");
        tx.commit()?;
        Ok::<_, rusqlite::Error>(Some(Ok((created, bro))))
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            robroz_contract::host_text::BRO_NOT_FOUND,
        ),
        Ok(Some(Err(owner))) => not_owner(owner),
        Ok(Some(Ok((created, bro)))) => (
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(bro),
        )
            .into_response(),
        Err(err) => db_error(err),
    }
}
