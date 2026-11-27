//! Group management: members, rename, delete and search.
//!
//! - `GET    /api/groups/{id}/members`          bros and their owners (and the creator)
//! - `PATCH  /api/groups/{id}`                  `{name}` rename
//! - `DELETE /api/groups/{id}`                  delete the group and everything in it
//! - `GET    /api/groups/{id}/search?q=&limit=` full-text search over its messages
//!
//! Users are not members of a group on their own: a user has a part in a group
//! when they created it, own a bro in it, are its DM user, or are an admin.
//! Members and search need that ([`NOT_GROUP_MEMBER`]); rename and delete
//! need the creator or an admin ([`NOT_GROUP_CREATOR`]). A DM is never renamed
//! or deleted here ([`GROUP_IS_DM`]): its name follows the bro.
//!
//! Delete stops the group's running containers (through [`REAPS`], which the
//! pump drains), expires its open approval cards, removes its rows in one
//! transaction, and after commit removes its files and stops its pooled MCP
//! processes. Bros, their memories and skills, and MCP server configs stay.
//! Approval cards of the group that the owner's DM shows as reminders are kept
//! so the DM can show them as expired; [`TOMBSTONES`] keeps the group's name
//! for them.

use std::collections::BTreeMap;
use std::path::Path;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use robroz_contract::host_text::{GROUP_IS_DM, NOT_GROUP_CREATOR, NOT_GROUP_MEMBER};
use robroz_contract::{
    Group, GroupMemberBro, GroupMemberUser, GroupMembers, GroupSearch, GroupSearchHit, RenameGroup,
    APPROVAL_EXPIRED_REASON, GROUP_SEARCH_DEFAULT_LIMIT, GROUP_SEARCH_MAX_LIMIT,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use crate::extract::{Json, Path as AxumPath, Query};
use crate::users::{CurrentUser, OWNER_SQL};
use crate::{db_error, error_response, lock_db, now_rfc3339, AppState};

/// Containers of deleted tasks that the pump still has to stop.
pub(crate) const REAPS: &str = "
    CREATE TABLE IF NOT EXISTS container_reaps (
        task_id TEXT PRIMARY KEY,
        created_at TEXT NOT NULL
    );
";

/// Deleted groups: only the name, for approval reminders that outlive them.
pub(crate) const TOMBSTONES: &str = "
    CREATE TABLE IF NOT EXISTS group_tombstones (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        deleted_at TEXT NOT NULL
    );
";

/// Full-text index over `messages.body` (trigram, so Chinese works without
/// word splitting), kept in sync by triggers.
const FTS: &str = "
    CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
        body, content='messages', content_rowid='rowid', tokenize='trigram'
    );
    CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
        INSERT INTO messages_fts(rowid, body) VALUES (new.rowid, new.body);
    END;
    CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
        INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.rowid, old.body);
    END;
    CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE OF body ON messages BEGIN
        INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.rowid, old.body);
        INSERT INTO messages_fts(rowid, body) VALUES (new.rowid, new.body);
    END;
";

/// Additive. `groups.created_by` stays `NULL` for older groups (admin only).
/// The search index is built from existing messages once, when it is created.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE groups ADD COLUMN created_by TEXT", []);
    conn.execute_batch(REAPS)?;
    conn.execute_batch(TOMBSTONES)?;
    let had_index: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'messages_fts')",
        [],
        |r| r.get(0),
    )?;
    conn.execute_batch(FTS)?;
    if !had_index {
        conn.execute(
            "INSERT INTO messages_fts(messages_fts) VALUES ('rebuild')",
            [],
        )?;
    }
    Ok(())
}

/// `(name, created_by, dm_user_id, is_dm)` of a group.
struct GroupRow {
    name: String,
    created_by: Option<String>,
    dm_user_id: Option<String>,
    is_dm: bool,
}

fn group_row(conn: &Connection, id: &str) -> rusqlite::Result<Option<GroupRow>> {
    conn.query_row(
        "SELECT name, created_by, dm_user_id, dm_bot_id IS NOT NULL FROM groups WHERE id = ?1",
        [id],
        |r| {
            Ok(GroupRow {
                name: r.get(0)?,
                created_by: r.get(1)?,
                dm_user_id: r.get(2)?,
                is_dm: r.get(3)?,
            })
        },
    )
    .optional()
}

/// Whether `user` has a part in the group (see the module doc).
fn can_see(
    conn: &Connection,
    id: &str,
    g: &GroupRow,
    user: &CurrentUser,
) -> rusqlite::Result<bool> {
    if g.is_dm {
        return Ok(g.dm_user_id.as_deref() == Some(user.0.as_str()));
    }
    if g.created_by.as_deref() == Some(user.0.as_str()) || crate::users::is_admin(conn, &user.0)? {
        return Ok(true);
    }
    conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM memberships m JOIN bots b ON b.id = m.bot_id
                           WHERE m.group_id = ?1 AND {OWNER_SQL} = ?2)"
        ),
        params![id, user.0],
        |r| r.get(0),
    )
}

/// `Ok(())` when `user` may rename or delete the group.
fn may_manage(
    conn: &Connection,
    g: &GroupRow,
    user: &CurrentUser,
) -> rusqlite::Result<Result<(), &'static str>> {
    if g.is_dm {
        return Ok(Err(GROUP_IS_DM));
    }
    if g.created_by.as_deref() == Some(user.0.as_str()) || crate::users::is_admin(conn, &user.0)? {
        return Ok(Ok(()));
    }
    Ok(Err(NOT_GROUP_CREATOR))
}

fn not_found() -> Response {
    error_response(
        StatusCode::NOT_FOUND,
        robroz_contract::host_text::GROUP_NOT_FOUND,
    )
}

pub(crate) async fn list_members(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let Some(g) = group_row(&conn, &id)? else {
            return Ok(None);
        };
        if !can_see(&conn, &id, &g, &user)? {
            return Ok(Some(Err(())));
        }
        let mut stmt = conn.prepare(&format!(
            "SELECT b.id, b.name, {OWNER_SQL}, u.name FROM memberships m
             JOIN bots b ON b.id = m.bot_id
             LEFT JOIN users u ON u.id = {OWNER_SQL}
             WHERE m.group_id = ?1 ORDER BY m.rowid"
        ))?;
        let bros = stmt
            .query_map([&id], |r| {
                Ok(GroupMemberBro {
                    bot_id: r.get(0)?,
                    name: r.get(1)?,
                    owner_user_id: r.get(2)?,
                    owner_name: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // creator, DM user and bro owners, each once.
        let mut users: BTreeMap<String, GroupMemberUser> = BTreeMap::new();
        let creator = if g.is_dm { None } else { g.created_by.clone() };
        for uid in creator.iter().chain(g.dm_user_id.iter()) {
            users.entry(uid.clone()).or_insert_with(|| GroupMemberUser {
                user_id: uid.clone(),
                name: None,
                creator: false,
                bro_ids: Vec::new(),
            });
        }
        for bro in &bros {
            if let Some(owner) = &bro.owner_user_id {
                users
                    .entry(owner.clone())
                    .or_insert_with(|| GroupMemberUser {
                        user_id: owner.clone(),
                        name: None,
                        creator: false,
                        bro_ids: Vec::new(),
                    })
                    .bro_ids
                    .push(bro.bot_id.clone());
            }
        }
        let mut users: Vec<GroupMemberUser> = users
            .into_values()
            .map(|mut u| {
                u.creator = creator.as_deref() == Some(u.user_id.as_str());
                u.name = crate::users::load(&conn, &u.user_id)
                    .ok()
                    .flatten()
                    .map(|(name, _)| name);
                u
            })
            .collect();
        users.sort_by(|a, b| {
            b.creator
                .cmp(&a.creator)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.user_id.cmp(&b.user_id))
        });
        Ok(Some(Ok(GroupMembers {
            group_id: id.clone(),
            created_by: creator,
            bros,
            users,
        })))
    })();
    drop(conn);
    match result {
        Ok(None) => not_found(),
        Ok(Some(Err(()))) => error_response(StatusCode::FORBIDDEN, NOT_GROUP_MEMBER),
        Ok(Some(Ok(body))) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn rename_group(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<RenameGroup>,
) -> Response {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            robroz_contract::host_text::NEED_NAME,
        );
    }
    let conn = lock_db(&state);
    let result = (|| {
        let Some(g) = group_row(&conn, &id)? else {
            return Ok(None);
        };
        if let Err(msg) = may_manage(&conn, &g, &user)? {
            return Ok(Some(Err(msg)));
        }
        conn.execute(
            "UPDATE groups SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        let tenant_id: String =
            conn.query_row("SELECT tenant_id FROM groups WHERE id = ?1", [&id], |r| {
                r.get(0)
            })?;
        Ok(Some(Ok(Group {
            id: id.clone(),
            name: name.clone(),
            tenant_id,
            created_by: g.created_by,
            dm: g.is_dm,
        })))
    })();
    drop(conn);
    match result {
        Ok(None) => not_found(),
        Ok(Some(Err(msg))) => error_response(StatusCode::FORBIDDEN, msg),
        Ok(Some(Ok(group))) => (StatusCode::OK, Json(group)).into_response(),
        Err(err) => db_error(err),
    }
}

/// What [`delete_rows`] found, for the work after commit.
#[derive(Debug, Default)]
pub(crate) struct Deleted {
    pub task_ids: Vec<String>,
}

/// Remove the group's rows (see the module doc). Runs in the caller's
/// transaction. Tasks with a live container are queued in `container_reaps`.
pub(crate) fn delete_rows(conn: &Connection, id: &str, name: &str) -> rusqlite::Result<Deleted> {
    let now = now_rfc3339();
    let task_ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM tasks WHERE group_id = ?1")?;
        let rows = stmt.query_map([id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    conn.execute(
        "INSERT OR IGNORE INTO container_reaps (task_id, created_at)
         SELECT id, ?2 FROM tasks WHERE group_id = ?1 AND (container_up = 1 OR state = 'running')",
        params![id, now],
    )?;
    // Open bro cards expire like any card past `expiresAt`, so the owner's DM
    // reminder shows 「已过期」. Cards shown as reminders are kept for the DM.
    conn.execute(
        "UPDATE approvals SET decision = 'denied', answered_at = ?2, reason = ?3, reason_code = ?4
         WHERE group_id = ?1 AND decision IS NULL",
        params![
            id,
            now,
            APPROVAL_EXPIRED_REASON,
            robroz_contract::ApprovalReasonCode::Expired.as_str()
        ],
    )?;
    conn.execute(
        "DELETE FROM approvals WHERE group_id = ?1
           AND NOT (subject_kind IS NULL AND (review_state IS NULL OR review_state = 'blocked'))",
        [id],
    )?;
    conn.execute(
        "UPDATE approvals SET task_id = NULL, intent_hash = NULL WHERE group_id = ?1",
        [id],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO group_tombstones (id, name, deleted_at) VALUES (?1, ?2, ?3)",
        params![id, name, now],
    )?;
    for table in ["follow_ups", "router_plans", "step_clocks"] {
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE task_id IN (SELECT id FROM tasks WHERE group_id = ?1)"
            ),
            [id],
        )?;
    }
    for table in [
        "turns",
        "messages",
        "deliveries",
        "tasks",
        "schedules",
        "choices",
        "group_reads",
        "group_skills",
        "group_mcp",
        "memberships",
        "group_changes",
    ] {
        conn.execute(&format!("DELETE FROM {table} WHERE group_id = ?1"), [id])?;
    }
    conn.execute("DELETE FROM groups WHERE id = ?1", [id])?;
    Ok(Deleted { task_ids })
}

/// A path segment we are willing to `remove_dir_all` under a root.
fn safe_segment(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// After commit: the group's volume (results and task files) and the
/// deleted tasks' control dirs. Errors are logged, never returned.
pub(crate) fn remove_files(volumes_root: &Path, control_root: &Path, id: &str, d: &Deleted) {
    if safe_segment(id) {
        let dir = volumes_root.join(id);
        if dir.exists() {
            if let Err(err) = std::fs::remove_dir_all(&dir) {
                eprintln!("delete group {id}: remove {}: {err}", dir.display());
            }
        }
    }
    for task in d.task_ids.iter().filter(|t| safe_segment(t)) {
        let dir = control_root.join(task);
        if dir.exists() {
            if let Err(err) = std::fs::remove_dir_all(&dir) {
                eprintln!("delete group {id}: remove {}: {err}", dir.display());
            }
        }
    }
}

pub(crate) async fn delete_group(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let tx = conn.unchecked_transaction()?;
        let Some(g) = group_row(&tx, &id)? else {
            return Ok(None);
        };
        if let Err(msg) = may_manage(&tx, &g, &user)? {
            return Ok(Some(Err(msg)));
        }
        let deleted = delete_rows(&tx, &id, &g.name)?;
        tx.commit()?;
        Ok(Some(Ok(deleted)))
    })();
    drop(conn);
    match result {
        Ok(None) => not_found(),
        Ok(Some(Err(msg))) => error_response(StatusCode::FORBIDDEN, msg),
        Ok(Some(Ok(deleted))) => {
            remove_files(&state.volumes_root, &state.control_root, &id, &deleted);
            state.mcp.evict_group(&id);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(err) => db_error(err),
    }
}

/// Task ids whose containers still have to be stopped.
pub(crate) fn pending_reaps(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT task_id FROM container_reaps ORDER BY created_at")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

pub(crate) fn reaped(conn: &Connection, task_id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM container_reaps WHERE task_id = ?1", [task_id])?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Shortest query the trigram index can match; shorter ones use `LIKE`.
const TRIGRAM: usize = 3;
/// Characters of context on each side of a `LIKE` match.
const SNIPPET_CONTEXT: usize = 24;

/// Plain text around the first case-insensitive match of `q` in `body`.
fn like_snippet(body: &str, q: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let lower: Vec<char> = body.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = q.chars().flat_map(char::to_lowercase).collect();
    // Lower-casing can change lengths; fall back to the start then.
    let at = if lower.len() == chars.len() {
        lower
            .windows(needle.len().max(1))
            .position(|w| w == needle.as_slice())
            .unwrap_or(0)
    } else {
        0
    };
    let start = at.saturating_sub(SNIPPET_CONTEXT);
    let end = (at + needle.len() + SNIPPET_CONTEXT).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

fn search_rows(
    conn: &Connection,
    group_id: &str,
    q: &str,
    limit: usize,
) -> rusqlite::Result<Vec<GroupSearchHit>> {
    let hit = |r: &rusqlite::Row<'_>| {
        Ok(GroupSearchHit {
            message_id: r.get(0)?,
            snippet: r.get(1)?,
            created_at: r.get(2)?,
            author_user_id: r.get(3)?,
            author_name: r.get(4)?,
        })
    };
    if q.chars().count() >= TRIGRAM {
        // The whole query is one phrase: a substring match, ranked by bm25.
        let phrase = format!("\"{}\"", q.replace('"', "\"\""));
        let mut stmt = conn.prepare(
            "SELECT m.id, snippet(messages_fts, 0, '', '', '…', 16), m.created_at,
                    m.author_user_id, u.name
             FROM messages_fts f
             JOIN messages m ON m.rowid = f.rowid
             LEFT JOIN users u ON u.id = m.author_user_id
             WHERE messages_fts MATCH ?1 AND m.group_id = ?2
             ORDER BY bm25(messages_fts), m.created_at DESC, m.rowid DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![phrase, group_id, limit as i64], hit)?;
        return rows.collect();
    }
    let pattern = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut stmt = conn.prepare(
        "SELECT m.id, m.body, m.created_at, m.author_user_id, u.name
         FROM messages m LEFT JOIN users u ON u.id = m.author_user_id
         WHERE m.group_id = ?1 AND m.body LIKE ?2 ESCAPE '\\'
         ORDER BY m.created_at DESC, m.rowid DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![group_id, pattern, limit as i64], hit)?;
    rows.map(|r| {
        r.map(|mut h| {
            h.snippet = like_snippet(&h.snippet, q);
            h
        })
    })
    .collect()
}

pub(crate) async fn search_group(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let q = query.q.as_deref().unwrap_or("").trim().to_string();
    if q.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            robroz_contract::host_text::NEED_TEXT,
        );
    }
    let limit = query
        .limit
        .unwrap_or(GROUP_SEARCH_DEFAULT_LIMIT)
        .clamp(1, GROUP_SEARCH_MAX_LIMIT);
    let conn = lock_db(&state);
    let result = (|| {
        let Some(g) = group_row(&conn, &id)? else {
            return Ok(None);
        };
        if !can_see(&conn, &id, &g, &user)? {
            return Ok(Some(Err(())));
        }
        Ok(Some(Ok(search_rows(&conn, &id, &q, limit)?)))
    })();
    drop(conn);
    match result {
        Ok(None) => not_found(),
        Ok(Some(Err(()))) => error_response(StatusCode::FORBIDDEN, NOT_GROUP_MEMBER),
        Ok(Some(Ok(results))) => (StatusCode::OK, Json(GroupSearch { results })).into_response(),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests;
