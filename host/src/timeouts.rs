//! Step timeouts that cancel: a running task whose current step has run past
//! the step timeout (`ROBROZ_STEP_TIMEOUT_SECS`) is stopped and ends as
//! `done` + [`robroz_card::NO_RESULT`].
//!
//! A "step" is one stretch of work with its own clock:
//!
//! - the model phase after a claim (the model call and its tool calls);
//! - each Router step ([`crate::router`]), which restarts the clock;
//! - the container run after `docker run`.
//!
//! The clock lives in SQLite (`step_clocks`, wall-clock RFC 3339), so it
//! survives a host restart. Only `running` tasks are checked: time spent
//! `blocked` on an approval card is never step time, and the claim after the
//! card is answered starts a fresh clock.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS step_clocks (
    task_id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL
);
";

/// Start (or restart) the step clock for `task_id` at `at` (RFC 3339).
pub(crate) fn start(conn: &Connection, task_id: &str, at: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO step_clocks (task_id, started_at) VALUES (?1, ?2)
         ON CONFLICT(task_id) DO UPDATE SET started_at = excluded.started_at",
        params![task_id, at],
    )?;
    Ok(())
}

/// Drop the clock once the task has ended.
pub(crate) fn clear(conn: &Connection, task_id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM step_clocks WHERE task_id = ?1", [task_id])?;
    Ok(())
}

/// When the current step of `task_id` started, if it has a clock.
#[cfg(test)]
pub(crate) fn started_at(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT started_at FROM step_clocks WHERE task_id = ?1",
        [task_id],
        |r| r.get(0),
    )
    .optional()
}

/// `running` tasks whose current step started at least `timeout` before
/// `now`, oldest first. A clock that can't be parsed counts as overdue: the
/// host wrote it, so it is broken, and a stuck task must still end.
pub(crate) fn overdue(
    conn: &Connection,
    now: DateTime<Utc>,
    timeout: Duration,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT c.task_id, c.started_at FROM step_clocks c
         JOIN tasks t ON t.id = c.task_id
         WHERE t.state = 'running'
         ORDER BY c.started_at, c.task_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let limit = chrono::Duration::from_std(timeout).unwrap_or(chrono::Duration::MAX);
    let mut out = Vec::new();
    for row in rows {
        let (id, at) = row?;
        let late = match DateTime::parse_from_rfc3339(&at) {
            Ok(at) => now.signed_duration_since(at.with_timezone(&Utc)) >= limit,
            Err(_) => true,
        };
        if late {
            out.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
