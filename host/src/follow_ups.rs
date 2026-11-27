//! Follow-ups: a person's `/say` to a bro (in its DM, or with a leading
//! `@name` in a group) when that bro already has a still-open task there
//! (`queued`, `running` or `blocked`, and not a hidden turn) joins that task
//! instead of starting a new one.
//!
//! The message is stored here, not yet in the task's prompt. It reaches the
//! bro on the task's next model step:
//!
//! - `queued`: the next start folds it into the prompt ([`fold`]), so the
//!   step that is about to run sees it.
//! - `running` (or a `blocked` task that resumes): the current step goes on
//!   with what it was given. When it ends `done` with a result, its reply is
//!   delivered as usual and, because a follow-up is still waiting, the same
//!   task goes back to `queued` ([`continue_after_done`]). The next step's
//!   prompt quotes that reply and the follow-up. Same task id, one card.
//! - A step that fails (or a dead run) ends the task as today. A follow-up
//!   still waiting then stays unanswered; nothing is re-queued.
//!
//! A follow-up is never a `resume`: a `blocked` task stays blocked until the
//! person answers its card.
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS follow_ups (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    body TEXT NOT NULL,
    folded INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS follow_ups_waiting ON follow_ups (task_id) WHERE folded = 0;
";

/// The still-open task a follow-up to `bot_id` in `group_id` joins: the
/// newest visible `queued` / `running` / `blocked` one. `(id, prompt, state)`.
pub(crate) fn open_task(
    conn: &Connection,
    group_id: &str,
    bot_id: &str,
) -> rusqlite::Result<Option<(String, String, String)>> {
    conn.query_row(
        "SELECT id, prompt, state FROM tasks
         WHERE group_id = ?1 AND assignee_bot_id = ?2 AND turn_hidden = 0
           AND state IN ('queued', 'running', 'blocked')
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        params![group_id, bot_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

/// Store `body` (the message without its `@name`) as waiting for `task_id`.
pub(crate) fn attach(
    conn: &Connection,
    task_id: &str,
    message_id: &str,
    body: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO follow_ups (id, task_id, message_id, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![crate::new_id(), task_id, message_id, body, now],
    )?;
    Ok(())
}

fn waiting(conn: &Connection, task_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, body FROM follow_ups WHERE task_id = ?1 AND folded = 0 ORDER BY created_at, rowid",
    )?;
    let rows = stmt.query_map([task_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// The text a follow-up adds to the prompt. A person's own words, kept as
/// they are, exactly like the prompt of a new `@name` task.
pub(crate) fn section(body: &str) -> String {
    format!("\n\nFollow-up:\n{body}")
}

/// Fold waiting follow-ups into a `queued` task's prompt before it starts.
/// Returns the new prompt, or `None` when nothing was waiting.
pub(crate) fn fold(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<String>> {
    let rows = waiting(conn, task_id)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let tx = conn.unchecked_transaction()?;
    let extra: String = rows.iter().map(|(_, body)| section(body)).collect();
    let changed = tx.execute(
        "UPDATE tasks SET prompt = prompt || ?2 WHERE id = ?1 AND state = 'queued'",
        params![task_id, extra],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    for (id, _) in &rows {
        tx.execute("UPDATE follow_ups SET folded = 1 WHERE id = ?1", [id])?;
    }
    let prompt: String =
        tx.query_row("SELECT prompt FROM tasks WHERE id = ?1", [task_id], |r| {
            r.get(0)
        })?;
    tx.commit()?;
    Ok(Some(prompt))
}

/// After a step ended `done` with a result: if a follow-up came in while it
/// ran, queue the same task again, its prompt now quoting `reply` (control
/// lines dropped, so a reply cannot gate the next step). The follow-up
/// itself is folded in by the next start. Returns whether it re-queued.
pub(crate) fn continue_after_done(
    conn: &Connection,
    task_id: &str,
    reply: Option<&str>,
    at: &str,
) -> rusqlite::Result<bool> {
    if waiting(conn, task_id)?.is_empty() {
        return Ok(false);
    }
    let quoted = crate::turns::quote(reply.unwrap_or("").trim());
    let extra =
        format!("\n\nYour reply to the last step (already posted, don't repeat it):\n{quoted}");
    let changed = conn.execute(
        "UPDATE tasks SET state = 'queued', prompt = prompt || ?3, updated_at = ?2, progress = NULL,
                          started_at = NULL
         WHERE id = ?1 AND state = 'done' AND failure IS NULL AND needs_setup IS NULL",
        params![task_id, at, extra],
    )?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests;
