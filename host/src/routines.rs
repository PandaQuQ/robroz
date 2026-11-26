//! Routines view over schedules: list a group's schedules and pause or resume
//! one. Pausing only stops `tick` from creating new runs; a run already queued
//! or live is left alone. Resuming picks the next slot after now, no backfill.

use crate::extract::Json;
use crate::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use robroz_contract::{Schedule, ScheduleEntry, Schedules, UpdateSchedule};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{db_error, error_response, group_exists, lock_db, now_rfc3339, schedule, AppState};

const COLUMNS: &str =
    "id, group_id, assignee_bot_id, cron, prompt, next_run_at, last_task_id, paused_at";

fn entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleEntry> {
    let paused_at: Option<String> = row.get(7)?;
    Ok(ScheduleEntry {
        schedule: Schedule {
            id: row.get(0)?,
            group_id: row.get(1)?,
            assignee_bot_id: row.get(2)?,
            cron: row.get(3)?,
            prompt: row.get(4)?,
            next_run_at: row.get(5)?,
            last_task_id: row.get(6)?,
        },
        paused: paused_at.is_some(),
    })
}

fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<ScheduleEntry>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM schedules WHERE id = ?1"),
        [id],
        entry_from_row,
    )
    .optional()
}

pub(crate) fn list_for_group(
    conn: &Connection,
    group_id: &str,
) -> rusqlite::Result<Vec<ScheduleEntry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM schedules WHERE group_id = ?1 ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([group_id], entry_from_row)?;
    rows.collect()
}

/// Sets or clears `paused_at`. Pausing twice keeps the first timestamp.
pub(crate) fn set_paused(
    conn: &Connection,
    id: &str,
    paused: bool,
) -> rusqlite::Result<Option<ScheduleEntry>> {
    let Some(current) = load(conn, id)? else {
        return Ok(None);
    };
    if paused {
        conn.execute(
            "UPDATE schedules SET paused_at = COALESCE(paused_at, ?1) WHERE id = ?2",
            params![now_rfc3339(), id],
        )?;
    } else if current.paused {
        let next = schedule::parse_cron(&current.schedule.cron)
            .and_then(|cron| schedule::next_after(&cron, Utc::now()))
            .map(schedule::stamp)
            .unwrap_or(current.schedule.next_run_at);
        conn.execute(
            "UPDATE schedules SET paused_at = NULL, next_run_at = ?1 WHERE id = ?2",
            params![next, id],
        )?;
    }
    load(conn, id)
}

pub(crate) async fn list_schedules(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
) -> Response {
    let conn = lock_db(&state);
    let result = (|| {
        if !group_exists(&conn, &group_id)? {
            return Ok(None);
        }
        list_for_group(&conn, &group_id).map(Some)
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            robroz_contract::host_text::GROUP_NOT_FOUND,
        ),
        Ok(Some(schedules)) => (StatusCode::OK, Json(Schedules { schedules })).into_response(),
        Err(err) => db_error(err),
    }
}

pub(crate) async fn update_schedule(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<UpdateSchedule>,
) -> Response {
    let conn = lock_db(&state);
    let result = set_paused(&conn, &id, body.paused);
    drop(conn);
    match result {
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            robroz_contract::host_text::ROUTINE_NOT_FOUND,
        ),
        Ok(Some(entry)) => (StatusCode::OK, Json(entry)).into_response(),
        Err(err) => db_error(err),
    }
}
