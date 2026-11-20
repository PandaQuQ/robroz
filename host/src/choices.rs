//! Choice cards: a bot in the group asks a question with fixed options and the
//! person's pick comes back as `choice`. A card is answered once.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{
    AnswerChoice, ChoiceOption, CreateChoice, Event, CHOICE_OPTIONS_MAX, CHOICE_OPTIONS_MIN,
};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, error_response, group_exists, lock_db, new_id, now_rfc3339, AppState, TimedEvent};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS choices (
        id TEXT PRIMARY KEY,
        group_id TEXT NOT NULL,
        bot_id TEXT NOT NULL,
        prompt TEXT NOT NULL,
        options TEXT NOT NULL,
        multi INTEGER NOT NULL DEFAULT 0,
        choice TEXT,
        created_at TEXT NOT NULL,
        answered_at TEXT
    );
";

const SELECT: &str = "SELECT c.id, c.bot_id, b.name, c.prompt, c.options, c.choice, c.created_at, c.answered_at, c.multi
     FROM choices c JOIN bots b ON b.id = c.bot_id";

/// Trims and checks options: 2 to 4, non-empty labels, distinct non-empty
/// values, at most one `primary`. `primary` is dropped on multi cards.
pub fn validate_options(options: &[ChoiceOption], multi: bool) -> Result<Vec<ChoiceOption>, &'static str> {
    if !(CHOICE_OPTIONS_MIN..=CHOICE_OPTIONS_MAX).contains(&options.len()) {
        return Err("选项需要 2 到 4 个");
    }
    if options.iter().filter(|o| o.primary).count() > 1 {
        return Err("推荐项最多一个");
    }
    let mut clean: Vec<ChoiceOption> = Vec::with_capacity(options.len());
    for option in options {
        let value = option.id.trim().to_string();
        let label = option.label.trim().to_string();
        if value.is_empty() || label.is_empty() {
            return Err("选项不能为空");
        }
        if clean.iter().any(|o| o.id == value) {
            return Err("选项重复");
        }
        clean.push(ChoiceOption { id: value, label, primary: option.primary && !multi });
    }
    Ok(clean)
}

/// Checks a pick against the card: single takes exactly one value, multi one
/// or more distinct values; every value must be an option. Returns it trimmed.
pub fn validate_pick(options: &[ChoiceOption], multi: bool, pick: &[String]) -> Result<Vec<String>, &'static str> {
    let pick: Vec<String> = pick.iter().map(|v| v.trim().to_string()).collect();
    if pick.is_empty() || (!multi && pick.len() != 1) {
        return Err(if multi { "至少选一个" } else { "只能选一个" });
    }
    for (i, value) in pick.iter().enumerate() {
        if pick[..i].contains(value) {
            return Err("选项重复");
        }
        if !options.iter().any(|o| &o.id == value) {
            return Err("没有这个选项");
        }
    }
    Ok(pick)
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let options: Vec<ChoiceOption> = json_column(row, 4)?;
    let choice: Option<String> = row.get(5)?;
    let choice = match choice {
        Some(text) => Some(serde_json::from_str(&text).map_err(|err| bad_json(5, err))?),
        None => None,
    };
    let multi: i64 = row.get(8)?;
    Ok(Event::Choice {
        id: row.get(0)?,
        bot_id: row.get(1)?,
        bot_name: row.get(2)?,
        prompt: row.get(3)?,
        options,
        multi: multi != 0,
        choice,
        created_at: row.get(6)?,
        answered_at: row.get(7)?,
    })
}

fn bad_json(column: usize, err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(err))
}

fn json_column(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<Vec<ChoiceOption>> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|err| bad_json(column, err))
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<Event>> {
    conn.query_row(&format!("{SELECT} WHERE c.id = ?1"), [id], event_from_row)
        .optional()
}

/// Choice events for one group, for the events stream.
pub(crate) fn timed_events(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<TimedEvent>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE c.group_id = ?1"))?;
    let rows = stmt.query_map([group_id], |row| {
        let id: String = row.get(0)?;
        let created_at: String = row.get(6)?;
        Ok(TimedEvent { created_at, id, event: event_from_row(row)? })
    })?;
    rows.collect()
}

pub(crate) async fn create_choice(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(body): Json<CreateChoice>,
) -> Response {
    let bot_id = body.bot_id.trim().to_string();
    let prompt = body.prompt.trim().to_string();
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(Err(error_response(StatusCode::NOT_FOUND, "群不存在")));
        }
        let member: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memberships WHERE group_id = ?1 AND bot_id = ?2",
            params![group_id, bot_id],
            |row| row.get(0),
        )?;
        if member == 0 {
            return Ok(Err(error_response(StatusCode::BAD_REQUEST, "这个 Bot 不在群里")));
        }
        if prompt.is_empty() {
            return Ok(Err(error_response(StatusCode::BAD_REQUEST, "请填写内容")));
        }
        let options = match validate_options(&body.options, body.multi) {
            Ok(options) => options,
            Err(message) => return Ok(Err(error_response(StatusCode::BAD_REQUEST, message))),
        };
        let id = new_id();
        let options_json = serde_json::to_string(&options).expect("options serialize");
        conn.execute(
            "INSERT INTO choices (id, group_id, bot_id, prompt, options, multi, choice, created_at, answered_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL)",
            params![id, group_id, bot_id, prompt, options_json, body.multi as i64, now_rfc3339()],
        )?;
        Ok(Ok(load(&conn, &id)?.expect("choice just inserted")))
    })();
    drop(conn);
    match result {
        Ok(Ok(event)) => (StatusCode::CREATED, Json(event)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}

pub(crate) async fn answer_choice(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<AnswerChoice>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        let Some(Event::Choice { options, multi, choice, .. }) = load(&conn, &id)? else {
            return Ok(Err(error_response(StatusCode::NOT_FOUND, "选项卡不存在")));
        };
        if choice.is_some() {
            return Ok(Err(error_response(StatusCode::CONFLICT, "已经选过了")));
        }
        let pick = match validate_pick(&options, multi, &body.choice) {
            Ok(pick) => pick,
            Err(message) => return Ok(Err(error_response(StatusCode::BAD_REQUEST, message))),
        };
        let pick_json = serde_json::to_string(&pick).expect("pick serializes");
        let changed = conn.execute(
            "UPDATE choices SET choice = ?1, answered_at = ?2 WHERE id = ?3 AND choice IS NULL",
            params![pick_json, now_rfc3339(), id],
        )?;
        if changed == 0 {
            return Ok(Err(error_response(StatusCode::CONFLICT, "已经选过了")));
        }
        Ok(Ok(load(&conn, &id)?.expect("choice exists")))
    })();
    drop(conn);
    match result {
        Ok(Ok(event)) => (StatusCode::OK, Json(event)).into_response(),
        Ok(Err(response)) => response,
        Err(err) => db_error(err),
    }
}
