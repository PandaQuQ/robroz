//! Progress text on a running task's status card.
//!
//! The card is the task's own `status` event, rebuilt from the `tasks` row; a
//! stream sends it again whenever `updated_at` moves. So [`set_progress`] bumps
//! `updated_at` only when the text really changes: same text, no new event.
//! Progress is shown while the task is `running` with a live container, and
//! also while `blocked` if a line is still stored (an MCP tool waiting for
//! approval). Other states do not show it.

use robroz_contract::{ProgressCode, ProgressData, ProgressParams};
use rusqlite::{params, Connection, OptionalExtension};

use crate::pump::newer_than;

/// Longest progress text on the card, in characters, "…" included.
pub(crate) const MAX_CHARS: usize = 60;

/// Key moments pump reports by itself.
pub(crate) const STARTED: &str = robroz_contract::host_text::PROGRESS_STARTED;
pub(crate) const THINKING: &str = robroz_contract::host_text::PROGRESS_THINKING;
pub(crate) const IN_SANDBOX: &str = robroz_contract::host_text::PROGRESS_IN_SANDBOX;

/// A progress line: the legacy text on the card and its code (#180), stored
/// together in `progress` and `progress_data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Progress {
    pub text: String,
    pub data: ProgressData,
}

impl Progress {
    fn new(text: impl Into<String>, code: ProgressCode, params: ProgressParams) -> Self {
        Self {
            text: text.into(),
            data: ProgressData { code, params },
        }
    }

    pub(crate) fn started() -> Self {
        Self::new(STARTED, ProgressCode::Started, ProgressParams::default())
    }

    pub(crate) fn thinking() -> Self {
        Self::new(THINKING, ProgressCode::Thinking, ProgressParams::default())
    }

    pub(crate) fn in_sandbox() -> Self {
        Self::new(
            IN_SANDBOX,
            ProgressCode::InSandbox,
            ProgressParams::default(),
        )
    }

    /// 「在用 {tool}」.
    pub(crate) fn using_tool(tool: &str) -> Self {
        Self::new(
            robroz_contract::host_text::using_tool(tool),
            ProgressCode::UsingTool,
            ProgressParams {
                tool: Some(tool.to_string()),
                ..Default::default()
            },
        )
    }

    /// 「{step}/{steps} 步」, `step` 1-based.
    pub(crate) fn router_step(step: usize, steps: usize) -> Self {
        Self::new(
            robroz_contract::host_text::router_step_progress(step, steps),
            ProgressCode::RouterStep,
            ProgressParams {
                step: u32::try_from(step).ok(),
                steps: u32::try_from(steps).ok(),
                ..Default::default()
            },
        )
    }

    /// Watchdog: a step past its timeout.
    pub(crate) fn step_timeout(minutes: u64) -> Self {
        Self::new(
            robroz_contract::host_text::step_timeout_hint(minutes),
            ProgressCode::StepTimeout,
            ProgressParams {
                minutes: Some(minutes),
                ..Default::default()
            },
        )
    }

    /// Watchdog: a live container with no new progress.
    pub(crate) fn stalled(minutes: u64) -> Self {
        Self::new(
            robroz_contract::host_text::stall_hint(minutes),
            ProgressCode::Stalled,
            ProgressParams {
                minutes: Some(minutes),
                ..Default::default()
            },
        )
    }

    /// The bro's own line (a `robroz-progress:` line from the model or the
    /// runner). `text` is normalized the same way the card is.
    pub(crate) fn custom(text: &str) -> Self {
        let line = normalize(text).unwrap_or_default();
        Self::new(
            line.clone(),
            ProgressCode::Custom,
            ProgressParams {
                text: Some(line),
                ..Default::default()
            },
        )
    }

    /// The `progress_data` column value.
    pub(crate) fn json(&self) -> String {
        serde_json::to_string(&self.data).expect("ProgressData serializes")
    }
}

/// Add `progress_data` next to `progress`. No backfill: a progress line only
/// lives while its task runs, and a row without data simply has no code.
pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let _ = conn.execute("ALTER TABLE tasks ADD COLUMN progress_data TEXT", []);
    Ok(())
}

/// `progressCode` and `progressParams` for a shown progress line, from its
/// stored data. `None` when nothing is shown or the row has no data.
pub(crate) fn codes(
    shown: Option<&str>,
    data: Option<&str>,
) -> (Option<ProgressCode>, Option<ProgressParams>) {
    if shown.is_none() {
        return (None, None);
    }
    match data.and_then(|d| serde_json::from_str::<ProgressData>(d).ok()) {
        Some(data) => (Some(data.code), data.wire_params()),
        None => (None, None),
    }
}

/// Set the progress line of a running task. Returns `true` when the card
/// changed (and a new `status` event will go out), `false` when the text is the
/// same, empty, or the task is not running with a live container.
pub(crate) fn set_progress(
    conn: &Connection,
    task_id: &str,
    progress: &Progress,
) -> rusqlite::Result<bool> {
    let Some(text) = normalize(&progress.text) else {
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
        "UPDATE tasks SET progress = ?1, progress_data = ?5, updated_at = ?2
         WHERE id = ?3 AND state = 'running' AND container_up = 1 AND updated_at = ?4",
        params![
            text,
            newer_than(&updated_at),
            task_id,
            updated_at,
            progress.json()
        ],
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

/// What the status card shows: while running with a container, or while blocked
/// with a stored line (MCP approval).
pub(crate) fn shown(
    state: robroz_contract::TaskState,
    container_up: bool,
    progress: Option<String>,
) -> Option<String> {
    match state {
        robroz_contract::TaskState::Running if container_up => progress,
        robroz_contract::TaskState::Blocked => progress,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::TaskState;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT PRIMARY KEY, state TEXT NOT NULL, container_up INTEGER NOT NULL,
                                 updated_at TEXT NOT NULL, progress TEXT, progress_data TEXT);
             INSERT INTO tasks VALUES ('t1', 'running', 1, '2026-10-03T19:00:01.000Z', NULL, NULL);
             INSERT INTO tasks VALUES ('t2', 'done', 0, '2026-10-03T19:00:01.000Z', NULL, NULL);",
        )
        .unwrap();
        conn
    }

    fn row(conn: &Connection, id: &str) -> (String, Option<String>) {
        conn.query_row(
            "SELECT updated_at, progress FROM tasks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn only_a_changed_text_bumps_updated_at() {
        let conn = db();
        let (before, _) = row(&conn, "t1");
        assert!(set_progress(&conn, "t1", &Progress::custom("第一步")).unwrap());
        let (first, text) = row(&conn, "t1");
        assert!(first > before);
        assert_eq!(text.as_deref(), Some("第一步"));

        assert!(
            !set_progress(&conn, "t1", &Progress::custom("  第一步 ")).unwrap(),
            "same text after trim"
        );
        assert!(
            !set_progress(&conn, "t1", &Progress::custom("   ")).unwrap(),
            "empty is ignored"
        );
        assert_eq!(row(&conn, "t1").0, first, "no new event");

        assert!(set_progress(&conn, "t1", &Progress::custom("第二步")).unwrap());
        let (second, _) = row(&conn, "t1");
        assert!(second > first, "{second} > {first}");
    }

    #[test]
    fn long_text_is_cut_to_sixty_chars_with_an_ellipsis() {
        assert_eq!(
            normalize(&"字".repeat(60)).unwrap(),
            "字".repeat(60),
            "exactly 60 is kept"
        );
        let cut = normalize(&"字".repeat(61)).unwrap();
        assert_eq!(cut.chars().count(), MAX_CHARS);
        assert_eq!(cut, format!("{}…", "字".repeat(59)));
        let conn = db();
        assert!(set_progress(&conn, "t1", &Progress::custom(&"长".repeat(200))).unwrap());
        assert_eq!(row(&conn, "t1").1.unwrap().chars().count(), MAX_CHARS);
        assert!(
            !set_progress(&conn, "t1", &Progress::custom(&"长".repeat(300))).unwrap(),
            "same after cutting: no new event"
        );
    }

    #[test]
    fn multi_line_text_becomes_one_line() {
        assert_eq!(
            normalize("  在查资料\n  第二行\r\n\t第三行 ").unwrap(),
            "在查资料 第二行 第三行"
        );
        assert_eq!(normalize("\n \n"), None);
        let conn = db();
        assert!(set_progress(&conn, "t1", &Progress::custom("在查\n资料")).unwrap());
        assert_eq!(row(&conn, "t1").1.as_deref(), Some("在查 资料"));
        assert!(
            !set_progress(&conn, "t1", &Progress::custom("在查  资料\n")).unwrap(),
            "same once normalized"
        );
    }

    #[test]
    fn not_running_is_left_alone() {
        let conn = db();
        assert!(!set_progress(&conn, "t2", &Progress::custom("第一步")).unwrap());
        assert_eq!(row(&conn, "t2"), ("2026-10-03T19:00:01.000Z".into(), None));
        assert!(!set_progress(&conn, "missing", &Progress::custom("第一步")).unwrap());
    }

    #[test]
    fn shown_only_while_running_with_a_container() {
        let p = || Some("在跑".to_string());
        assert_eq!(shown(TaskState::Running, true, p()), p());
        assert_eq!(shown(TaskState::Running, false, p()), None);
        assert_eq!(shown(TaskState::Done, false, p()), None);
        assert_eq!(shown(TaskState::Blocked, false, p()), p());
        assert_eq!(shown(TaskState::Blocked, false, None), None);
        assert_eq!(shown(TaskState::Queued, false, p()), None);
    }

    #[test]
    fn set_progress_stores_the_code_and_codes_reads_it_back() {
        let conn = db();
        assert!(set_progress(&conn, "t1", &Progress::using_tool("web_search")).unwrap());
        let data: Option<String> = conn
            .query_row("SELECT progress_data FROM tasks WHERE id = 't1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let (code, params) = codes(Some("在用 web_search"), data.as_deref());
        assert_eq!(code, Some(ProgressCode::UsingTool));
        assert_eq!(params.unwrap().tool.as_deref(), Some("web_search"));
        assert_eq!(codes(None, data.as_deref()), (None, None), "nothing shown");
        assert_eq!(codes(Some("x"), None), (None, None), "old row");
    }

    #[test]
    fn coded_lines_keep_the_legacy_text() {
        assert_eq!(Progress::started().text, STARTED);
        assert_eq!(Progress::router_step(2, 3).text, "2/3 步");
        assert_eq!(
            Progress::stalled(4).text,
            robroz_contract::host_text::stall_hint(4)
        );
        let custom = Progress::custom("  在查\n资料 ");
        assert_eq!(custom.text, "在查 资料");
        assert_eq!(custom.data.params.text.as_deref(), Some("在查 资料"));
    }
}
