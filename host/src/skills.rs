//! Skills: short instructions a bro can follow, stored as prompt text.
//!
//! - `GET    /api/skills?scope=&status=`  not forgotten; no `status` means
//!   `kept` + `pending` (`rejected` only when asked for). Newest `updatedAt` first.
//! - `POST   /api/skills`       `{scope, slug, title, instruction}` written by
//!   the person: `source=user`, `status=kept`. A live slug already in use is 409.
//! - `PATCH  /api/skills/{slug}`  `{title?, instruction?, scope?}`; forgotten or
//!   unknown → 404. `status` cannot change here.
//! - `DELETE /api/skills/{slug}`  forget: soft delete. Forgetting again is 204;
//!   a slug that never existed is 404. Forgetting a pending skill closes its card.
//!
//! `scope` is `user` or `bot:<botId>`, the same as memories
//! ([`memories::checked_scope`](crate::memories::checked_scope)).
//!
//! A group enables a kept skill with `PUT /api/groups/{id}/skills/{slug}` and
//! turns it off with `DELETE`. `GET /api/groups/{id}/skills` lists the
//! enabled, not-forgotten skills for that group (same [`Skills`] body as
//! `GET /api/skills`); a missing group is 404, none enabled is an empty list.
//! Only enabled, kept, in-scope skills go into the
//! model prompt ([`kept_for`] plus [`prompt_section`]). Skills are prompt text;
//! permissions stay in `robroz-policy`.
//!
//! A bro's proposals arrive as `pending` (`source=bot`) with a `confirm` card
//! (see [`crate::proposals`]); `confirmId` names that card while it is open.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{
    parse_skill_slug, CreateSkill, MemoryScope, MemorySource, MemoryStatus, Skill, Skills, UpdateSkill,
    SKILL_INSTRUCTION_MAX, SKILL_TITLE_MAX,
};
use rusqlite::{params, Connection, OptionalExtension, ToSql};

use crate::{db_error, error_response, lock_db, new_id, now_rfc3339, AppState};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS skills (
        id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,
        slug TEXT NOT NULL,
        title TEXT NOT NULL,
        instruction TEXT NOT NULL,
        source TEXT NOT NULL,
        status TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        forgotten_at TEXT
    );
    CREATE UNIQUE INDEX IF NOT EXISTS skills_slug_live ON skills(slug) WHERE forgotten_at IS NULL;
    CREATE TABLE IF NOT EXISTS group_skills (
        group_id TEXT NOT NULL,
        slug TEXT NOT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY (group_id, slug)
    );
";

/// The last column is the open confirm card of a pending skill, if any.
const SELECT: &str = "SELECT skills.id, skills.scope, skills.slug, skills.title, skills.instruction,
            skills.source, skills.status, skills.created_at, skills.updated_at,
            (SELECT a.id FROM approvals a
             WHERE a.subject_kind = 'skill' AND a.subject_id = skills.id AND a.decision IS NULL
             ORDER BY a.created_at LIMIT 1)
     FROM skills";
/// Newest change first; rowid breaks ties within the same millisecond.
const ORDER: &str = "ORDER BY skills.updated_at DESC, skills.rowid DESC";

const BAD_SCOPE: &str = "scope 只能是 user 或 bot:<botId>";
const BAD_STATUS: &str = "status 只能是 pending、kept 或 rejected";
const BAD_SLUG: &str = "slug 只能是小写字母、数字和连字符";
const SLUG_TAKEN: &str = "这个 slug 已经有了";
const NOT_FOUND: &str = "技能不存在";
const GROUP_NOT_FOUND: &str = "群不存在";
fn bad_column(column: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, format!("unknown value {text:?}").into())
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Skill> {
    let source: String = row.get(5)?;
    let status: String = row.get(6)?;
    Ok(Skill {
        id: row.get(0)?,
        scope: row.get(1)?,
        slug: row.get(2)?,
        title: row.get(3)?,
        instruction: row.get(4)?,
        source: MemorySource::parse(&source).ok_or_else(|| bad_column(5, &source))?,
        status: MemoryStatus::parse(&status).ok_or_else(|| bad_column(6, &status))?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        confirm_id: if status == MemoryStatus::Pending.as_str() { row.get(9)? } else { None },
    })
}

fn load_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<Skill>> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1 AND forgotten_at IS NULL"), [id], from_row).optional()
}

fn load_by_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Option<Skill>> {
    conn.query_row(&format!("{SELECT} WHERE slug = ?1 AND forgotten_at IS NULL"), [slug], from_row).optional()
}

/// A live (not forgotten) skill with this slug, any status.
pub(crate) fn has_live_slug(conn: &Connection, slug: &str) -> rusqlite::Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM skills WHERE slug = ?1 AND forgotten_at IS NULL", [slug], |r| r.get(0))
        .optional()?;
    Ok(found.is_some())
}

/// Live (not forgotten) skills enabled on this group, any status, newest
/// `updatedAt` first. Used by `GET /api/groups/{id}/skills`.
pub(crate) fn enabled_for(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<Skill>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT}
         JOIN group_skills g ON g.slug = skills.slug
         WHERE g.group_id = ?1 AND skills.forgotten_at IS NULL
         {ORDER}"
    ))?;
    let rows = stmt.query_map([group_id], from_row)?;
    rows.collect()
}

/// Kept, not forgotten skills a bro may use in this group: everyone's (`user`)
/// plus its own (`bot:<id>`), enabled on the group, newest `updatedAt` first.
pub(crate) fn kept_for(conn: &Connection, bot_id: &str, group_id: &str) -> rusqlite::Result<Vec<Skill>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT}
         JOIN group_skills g ON g.slug = skills.slug
         WHERE g.group_id = ?1 AND skills.scope IN ('user', ?2) AND skills.status = 'kept'
           AND skills.forgotten_at IS NULL
         ORDER BY skills.updated_at DESC, skills.rowid DESC"
    ))?;
    let own = MemoryScope::for_bot(bot_id).to_string();
    let rows = stmt.query_map(params![group_id, own], from_row)?;
    rows.collect()
}

/// Max length of the skill paragraph in the system prompt, in Unicode scalar
/// values. Over this, oldest whole skills are dropped; a skill is never cut.
pub(crate) const SKILL_PROMPT_MAX: usize = 8000;

const SKILL_PROMPT_HEADER: &str = "你会这些技能：";

fn prompt_block(skill: &Skill) -> String {
    format!("### {} ({})\n{}", skill.title, skill.slug, skill.instruction)
}

/// The paragraph to append to the system prompt, or `None` when there is
/// nothing to add. `skills` must already be newest-first, as [`kept_for`]
/// returns them. Entries that would go past `max_chars` are dropped from the
/// oldest end.
pub(crate) fn prompt_section(skills: &[Skill]) -> Option<String> {
    prompt_section_with(skills, SKILL_PROMPT_MAX)
}

pub(crate) fn prompt_section_with(skills: &[Skill], max_chars: usize) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut blocks = Vec::new();
    let mut total = SKILL_PROMPT_HEADER.chars().count();
    for skill in skills {
        let block = prompt_block(skill);
        // First block is joined by one newline; later ones by a blank line.
        let sep = if blocks.is_empty() { 1 } else { 2 };
        let add = sep + block.chars().count();
        if total + add > max_chars {
            break;
        }
        blocks.push(block);
        total += add;
    }
    if blocks.is_empty() {
        return None;
    }
    Some(format!("{SKILL_PROMPT_HEADER}\n{}", blocks.join("\n\n")))
}

pub(crate) fn clean_title(raw: &str) -> Result<String, &'static str> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("请填写标题");
    }
    if text.chars().count() > SKILL_TITLE_MAX {
        return Err("标题太长，最多 80 个字");
    }
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub(crate) fn clean_instruction(raw: &str) -> Result<String, &'static str> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("请填写说明");
    }
    if text.chars().count() > SKILL_INSTRUCTION_MAX {
        return Err("说明太长，最多 8000 个字");
    }
    Ok(text.to_string())
}

pub(crate) fn clean_slug(raw: &str) -> Result<String, &'static str> {
    parse_skill_slug(raw).map(str::to_string).ok_or(BAD_SLUG)
}

fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::ConstraintViolation))
}

fn body_error(rejection: JsonRejection) -> Response {
    let _ = rejection;
    error_response(StatusCode::BAD_REQUEST, "请求格式不正确")
}

type ListQuery = std::collections::HashMap<String, String>;

pub(crate) async fn list_skills(State(state): State<AppState>, Query(query): Query<ListQuery>) -> Response {
    let param = |key: &str| query.get(key).map(String::as_str).filter(|v| !v.is_empty());
    let mut filters = vec!["forgotten_at IS NULL".to_string()];
    let mut values: Vec<String> = Vec::new();
    if let Some(scope) = param("scope") {
        let Some(scope) = MemoryScope::parse(scope) else { return error_response(StatusCode::BAD_REQUEST, BAD_SCOPE) };
        values.push(scope.to_string());
        filters.push(format!("scope = ?{}", values.len()));
    }
    match param("status") {
        Some(status) => {
            let Some(status) = MemoryStatus::parse(status) else {
                return error_response(StatusCode::BAD_REQUEST, BAD_STATUS);
            };
            values.push(status.as_str().into());
            filters.push(format!("status = ?{}", values.len()));
        }
        None => filters.push("status IN ('kept', 'pending')".into()),
    }
    let sql = format!("{SELECT} WHERE {} {ORDER}", filters.join(" AND "));
    let conn = lock_db(&state);
    let result = (|| {
        let mut stmt = conn.prepare(&sql)?;
        let args: Vec<&dyn ToSql> = values.iter().map(|v| v as &dyn ToSql).collect();
        let skills = stmt.query_map(args.as_slice(), from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok::<_, rusqlite::Error>(Skills { skills })
    })();
    drop(conn);
    match result {
        Ok(body) => (StatusCode::OK, Json(body)).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn create_skill(
    State(state): State<AppState>,
    body: Result<Json<CreateSkill>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return body_error(rejection),
    };
    let slug = match clean_slug(&body.slug) {
        Ok(slug) => slug,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let title = match clean_title(&body.title) {
        Ok(title) => title,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let instruction = match clean_instruction(&body.instruction) {
        Ok(instruction) => instruction,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let conn = lock_db(&state);
    let result = (|| {
        let scope = match crate::memories::checked_scope(&conn, &body.scope)? {
            Ok(scope) => scope,
            Err(msg) => return Ok(Err((StatusCode::BAD_REQUEST, msg))),
        };
        let id = new_id();
        let now = now_rfc3339();
        match conn.execute(
            "INSERT INTO skills (id, scope, slug, title, instruction, source, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![id, scope, slug, title, instruction, MemorySource::User.as_str(), MemoryStatus::Kept.as_str(), now],
        ) {
            Ok(_) => {}
            Err(err) if is_unique_violation(&err) => return Ok(Err((StatusCode::CONFLICT, SLUG_TAKEN))),
            Err(err) => return Err(err),
        }
        Ok::<_, rusqlite::Error>(Ok(load_by_id(&conn, &id)?.expect("just inserted")))
    })();
    drop(conn);
    match result {
        Ok(Ok(skill)) => (StatusCode::CREATED, Json(skill)).into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn update_skill(
    State(state): State<AppState>,
    AxumPath(slug): AxumPath<String>,
    body: Result<Json<UpdateSkill>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return body_error(rejection),
    };
    if body.title.is_none() && body.instruction.is_none() && body.scope.is_none() {
        return error_response(StatusCode::BAD_REQUEST, "没有要改的内容");
    }
    let title = match body.title.as_deref().map(clean_title).transpose() {
        Ok(title) => title,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let instruction = match body.instruction.as_deref().map(clean_instruction).transpose() {
        Ok(instruction) => instruction,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };
    let Some(slug) = parse_skill_slug(&slug).map(str::to_string) else {
        return error_response(StatusCode::NOT_FOUND, NOT_FOUND);
    };
    let conn = lock_db(&state);
    let result = (|| {
        let Some(current) = load_by_slug(&conn, &slug)? else { return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND))) };
        let scope = match body.scope.as_deref() {
            None => current.scope,
            Some(raw) => match crate::memories::checked_scope(&conn, raw)? {
                Ok(scope) => scope,
                Err(msg) => return Ok(Err((StatusCode::BAD_REQUEST, msg))),
            },
        };
        let title = title.unwrap_or(current.title);
        let instruction = instruction.unwrap_or(current.instruction);
        conn.execute(
            "UPDATE skills SET scope = ?2, title = ?3, instruction = ?4, updated_at = ?5
             WHERE slug = ?1 AND forgotten_at IS NULL",
            params![slug, scope, title, instruction, now_rfc3339()],
        )?;
        Ok::<_, rusqlite::Error>(Ok(load_by_slug(&conn, &slug)?.expect("still there")))
    })();
    drop(conn);
    match result {
        Ok(Ok(skill)) => (StatusCode::OK, Json(skill)).into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn forget_skill(State(state): State<AppState>, AxumPath(slug): AxumPath<String>) -> Response {
    let Some(slug) = parse_skill_slug(&slug).map(str::to_string) else {
        return error_response(StatusCode::NOT_FOUND, NOT_FOUND);
    };
    let mut conn = lock_db(&state);
    let result = (|| {
        let conn = conn.transaction()?;
        let row: Option<(String, Option<String>)> = conn
            .query_row("SELECT id, forgotten_at FROM skills WHERE slug = ?1 ORDER BY forgotten_at IS NULL DESC, rowid DESC", [&slug], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        match row {
            None => Ok(false),
            Some((_, Some(_))) => Ok(true),
            Some((id, None)) => {
                let now = now_rfc3339();
                conn.execute("UPDATE skills SET forgotten_at = ?2 WHERE id = ?1", params![id, now])?;
                conn.execute("DELETE FROM group_skills WHERE slug = ?1", [&slug])?;
                crate::proposals::close_for_forgotten(&conn, robroz_contract::ConfirmSubject::Skill, &id, &now)?;
                conn.commit()?;
                Ok::<_, rusqlite::Error>(true)
            }
        }
    })();
    drop(conn);
    match result {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error_response(StatusCode::NOT_FOUND, NOT_FOUND),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn list_group_skills(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !crate::group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        Ok(Ok(Skills { skills: enabled_for(&conn, &group_id)? }))
    })();
    drop(conn);
    match result {
        Ok(Ok(body)) => (StatusCode::OK, Json(body)).into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn enable_skill(
    State(state): State<AppState>,
    AxumPath((group_id, slug)): AxumPath<(String, String)>,
) -> Response {
    let Some(slug) = parse_skill_slug(&slug).map(str::to_string) else {
        return error_response(StatusCode::NOT_FOUND, NOT_FOUND);
    };
    let conn = lock_db(&state);
    let result = (|| {
        if !crate::group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        if load_by_slug(&conn, &slug)?.is_none() {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        }
        conn.execute(
            "INSERT OR IGNORE INTO group_skills (group_id, slug, created_at) VALUES (?1, ?2, ?3)",
            params![group_id, slug, now_rfc3339()],
        )?;
        Ok::<_, rusqlite::Error>(Ok(()))
    })();
    drop(conn);
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn disable_skill(
    State(state): State<AppState>,
    AxumPath((group_id, slug)): AxumPath<(String, String)>,
) -> Response {
    let Some(slug) = parse_skill_slug(&slug).map(str::to_string) else {
        return error_response(StatusCode::NOT_FOUND, NOT_FOUND);
    };
    let conn = lock_db(&state);
    let result = (|| {
        if !crate::group_exists(&conn, &group_id)? {
            return Ok(Err((StatusCode::NOT_FOUND, GROUP_NOT_FOUND)));
        }
        if load_by_slug(&conn, &slug)?.is_none() {
            return Ok(Err((StatusCode::NOT_FOUND, NOT_FOUND)));
        }
        conn.execute("DELETE FROM group_skills WHERE group_id = ?1 AND slug = ?2", params![group_id, slug])?;
        Ok::<_, rusqlite::Error>(Ok(()))
    })();
    drop(conn);
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err((status, msg))) => error_response(status, msg),
        Err(err) => db_error(err),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::{enabled_for, kept_for, prompt_section, prompt_section_with, SCHEMA, SKILL_PROMPT_HEADER, SKILL_PROMPT_MAX};

    fn temp_path(ext: &str) -> PathBuf {
        std::env::temp_dir().join(format!("robroz-skill-test-{}.{ext}", uuid::Uuid::new_v4()))
    }

    fn app() -> (Router, PathBuf) {
        let db = temp_path("db");
        let volumes = temp_path("volumes");
        let router = crate::Host::open(&db, &volumes, "robroz-runner:local")
            .unwrap()
            .with_control_root(volumes.with_extension("control"))
            .router();
        (router, db)
    }

    async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
        (status, value)
    }

    async fn new_bro(app: &Router, name: &str) -> String {
        let (status, bro) = send(app, "POST", "/api/bros", Some(json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::CREATED);
        bro["id"].as_str().unwrap().to_string()
    }

    async fn post_skill(app: &Router, scope: &str, slug: &str, title: &str, instruction: &str) -> (StatusCode, Value) {
        send(
            app,
            "POST",
            "/api/skills",
            Some(json!({ "scope": scope, "slug": slug, "title": title, "instruction": instruction })),
        )
        .await
    }

    fn sample(slug: &str, title: &str, instruction: &str) -> robroz_contract::Skill {
        use robroz_contract::{MemorySource, MemoryStatus, Skill};
        Skill {
            id: slug.to_string(),
            scope: "user".into(),
            slug: slug.into(),
            title: title.into(),
            instruction: instruction.into(),
            source: MemorySource::User,
            status: MemoryStatus::Kept,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            updated_at: "2026-01-01T00:00:00.000Z".into(),
            confirm_id: None,
        }
    }

    #[tokio::test]
    async fn crud_lists_filters_and_rejects_a_taken_slug() {
        let (app, _) = app();
        let ada = new_bro(&app, "Ada").await;
        let (status, weekly) = post_skill(&app, "user", "weekly-report", "写周报", "先列三点").await;
        assert_eq!(status, StatusCode::CREATED, "{weekly}");
        assert_eq!(weekly["slug"], "weekly-report");
        assert_eq!(weekly["status"], "kept");
        assert_eq!(weekly["source"], "user");
        assert!(weekly.get("confirmId").is_none());

        let (status, _) = post_skill(&app, &format!("bot:{ada}"), "weekly-report", "另一个", "说明").await;
        assert_eq!(status, StatusCode::CONFLICT);

        let (status, ada_skill) = post_skill(&app, &format!("bot:{ada}"), "ada-notes", "Ada 的笔记", "只给 Ada").await;
        assert_eq!(status, StatusCode::CREATED, "{ada_skill}");

        let (status, body) = send(&app, "GET", "/api/skills", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 2, "newest first, both kept");
        assert_eq!(body["skills"][0]["slug"], "ada-notes");

        let (status, body) = send(&app, "GET", &format!("/api/skills?scope=bot:{ada}"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 1);
        assert_eq!(body["skills"][0]["slug"], "ada-notes");

        let (status, body) = send(&app, "PATCH", "/api/skills/weekly-report", Some(json!({ "title": "周报", "instruction": "用中文" }))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!((body["title"].as_str(), body["instruction"].as_str()), (Some("周报"), Some("用中文")));

        let (status, body) = send(&app, "PATCH", "/api/skills/weekly-report", Some(json!({}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "没有要改的内容");

        let (status, _) = send(&app, "DELETE", "/api/skills/weekly-report", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, "DELETE", "/api/skills/weekly-report", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&app, "GET", "/api/skills", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 1);
        assert_eq!(body["skills"][0]["slug"], "ada-notes");

        // A forgotten slug can be reused.
        let (status, _) = post_skill(&app, "user", "weekly-report", "写周报", "再记一次").await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, body) = send(&app, "PATCH", "/api/skills/nope", Some(json!({ "title": "x" }))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "技能不存在");
        let (status, _) = send(&app, "DELETE", "/api/skills/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_rejects_bad_fields_and_unknown_bros() {
        let (app, _) = app();
        for (body, status) in [
            (json!({ "scope": "user", "slug": "Weekly", "title": "t", "instruction": "i" }), StatusCode::BAD_REQUEST),
            (json!({ "scope": "user", "slug": "ok", "title": "  ", "instruction": "i" }), StatusCode::BAD_REQUEST),
            (json!({ "scope": "user", "slug": "ok", "title": "t", "instruction": "  " }), StatusCode::BAD_REQUEST),
            (json!({ "scope": "bot:nobody", "slug": "ok", "title": "t", "instruction": "i" }), StatusCode::BAD_REQUEST),
            (json!({ "scope": "group:g1", "slug": "ok", "title": "t", "instruction": "i" }), StatusCode::BAD_REQUEST),
            (json!({ "scope": "user", "slug": "ok", "title": "t", "instruction": "长".repeat(8001) }), StatusCode::BAD_REQUEST),
        ] {
            let (got, error) = send(&app, "POST", "/api/skills", Some(body.clone())).await;
            assert_eq!(got, status, "{body} {error}");
        }
        let (status, body) = send(&app, "GET", "/api/skills?status=maybe", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, body) = send(&app, "GET", "/api/skills?scope=group:g1", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn group_skills_list_tracks_enable_disable_forget_and_missing() {
        let (app, _) = app();
        let (status, group) = send(&app, "POST", "/api/groups", Some(json!({ "name": "工作" }))).await;
        assert_eq!(status, StatusCode::CREATED);
        let group_id = group["id"].as_str().unwrap();
        let list = format!("/api/groups/{group_id}/skills");

        let (status, body) = send(&app, "GET", &list, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 0);

        let (status, _) = post_skill(&app, "user", "weekly-report", "写周报", "先列三点").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, _) = post_skill(&app, "user", "ada-notes", "笔记", "记下").await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, body) = send(&app, "GET", &list, None).await;
        assert_eq!(status, StatusCode::OK, "not enabled yet");
        assert_eq!(body["skills"].as_array().unwrap().len(), 0);

        let (status, _) = send(&app, "PUT", &format!("/api/groups/{group_id}/skills/weekly-report"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&app, "GET", &list, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 1);
        assert_eq!(body["skills"][0]["slug"], "weekly-report");

        let (status, _) = send(&app, "PUT", &format!("/api/groups/{group_id}/skills/ada-notes"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&app, "GET", &list, None).await;
        assert_eq!(status, StatusCode::OK);
        let slugs: Vec<&str> = body["skills"].as_array().unwrap().iter().map(|s| s["slug"].as_str().unwrap()).collect();
        assert_eq!(slugs, ["ada-notes", "weekly-report"], "newest updatedAt first");

        let (status, _) = send(&app, "DELETE", &format!("/api/groups/{group_id}/skills/weekly-report"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&app, "GET", &list, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 1);
        assert_eq!(body["skills"][0]["slug"], "ada-notes");

        let (status, _) = send(&app, "DELETE", "/api/skills/ada-notes", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&app, "GET", &list, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["skills"].as_array().unwrap().len(), 0);

        let (status, body) = send(&app, "GET", "/api/groups/missing/skills", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "群不存在");
    }

    #[tokio::test]
    async fn group_enable_is_idempotent_and_missing_is_404() {
        let (app, _) = app();
        let (status, group) = send(&app, "POST", "/api/groups", Some(json!({ "name": "工作" }))).await;
        assert_eq!(status, StatusCode::CREATED);
        let group_id = group["id"].as_str().unwrap();
        let (status, _) = post_skill(&app, "user", "weekly-report", "写周报", "先列三点").await;
        assert_eq!(status, StatusCode::CREATED);

        let uri = format!("/api/groups/{group_id}/skills/weekly-report");
        let (status, _) = send(&app, "PUT", &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, "PUT", &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = send(&app, "PUT", "/api/groups/missing/skills/weekly-report", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "群不存在");
        let (status, body) = send(&app, "PUT", &format!("/api/groups/{group_id}/skills/nope"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "技能不存在");
    }

    #[tokio::test]
    async fn skills_require_the_api_token() {
        let db = temp_path("db");
        let token = crate::auth::ApiToken::parse("0123456789abcdef-good").unwrap();
        let app = crate::Host::open(&db, temp_path("volumes"), "robroz-runner:local").unwrap().with_token(token).router();
        for (method, uri) in [
            ("GET", "/api/skills"),
            ("POST", "/api/skills"),
            ("PATCH", "/api/skills/x"),
            ("DELETE", "/api/skills/x"),
            ("GET", "/api/groups/g/skills"),
            ("PUT", "/api/groups/g/skills/x"),
            ("DELETE", "/api/groups/g/skills/x"),
        ] {
            let (status, _) = send(&app, method, uri, Some(json!({}))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
    }

    #[test]
    fn prompt_section_is_newest_first_and_empty_adds_nothing() {
        assert_eq!(prompt_section(&[]), None);
        let section = prompt_section(&[
            sample("new-one", "新技能", "新说明"),
            sample("old-one", "旧技能", "旧说明"),
        ])
        .unwrap();
        assert_eq!(
            section,
            "你会这些技能：\n### 新技能 (new-one)\n新说明\n\n### 旧技能 (old-one)\n旧说明"
        );
    }

    #[test]
    fn prompt_section_drops_oldest_whole_skills_at_the_budget() {
        let newer = sample("n", "新", "N");
        let older = sample("o", "旧", "O");
        let newer_block = "### 新 (n)\nN";
        let older_block = "### 旧 (o)\nO";
        let header = SKILL_PROMPT_HEADER.chars().count();
        let one = header + 1 + newer_block.chars().count();
        let two = one + 2 + older_block.chars().count();
        assert_eq!(
            prompt_section_with(&[newer.clone(), older.clone()], two).as_deref(),
            Some("你会这些技能：\n### 新 (n)\nN\n\n### 旧 (o)\nO")
        );
        assert_eq!(
            prompt_section_with(&[newer.clone(), older.clone()], two - 1).as_deref(),
            Some("你会这些技能：\n### 新 (n)\nN")
        );
        assert_eq!(prompt_section_with(&[newer, older], one - 1), None);
        assert!(prompt_section(&[sample("s", "短", "x")]).unwrap().chars().count() <= SKILL_PROMPT_MAX);
    }

    #[test]
    fn kept_for_skips_other_bros_disabled_and_unusable_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(SCHEMA).expect("migration is idempotent");
        conn.execute_batch(
            "CREATE TABLE groups (id TEXT PRIMARY KEY);
             CREATE TABLE bots (id TEXT PRIMARY KEY, tenant_id TEXT, name TEXT);
             INSERT INTO groups (id) VALUES ('g1');
             INSERT INTO bots (id, tenant_id, name) VALUES ('ada', 'local', 'Ada'), ('bob', 'local', 'Bob');",
        )
        .unwrap();
        conn.execute_batch(crate::approvals::SCHEMA).unwrap();
        let _ = conn.execute("ALTER TABLE approvals ADD COLUMN task_id TEXT", []);
        crate::proposals::migrate(&conn).unwrap();
        let rows = [
            ("shared", "user", "kept", "shared", None, "2026-01-01T00:00:01.000Z"),
            ("ada-old", "bot:ada", "kept", "ada-old", None, "2026-01-01T00:00:00.000Z"),
            ("ada-new", "bot:ada", "kept", "ada-new", None, "2026-01-01T00:00:02.000Z"),
            ("bob", "bot:bob", "kept", "bob", None, "2026-01-01T00:00:03.000Z"),
            ("pending", "user", "pending", "pending", None, "2026-01-01T00:00:04.000Z"),
            ("rejected", "bot:ada", "rejected", "rejected", None, "2026-01-01T00:00:05.000Z"),
            ("forgotten", "user", "kept", "forgotten", Some("2026-01-02T00:00:00.000Z"), "2026-01-01T00:00:06.000Z"),
            ("off", "user", "kept", "off", None, "2026-01-01T00:00:07.000Z"),
        ];
        for (id, scope, status, slug, forgotten, at) in rows {
            conn.execute(
                "INSERT INTO skills (id, scope, slug, title, instruction, source, status, created_at, updated_at, forgotten_at)
                 VALUES (?1, ?2, ?4, ?4, ?1, 'user', ?3, ?6, ?6, ?5)",
                rusqlite::params![id, scope, status, slug, forgotten, at],
            )
            .unwrap();
        }
        for slug in ["shared", "ada-old", "ada-new", "bob", "pending", "rejected", "forgotten"] {
            conn.execute(
                "INSERT INTO group_skills (group_id, slug, created_at) VALUES ('g1', ?1, '2026-01-01T00:00:00.000Z')",
                [slug],
            )
            .unwrap();
        }
        // `off` is kept and in scope but not enabled on the group.
        let slugs = |bot: &str| kept_for(&conn, bot, "g1").unwrap().into_iter().map(|s| s.slug).collect::<Vec<_>>();
        assert_eq!(slugs("ada"), ["ada-new", "shared", "ada-old"]);
        assert_eq!(slugs("bob"), ["bob", "shared"]);
        assert_eq!(slugs("carol"), ["shared"]);
        let section = prompt_section(&kept_for(&conn, "ada", "g1").unwrap()).unwrap();
        assert!(section.contains("### ada-new (ada-new)"));
        assert!(section.contains("### shared (shared)"));
        assert!(section.contains("### ada-old (ada-old)"));
        assert!(!section.contains("bob") && !section.contains("pending") && !section.contains("rejected"));
        assert!(!section.contains("forgotten") && !section.contains("\noff") && !section.contains("(off)"));
        assert!(kept_for(&conn, "ada", "g-missing").unwrap().is_empty());
    }

    #[test]
    fn enabled_for_skips_forgotten_and_other_groups() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(crate::approvals::SCHEMA).unwrap();
        let _ = conn.execute("ALTER TABLE approvals ADD COLUMN task_id TEXT", []);
        crate::proposals::migrate(&conn).unwrap();
        // Enable a forgotten skill by leaving a leftover group_skills row.
        let rows = [
            ("live", "kept", "live", None, "2026-01-01T00:00:02.000Z"),
            ("gone", "kept", "gone", Some("2026-01-01T00:00:03.000Z"), "2026-01-01T00:00:01.000Z"),
            ("other", "kept", "other", None, "2026-01-01T00:00:04.000Z"),
        ];
        for (id, status, slug, forgotten, at) in rows {
            conn.execute(
                "INSERT INTO skills (id, scope, slug, title, instruction, source, status, created_at, updated_at, forgotten_at)
                 VALUES (?1, 'user', ?3, ?3, ?1, 'user', ?2, ?5, ?5, ?4)",
                rusqlite::params![id, status, slug, forgotten, at],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO group_skills (group_id, slug, created_at) VALUES ('g1', 'live', '2026-01-01T00:00:00.000Z')", []).unwrap();
        conn.execute("INSERT INTO group_skills (group_id, slug, created_at) VALUES ('g1', 'gone', '2026-01-01T00:00:00.000Z')", []).unwrap();
        conn.execute("INSERT INTO group_skills (group_id, slug, created_at) VALUES ('g2', 'other', '2026-01-01T00:00:00.000Z')", []).unwrap();
        let slugs = enabled_for(&conn, "g1").unwrap().into_iter().map(|s| s.slug).collect::<Vec<_>>();
        assert_eq!(slugs, ["live"]);
        assert!(enabled_for(&conn, "g-missing").unwrap().is_empty());
    }
}
