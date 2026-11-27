//! Change log behind the live stream (`GET /api/groups/{id}/stream`).
//!
//! Every write to a table that a card is built from appends one row to
//! `group_changes`: which group's stream should send which card again. SQLite
//! triggers write the rows, so no write path has to remember to. `seq` is an
//! `AUTOINCREMENT` key, so it only grows and is never reused, and because all
//! writes are serialised by SQLite it follows commit order, not the wall
//! clock. The stream's SSE `id:` is that `seq`.
//!
//! A row only says "this card changed"; the stream rebuilds the card's
//! current state from its own table. So the log can be compacted to the
//! newest row per card without losing anything a reconnect needs. Rows older
//! than [`DEFAULT_RETENTION`] are dropped too; `event_log_meta.pruned_through`
//! remembers the highest `seq` dropped that way, and a reconnect from before it
//! gets a full snapshot instead of a replay.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};

/// Default for `ROBROZ_EVENT_RETENTION_HOURS`: 7 days.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(168 * 60 * 60);

/// How often the host compacts and prunes the log.
pub const MAINTAIN_EVERY: Duration = Duration::from_secs(60 * 60);

/// Tables a card is built from that have triggers, so a write to them is
/// pushed. Read by the test that checks every table the card builders read.
pub(crate) const TRACKED_TABLES: &[&str] = &[
    "messages",
    "tasks",
    "deliveries",
    "choices",
    "approvals",
    "memories",
    "skills",
];

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS group_changes (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id TEXT NOT NULL,
    card_id TEXT NOT NULL,
    at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS group_changes_group_seq ON group_changes (group_id, seq);
CREATE INDEX IF NOT EXISTS group_changes_card ON group_changes (group_id, card_id);
CREATE INDEX IF NOT EXISTS group_changes_at ON group_changes (at);
CREATE TABLE IF NOT EXISTS event_log_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pruned_through INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO event_log_meta (id, pruned_through) VALUES (1, 0);
CREATE INDEX IF NOT EXISTS messages_group ON messages (group_id);
CREATE INDEX IF NOT EXISTS tasks_group ON tasks (group_id);
CREATE INDEX IF NOT EXISTS deliveries_group ON deliveries (group_id);
CREATE INDEX IF NOT EXISTS choices_group ON choices (group_id);
CREATE INDEX IF NOT EXISTS approvals_group ON approvals (group_id);
CREATE INDEX IF NOT EXISTS approvals_origin_group ON approvals (origin_group_id);
";

/// `strftime` for the `at` column: RFC 3339 UTC with milliseconds, the same
/// shape as every other timestamp the host writes.
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// One `INSERT INTO group_changes` per card a row of `table` shows up as.
fn trigger_body(table: &str) -> String {
    let log = |group: &str, card: &str| {
        format!(
            "INSERT INTO group_changes (group_id, card_id, at) VALUES ({group}, {card}, {NOW});"
        )
    };
    match table {
        "messages" | "tasks" | "deliveries" | "choices" => log("NEW.group_id", "NEW.id"),
        // The card in its group, again where a bro first posted it, the
        // blocked task it holds up (its `waitingFor` reads open cards), and the owner's DM reminder line.
        "approvals" => format!(
            "{}
             INSERT INTO group_changes (group_id, card_id, at)
                 SELECT NEW.origin_group_id, NEW.id, {NOW}
                 WHERE NEW.origin_group_id IS NOT NULL AND NEW.origin_group_id <> NEW.group_id;
             INSERT INTO group_changes (group_id, card_id, at)
                 SELECT t.group_id, t.id, {NOW} FROM tasks t
                 WHERE NEW.subject_kind IS NULL AND t.id = NEW.task_id AND t.state = 'blocked';
             INSERT INTO group_changes (group_id, card_id, at)
                 SELECT d.id, 'reminder:' || NEW.id, {NOW}
                 FROM groups d JOIN bots b ON b.id = NEW.bot_id
                 WHERE NEW.subject_kind IS NULL AND d.dm_bot_id = NEW.bot_id
                   AND d.dm_user_id = COALESCE(b.owner_user_id, 'local-admin');",
            log("NEW.group_id", "NEW.id")
        ),
        // A confirm card shows its memory or skill as it is now.
        "memories" | "skills" => {
            let kind = if table == "memories" {
                "memory"
            } else {
                "skill"
            };
            format!(
                "INSERT INTO group_changes (group_id, card_id, at)
                     SELECT a.group_id, a.id, {NOW} FROM approvals a
                     WHERE a.subject_kind = '{kind}' AND a.subject_id = NEW.id;"
            )
        }
        other => unreachable!("no trigger for {other}"),
    }
}

/// Create the log, its triggers and the `group_id` indexes the stream needs.
/// Runs after every other migration, since triggers name their columns.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    for table in TRACKED_TABLES {
        let body = trigger_body(table);
        for op in ["insert", "update"] {
            // Dropped and created again so a newer host can change the body.
            conn.execute_batch(&format!(
                "DROP TRIGGER IF EXISTS {table}_log_{op};
                 CREATE TRIGGER {table}_log_{op} AFTER {} ON {table}
                 BEGIN {body} END;",
                op.to_uppercase()
            ))?;
        }
    }
    Ok(())
}

/// The highest `seq` ever handed out (0 for an empty log).
pub(crate) fn head(conn: &Connection) -> rusqlite::Result<i64> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'group_changes'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(seq.unwrap_or(0))
}

/// Highest `seq` dropped by age. A cursor below it cannot be replayed.
pub(crate) fn pruned_through(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT pruned_through FROM event_log_meta WHERE id = 1",
        [],
        |r| r.get(0),
    )
}

/// Whether a stream at `cursor` can be replayed from the log: not from before
/// what was pruned, and not from the future (a log that was reset).
pub(crate) fn replayable(conn: &Connection, cursor: i64) -> rusqlite::Result<bool> {
    Ok(cursor >= pruned_through(conn)? && cursor <= head(conn)?)
}

/// Cards of `group_id` that changed after `after`, and the newest `seq` seen.
/// `None` when nothing changed.
pub(crate) fn changed_since(
    conn: &Connection,
    group_id: &str,
    after: i64,
) -> rusqlite::Result<Option<(HashSet<String>, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, card_id FROM group_changes WHERE group_id = ?1 AND seq > ?2 ORDER BY seq",
    )?;
    let mut rows = stmt.query(params![group_id, after])?;
    let mut cards = HashSet::new();
    let mut last = after;
    while let Some(row) = rows.next()? {
        last = row.get(0)?;
        cards.insert(row.get(1)?);
    }
    Ok((last > after).then_some((cards, last)))
}

/// What one maintenance pass removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MaintainReport {
    /// Rows dropped because a newer row names the same card.
    pub compacted: usize,
    /// Rows dropped because they were older than the retention.
    pub expired: usize,
}

/// Compact the log to the newest row per card, then drop rows older than
/// `retention` and raise `pruned_through` past them.
pub(crate) fn maintain(
    conn: &Connection,
    now: DateTime<Utc>,
    retention: Duration,
) -> rusqlite::Result<MaintainReport> {
    let tx = conn.unchecked_transaction()?;
    let compacted = tx.execute(
        "DELETE FROM group_changes WHERE seq NOT IN (
             SELECT MAX(seq) FROM group_changes GROUP BY group_id, card_id
         )",
        [],
    )?;
    let cutoff = (now - chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::MAX))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let oldest_kept: Option<i64> = tx.query_row(
        "SELECT MAX(seq) FROM group_changes WHERE at < ?1",
        [&cutoff],
        |r| r.get(0),
    )?;
    let mut expired = 0;
    if let Some(through) = oldest_kept {
        expired = tx.execute("DELETE FROM group_changes WHERE seq <= ?1", [through])?;
        tx.execute(
            "UPDATE event_log_meta SET pruned_through = MAX(pruned_through, ?1) WHERE id = 1",
            [through],
        )?;
    }
    tx.commit()?;
    Ok(MaintainReport { compacted, expired })
}

/// `ROBROZ_EVENT_RETENTION_HOURS`; a missing, zero or unparsable value keeps
/// [`DEFAULT_RETENTION`].
pub fn retention_from_env() -> Duration {
    retention_from(
        std::env::var("ROBROZ_EVENT_RETENTION_HOURS")
            .ok()
            .as_deref(),
    )
}

fn retention_from(raw: Option<&str>) -> Duration {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&h| h > 0)
        .and_then(|h| h.checked_mul(60 * 60))
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_RETENTION)
}

#[cfg(test)]
pub(crate) mod tests;
