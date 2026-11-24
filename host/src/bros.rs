//! Bros and private chats.
//!
//! A bro is a `bots` row. A private chat (DM) is an ordinary group whose only
//! member is that bro, marked by `groups.dm_bot_id`, so messages, tasks,
//! events, the stream and unread all work on it unchanged. No new concept.
//!
//! - `POST  /api/bros`          `{name?}` → new bro + its DM. Default `bro`, `bro 2`…
//! - `GET   /api/bros`          all bros with their DM group id
//! - `PATCH /api/bros/{id}`     `{name}` rename; the DM's name follows
//! - `POST  /api/bros/{id}/dm`  get or create the DM for an existing bro
//!
//! A DM keeps exactly one member: adding another bro to it is refused (409).

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{next_bro_name, Bro, Bros, CreateBro, RenameBro, BRO_NAME_MAX};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, error_response, lock_db, new_id, now_rfc3339, AppState, TENANT_ID};

/// Fresh databases get the column from here too; duplicate-column errors are ignored.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE groups ADD COLUMN dm_bot_id TEXT", []);
    conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS groups_dm_bot ON groups(dm_bot_id) WHERE dm_bot_id IS NOT NULL;")
}

/// `Some(bot)` when `group_id` is a DM.
pub(crate) fn dm_owner(conn: &Connection, group_id: &str) -> rusqlite::Result<Option<String>> {
    Ok(conn
        .query_row("SELECT dm_bot_id FROM groups WHERE id = ?1", [group_id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

fn clean_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("请填写名称");
    }
    if name.chars().count() > BRO_NAME_MAX {
        return Err("名字太长");
    }
    Ok(name.to_string())
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Bro>> {
    conn.query_row(
        "SELECT b.id, b.name, g.id FROM bots b LEFT JOIN groups g ON g.dm_bot_id = b.id WHERE b.id = ?1",
        [id],
        |r| Ok(Bro { id: r.get(0)?, name: r.get(1)?, dm_group_id: r.get(2)? }),
    )
    .optional()
}

pub(crate) fn ensure_dm(conn: &Connection, bot_id: &str, name: &str) -> rusqlite::Result<bool> {
    if conn
        .query_row("SELECT 1 FROM groups WHERE dm_bot_id = ?1", [bot_id], |_| Ok(()))
        .optional()?
        .is_some()
    {
        return Ok(false);
    }
    let group_id = new_id();
    conn.execute(
        "INSERT INTO groups (id, tenant_id, name, created_at, dm_bot_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![group_id, TENANT_ID, name, now_rfc3339(), bot_id],
    )?;
    conn.execute("INSERT INTO memberships (group_id, bot_id) VALUES (?1, ?2)", params![group_id, bot_id])?;
    Ok(true)
}

pub(crate) async fn create_bro(State(state): State<AppState>, body: Option<Json<CreateBro>>) -> Response {
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
                let taken = stmt.query_map([TENANT_ID], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                next_bro_name(taken.iter().map(String::as_str))
            }
        };
        let id = new_id();
        tx.execute("INSERT INTO bots (id, tenant_id, name) VALUES (?1, ?2, ?3)", params![id, TENANT_ID, name])?;
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

pub(crate) async fn list_bros(State(state): State<AppState>) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let mut stmt = conn.prepare(
            "SELECT b.id, b.name, g.id FROM bots b LEFT JOIN groups g ON g.dm_bot_id = b.id
             WHERE b.tenant_id = ?1 ORDER BY b.rowid",
        )?;
        let bros = stmt
            .query_map([TENANT_ID], |r| Ok(Bro { id: r.get(0)?, name: r.get(1)?, dm_group_id: r.get(2)? }))?
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
        if tx.execute("UPDATE bots SET name = ?2 WHERE id = ?1", params![id, name])? == 0 {
            return Ok(None);
        }
        tx.execute("UPDATE groups SET name = ?2 WHERE dm_bot_id = ?1", params![id, name])?;
        let bro = load(&tx, &id)?;
        tx.commit()?;
        Ok::<_, rusqlite::Error>(bro)
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "bro 不存在"),
        Ok(Some(bro)) => (StatusCode::OK, Json(bro)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn open_dm(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let tx = conn.unchecked_transaction()?;
        let Some(bro) = load(&tx, &id)? else { return Ok(None) };
        let created = ensure_dm(&tx, &bro.id, &bro.name)?;
        let bro = load(&tx, &id)?.expect("exists");
        tx.commit()?;
        Ok::<_, rusqlite::Error>(Some((created, bro)))
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "bro 不存在"),
        Ok(Some((created, bro))) => {
            (if created { StatusCode::CREATED } else { StatusCode::OK }, Json(bro)).into_response()
        }
        Err(err) => db_error(err),
    }
}
