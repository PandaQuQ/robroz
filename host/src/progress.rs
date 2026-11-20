//! Progress text on a running task's status card.
//!
//! The card is the task's own `status` event, rebuilt from the `tasks` row; a
//! stream sends it again whenever `updated_at` moves. So [`set_progress`] bumps
//! `updated_at` only when the text really changes: same text, no new event.
//! Progress is only kept (and only shown) while the task is `running` with a
//! live container; every other state clears it.

use rusqlite::{params, Connection, OptionalExtension};

use crate::pump::newer_than;

/// Longest progress text on the card, in characters, "…" included.
pub(crate) const MAX_CHARS: usize = 60;

/// Key moments pump reports by itself.
pub(crate) const STARTED: &str = "开始干活";
pub(crate) const THINKING: &str = "在想怎么回";
pub(crate) const IN_SANDBOX: &str = "在沙箱里跑";

/// Set the progress text of a running task. Returns `true` when the card
/// changed (and a new `status` event will go out), `false` when the text is the
/// same, empty, or the task is not running with a live container.
pub(crate) fn set_progress(conn: &Connection, task_id: &str, text: &str) -> rusqlite::Result<bool> {
    let Some(text) = normalize(text) else {
        return Ok(false);
    };
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT updated_at, progress FROM tasks
             WHERE id = ?1 AND state = 'running' AND container_up = 1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((updated_at, current)) = row else {
        return Ok(false);
    };
    if current.as_deref() == Some(text.as_str()) {
        return Ok(false);
    }
    // Guarded on the old `updated_at`: if the task moved meanwhile, leave it.
    let changed = conn.execute(
        "UPDATE tasks SET progress = ?1, updated_at = ?2
         WHERE id = ?3 AND state = 'running' AND container_up = 1 AND updated_at = ?4",
        params![text, newer_than(&updated_at), task_id, updated_at],
    )?;
    Ok(changed > 0)
}

/// One line for the card: every run of whitespace (newlines too) becomes one
/// space, trimmed, at most [`MAX_CHARS`] characters with "…" when cut.
/// `None` when nothing is left. Dedupe compares this, not the raw text.
pub(crate) fn normalize(text: &str) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= MAX_CHARS {
        return Some(line);
    }
    Some(crate::text::clip(&line, MAX_CHARS - 1))
}

/// What the status card shows: progress only while running with a container.
pub(crate) fn shown(state: robroz_contract::TaskState, container_up: bool, progress: Option<String>) -> Option<String> {
    (state == robroz_contract::TaskState::Running && container_up)
        .then_some(progress)
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::TaskState;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT PRIMARY KEY, state TEXT NOT NULL, container_up INTEGER NOT NULL,
                                 updated_at TEXT NOT NULL, progress TEXT);
             INSERT INTO tasks VALUES ('t1', 'running', 1, '2026-10-03T19:00:01.000Z', NULL);
             INSERT INTO tasks VALUES ('t2', 'done', 0, '2026-10-03T19:00:01.000Z', NULL);",
        )
        .unwrap();
        conn
    }

    fn row(conn: &Connection, id: &str) -> (String, Option<String>) {
        conn.query_row("SELECT updated_at, progress FROM tasks WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
    }

    #[test]
    fn only_a_changed_text_bumps_updated_at() {
        let conn = db();
        let (before, _) = row(&conn, "t1");
        assert!(set_progress(&conn, "t1", "第一步").unwrap());
        let (first, text) = row(&conn, "t1");
        assert!(first > before);
        assert_eq!(text.as_deref(), Some("第一步"));

        assert!(!set_progress(&conn, "t1", "  第一步 ").unwrap(), "same text after trim");
        assert!(!set_progress(&conn, "t1", "   ").unwrap(), "empty is ignored");
        assert_eq!(row(&conn, "t1").0, first, "no new event");

        assert!(set_progress(&conn, "t1", "第二步").unwrap());
        let (second, _) = row(&conn, "t1");
        assert!(second > first, "{second} > {first}");
    }

    #[test]
    fn long_text_is_cut_to_sixty_chars_with_an_ellipsis() {
        assert_eq!(normalize(&"字".repeat(60)).unwrap(), "字".repeat(60), "exactly 60 is kept");
        let cut = normalize(&"字".repeat(61)).unwrap();
        assert_eq!(cut.chars().count(), MAX_CHARS);
        assert_eq!(cut, format!("{}…", "字".repeat(59)));
        let conn = db();
        assert!(set_progress(&conn, "t1", &"长".repeat(200)).unwrap());
        assert_eq!(row(&conn, "t1").1.unwrap().chars().count(), MAX_CHARS);
        assert!(!set_progress(&conn, "t1", &"长".repeat(300)).unwrap(), "same after cutting: no new event");
    }

    #[test]
    fn multi_line_text_becomes_one_line() {
        assert_eq!(normalize("  在查资料\n  第二行\r\n\t第三行 ").unwrap(), "在查资料 第二行 第三行");
        assert_eq!(normalize("\n \n"), None);
        let conn = db();
        assert!(set_progress(&conn, "t1", "在查\n资料").unwrap());
        assert_eq!(row(&conn, "t1").1.as_deref(), Some("在查 资料"));
        assert!(!set_progress(&conn, "t1", "在查  资料\n").unwrap(), "same once normalized");
    }

    #[test]
    fn not_running_is_left_alone() {
        let conn = db();
        assert!(!set_progress(&conn, "t2", "第一步").unwrap());
        assert_eq!(row(&conn, "t2"), ("2026-10-03T19:00:01.000Z".into(), None));
        assert!(!set_progress(&conn, "missing", "第一步").unwrap());
    }

    #[test]
    fn shown_only_while_running_with_a_container() {
        let p = || Some("在跑".to_string());
        assert_eq!(shown(TaskState::Running, true, p()), p());
        assert_eq!(shown(TaskState::Running, false, p()), None);
        assert_eq!(shown(TaskState::Done, false, p()), None);
        assert_eq!(shown(TaskState::Blocked, false, p()), None);
        assert_eq!(shown(TaskState::Queued, false, p()), None);
    }
}
