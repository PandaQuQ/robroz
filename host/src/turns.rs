//! Group turns: every group message gives every bro in the room one turn.
//!
//! `POST /api/groups/{id}/say` in a group (not a DM) opens a *round* for the
//! message: one row per member in `turns`. A bro named by a leading `@name`
//! goes first and runs exactly like before (its task is visible from the
//! start, with the mention stripped from the prompt). Every other member
//! follows in join order (`memberships` insertion order).
//!
//! Turns run one after another: the next turn's task is only created once
//! every earlier turn's task has left `queued` / `running` (it is `done` or
//! `blocked`), and its prompt quotes what the earlier bros wrote. A turn also
//! waits while its bro has any other task in the group still `queued` or
//! `running`, so one bro never runs twice on the group volume at once. A non-mentioned
//! turn's task starts hidden (`tasks.turn_hidden = 1`): no status card, no
//! unread. When its model step answers with [`SILENT_REPLY`] (or nothing), the
//! task ends `done` and stays hidden: no message, no delivery, no card. Any
//! other reply reveals the task and it continues like any other task.
//!
//! Only a person's `/say` opens a round. Bro replies (deliveries), schedules
//! and `POST /api/groups/{id}/tasks` never do, so bros cannot set off each
//! other. DMs are unchanged.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

/// The whole reply a bro gives to stay silent on its turn.
pub const SILENT_REPLY: &str = "robroz-silent";

/// The depth of a round a person's message opens. A later change can let a
/// bro's reply open a round at `depth + 1`, stopping at a cap; today only
/// people open rounds, so every round is at this depth.
pub const PERSON_DEPTH: i64 = 0;

/// At most this many characters of each earlier reply are quoted.
const QUOTE_MAX: usize = 2000;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS turns (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    bot_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    body TEXT NOT NULL,
    task_id TEXT,
    skipped INTEGER NOT NULL DEFAULT 0,
    depth INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS turns_open ON turns (message_id, position) WHERE task_id IS NULL AND skipped = 0;
";

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    for column in [
        "turn_hidden INTEGER NOT NULL DEFAULT 0",
        "turn_silent INTEGER NOT NULL DEFAULT 0",
    ] {
        let _ = conn.execute(&format!("ALTER TABLE tasks ADD COLUMN {column}"), []);
    }
    Ok(())
}

/// Whether a turn reply means "stay silent": empty, or only [`SILENT_REPLY`]
/// (progress lines aside).
pub fn is_silent(reply: &str) -> bool {
    let lines: Vec<&str> = reply
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("robroz-progress:"))
        .collect();
    lines.is_empty() || lines == [SILENT_REPLY]
}

/// Members of `group_id` in join order: `(bot id, name)`.
fn members(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.name FROM memberships m JOIN bots b ON b.id = m.bot_id
         WHERE m.group_id = ?1 ORDER BY m.rowid",
    )?;
    let rows = stmt.query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Open a round for a group message. `mentioned` is the `@name` bro and the
/// task already queued for it, if any; that turn is recorded as started.
/// Every other member gets a turn after it, in join order. Starts the first
/// waiting turn when nothing is ahead of it.
pub(crate) fn open_round(
    conn: &Connection,
    group_id: &str,
    message_id: &str,
    body: &str,
    mentioned: Option<(&str, &str)>,
    depth: i64,
    now: &str,
) -> rusqlite::Result<()> {
    let mut position = 0i64;
    let mut insert = |bot: &str, task: Option<&str>| -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO turns (id, group_id, message_id, bot_id, position, body, task_id, depth, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![crate::new_id(), group_id, message_id, bot, position, body, task, depth, now],
        )?;
        position += 1;
        Ok(())
    };
    if let Some((bot, task)) = mentioned {
        insert(bot, Some(task))?;
    }
    for (bot, _) in members(conn, group_id)? {
        if mentioned.is_some_and(|(m, _)| m == bot) {
            continue;
        }
        insert(&bot, None)?;
    }
    advance(conn, None, now)?;
    Ok(())
}

/// One waiting turn, with the round it belongs to.
struct Waiting {
    id: String,
    group_id: String,
    message_id: String,
    bot_id: String,
    position: i64,
    body: String,
}

/// Start every turn whose earlier turns are all finished (`done` or
/// `blocked`). Earlier replies are read from the group volume under
/// `volumes_root`; without it (inside `/say`), only a round's first turn can
/// start. Idempotent. Returns the task ids it queued.
pub(crate) fn advance(
    conn: &Connection,
    volumes_root: Option<&Path>,
    now: &str,
) -> rusqlite::Result<Vec<String>> {
    let waiting: Vec<Waiting> = {
        // The first waiting turn of each round.
        let mut stmt = conn.prepare(
            "SELECT t.id, t.group_id, t.message_id, t.bot_id, t.position, t.body FROM turns t
             WHERE t.task_id IS NULL AND t.skipped = 0
               AND NOT EXISTS (SELECT 1 FROM turns e WHERE e.message_id = t.message_id
                               AND e.position < t.position AND e.task_id IS NULL AND e.skipped = 0)
             ORDER BY t.created_at, t.rowid",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Waiting {
                id: r.get(0)?,
                group_id: r.get(1)?,
                message_id: r.get(2)?,
                bot_id: r.get(3)?,
                position: r.get(4)?,
                body: r.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut started = Vec::new();
    for turn in waiting {
        let mut turn = Some(turn);
        // A skipped turn (bro left the group) lets the next one go at once.
        while let Some(t) = turn.take() {
            match start(conn, &t, volumes_root, now)? {
                Start::Busy => {}
                Start::Started(task) => started.push(task),
                Start::Skipped => turn = next_waiting(conn, &t.message_id)?,
            }
        }
    }
    Ok(started)
}

fn next_waiting(conn: &Connection, message_id: &str) -> rusqlite::Result<Option<Waiting>> {
    conn.query_row(
        "SELECT id, group_id, message_id, bot_id, position, body FROM turns
         WHERE message_id = ?1 AND task_id IS NULL AND skipped = 0 ORDER BY position LIMIT 1",
        [message_id],
        |r| {
            Ok(Waiting {
                id: r.get(0)?,
                group_id: r.get(1)?,
                message_id: r.get(2)?,
                bot_id: r.get(3)?,
                position: r.get(4)?,
                body: r.get(5)?,
            })
        },
    )
    .optional()
}

enum Start {
    Busy,
    Started(String),
    Skipped,
}

fn start(
    conn: &Connection,
    turn: &Waiting,
    volumes_root: Option<&Path>,
    now: &str,
) -> rusqlite::Result<Start> {
    // Earlier turns that ran, oldest first: (bot name, task id, state).
    let earlier: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(b.name, e.bot_id), k.id, k.state FROM turns e
             JOIN tasks k ON k.id = e.task_id LEFT JOIN bots b ON b.id = e.bot_id
             WHERE e.message_id = ?1 AND e.position < ?2 ORDER BY e.position",
        )?;
        let rows = stmt.query_map(params![turn.message_id, turn.position], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if earlier
        .iter()
        .any(|(_, _, state)| state == "queued" || state == "running")
    {
        return Ok(Start::Busy);
    }
    let member: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM memberships m JOIN bots b ON b.id = m.bot_id
                       WHERE m.group_id = ?1 AND m.bot_id = ?2)",
        params![turn.group_id, turn.bot_id],
        |r| r.get(0),
    )?;
    if !member {
        conn.execute("UPDATE turns SET skipped = 1 WHERE id = ?1", [&turn.id])?;
        return Ok(Start::Skipped);
    }
    // The bro is still working in this group (its own task, or a turn of
    // another round): wait until that leaves `queued` / `running`, so two
    // runs of one bro never share the group volume at once.
    let working: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE group_id = ?1 AND assignee_bot_id = ?2
                       AND state IN ('queued', 'running'))",
        params![turn.group_id, turn.bot_id],
        |r| r.get(0),
    )?;
    if working {
        return Ok(Start::Busy);
    }
    let replies = match volumes_root {
        Some(root) => earlier
            .iter()
            .filter_map(|(name, task, _)| {
                reply_of(conn, root, &turn.group_id, task).map(|r| (name.clone(), r))
            })
            .collect(),
        None if earlier.is_empty() => Vec::new(),
        // Earlier replies live in the group volume: wait for the pump.
        None => return Ok(Start::Busy),
    };
    let group_name: String = conn
        .query_row(
            "SELECT name FROM groups WHERE id = ?1",
            [&turn.group_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let task_id = crate::new_id();
    conn.execute(
        "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, turn_hidden)
         VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?5, 1)",
        params![task_id, turn.group_id, turn.bot_id, prompt(&group_name, &turn.body, &replies), now],
    )?;
    conn.execute(
        "UPDATE turns SET task_id = ?2 WHERE id = ?1",
        params![turn.id, task_id],
    )?;
    Ok(Start::Started(task_id))
}

/// What an earlier bro wrote on its turn: its delivered result, if any.
fn reply_of(
    conn: &Connection,
    volumes_root: &Path,
    group_id: &str,
    task_id: &str,
) -> Option<String> {
    let path: String = conn
        .query_row(
            "SELECT path FROM deliveries WHERE task_id = ?1 AND failure IS NULL ORDER BY created_at DESC LIMIT 1",
            [task_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    let text =
        std::fs::read_to_string(crate::pump::group_volume(volumes_root, group_id).join(path))
            .ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| clip(text))
}

fn clip(text: &str) -> String {
    if text.chars().count() <= QUOTE_MAX {
        return text.to_string();
    }
    let mut out: String = text.chars().take(QUOTE_MAX).collect();
    out.push('…');
    out
}

/// Quoted text loses its `robroz-` control lines, so a quote can never ask
/// for an outside write, a progress line or silence on the reader's behalf.
pub(crate) fn quote(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("robroz-"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prompt for a non-mentioned turn.
pub(crate) fn prompt(group_name: &str, body: &str, replies: &[(String, String)]) -> String {
    let mut text = format!(
        "你在群「{group_name}」里。有人发了一条消息，群里每个 bro 按顺序各轮一次，现在轮到你。\n\
         没有要补充的就不说话：整条回复只写一行 {SILENT_REPLY}。\n\
         不要替别的 bro 回答，也不要重复前面 bro 说过的话。\n\n\
         消息：\n{}\n",
        quote(body)
    );
    if !replies.is_empty() {
        text.push_str("\n前面 bro 的回复：\n");
        for (name, reply) in replies {
            text.push_str(&format!("【{name}】\n{}\n", quote(reply)));
        }
    }
    text
}

/// End a hidden turn task without a word: `done`, still hidden, no delivery.
pub(crate) fn end_silent(
    conn: &Connection,
    task_id: &str,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE tasks SET state = 'done', container_up = 0, updated_at = ?2, progress = NULL, turn_silent = 1
         WHERE id = ?1 AND state = 'running' AND turn_hidden = 1",
        params![task_id, updated_at],
    )?;
    Ok(changed > 0)
}

/// Show a turn task that decided to speak: its status card appears now.
pub(crate) fn reveal(conn: &Connection, task_id: &str, at: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET turn_hidden = 0, created_at = ?2, updated_at = ?2 WHERE id = ?1 AND turn_hidden = 1",
        params![task_id, at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
