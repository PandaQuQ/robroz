//! Host Watchdog: a periodic check next to `tick` and `pump`.
//!
//! It looks at running work for three things:
//!
//! - a step (the model call and its tool calls, see [`crate::steps`]) that has
//!   run for half of [`WatchdogConfig::step_timeout`] ([`WatchdogConfig::hint_after`]);
//! - a live container whose progress text has not changed for
//!   [`WatchdogConfig::stall_after`];
//! - a dead run: `running` with `containerUp` false.
//!
//! The first two only log and put a one-line hint in the task's existing
//! `progress` text. They never change the task's state and never re-queue it.
//! Cancelling a step at the full timeout is the pump's job
//! ([`crate::timeouts`]); the hint comes at half of it, so the card shows it
//! before the step is cancelled.
//! A dead run is ended as `done` with the `failure` [`DEAD_RUN_FAILURE`], in a
//! compare-and-set write. It is not re-queued and no container is started.
//!
//! Time is passed in (`now`), so tests do not sleep.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use crate::progress;
use crate::pump::newer_than;

/// `failure` on a dead run that the host ended. The card text, verbatim.
pub const DEAD_RUN_FAILURE: &str = robroz_contract::legacy::NO_RESULT;

/// Default for `ROBROZ_STEP_TIMEOUT_SECS`.
pub const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Default for `ROBROZ_STALL_SECS`.
pub const DEFAULT_STALL_AFTER: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogConfig {
    /// A step running this long is cancelled by the pump; at half of it the
    /// Watchdog puts up a hint ([`Self::hint_after`]).
    pub step_timeout: Duration,
    /// A live container whose progress has not moved for this long gets a hint.
    pub stall_after: Duration,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            step_timeout: DEFAULT_STEP_TIMEOUT,
            stall_after: DEFAULT_STALL_AFTER,
        }
    }
}

impl WatchdogConfig {
    /// When a step gets the 「模型和工具还没回」 hint: half of
    /// [`Self::step_timeout`], so it shows before the cancel.
    pub fn hint_after(&self) -> Duration {
        self.step_timeout / 2
    }

    /// `ROBROZ_STEP_TIMEOUT_SECS` and `ROBROZ_STALL_SECS`; a missing, zero or
    /// unparsable value keeps the default.
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let secs = |name: &str, default: Duration| {
            get(name)
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|&n| n > 0)
                .map(Duration::from_secs)
                .unwrap_or(default)
        };
        Self {
            step_timeout: secs("ROBROZ_STEP_TIMEOUT_SECS", DEFAULT_STEP_TIMEOUT),
            stall_after: secs("ROBROZ_STALL_SECS", DEFAULT_STALL_AFTER),
        }
    }
}

/// Hint text for a step past its timeout.
pub fn step_timeout_hint(elapsed: Duration) -> String {
    robroz_contract::host_text::step_timeout_hint(minutes(elapsed))
}

/// Hint text for a live container whose progress has not moved.
pub fn stall_hint(idle: Duration) -> String {
    robroz_contract::host_text::stall_hint(minutes(idle))
}

fn minutes(d: Duration) -> u64 {
    (d.as_secs() / 60).max(1)
}

/// A running task with a live container, as the Watchdog sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTask {
    pub id: String,
    pub progress: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingKind {
    StepTimeout,
    Stalled,
}

/// One thing the Watchdog wants logged and shown on the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub task_id: String,
    pub kind: FindingKind,
    pub hint: String,
    /// The minutes in [`Self::hint`], for the coded progress (#180).
    pub minutes: u64,
}

impl Finding {
    /// The hint as a coded progress line.
    pub(crate) fn progress(&self) -> progress::Progress {
        let line = match self.kind {
            FindingKind::StepTimeout => progress::Progress::step_timeout(self.minutes),
            FindingKind::Stalled => progress::Progress::stalled(self.minutes),
        };
        debug_assert_eq!(line.text, self.hint);
        line
    }
}

#[derive(Debug)]
struct Seen {
    /// Last progress text that was not our own hint.
    baseline: Option<String>,
    since: Instant,
    /// The hint we put on the card for this baseline, if any.
    hint: Option<String>,
    /// Start of the step we already flagged, so one step is flagged once.
    flagged_step: Option<Instant>,
}

/// Remembers, across passes, when each task's progress last moved.
#[derive(Debug)]
pub struct Watchdog {
    cfg: WatchdogConfig,
    seen: HashMap<String, Seen>,
}

impl Watchdog {
    pub fn new(cfg: WatchdogConfig) -> Self {
        Self {
            cfg,
            seen: HashMap::new(),
        }
    }

    pub fn config(&self) -> WatchdogConfig {
        self.cfg
    }

    /// Decide what to flag. `live` is every running task with a live
    /// container; `steps` is every step in flight with its start time. Pure:
    /// no database, no clock.
    pub fn check(
        &mut self,
        live: &[LiveTask],
        steps: &[(String, Instant)],
        now: Instant,
    ) -> Vec<Finding> {
        self.seen.retain(|id, _| live.iter().any(|t| &t.id == id));
        let mut findings = Vec::new();
        for task in live {
            let seen = self.seen.entry(task.id.clone()).or_insert_with(|| Seen {
                baseline: task.progress.clone(),
                since: now,
                hint: None,
                flagged_step: None,
            });
            let ours = seen.hint.is_some() && seen.hint == task.progress;
            if !ours && seen.baseline != task.progress {
                seen.baseline = task.progress.clone();
                seen.since = now;
                seen.hint = None;
            }
            let step = steps
                .iter()
                .find(|(id, _)| id == &task.id)
                .map(|(_, at)| *at);
            if let Some(started) = step {
                let elapsed = now.saturating_duration_since(started);
                if elapsed >= self.cfg.hint_after() && seen.flagged_step != Some(started) {
                    let hint = step_timeout_hint(elapsed);
                    seen.flagged_step = Some(started);
                    seen.hint = Some(hint.clone());
                    findings.push(Finding {
                        task_id: task.id.clone(),
                        kind: FindingKind::StepTimeout,
                        hint,
                        minutes: minutes(elapsed),
                    });
                }
                continue;
            }
            let idle = now.saturating_duration_since(seen.since);
            if idle >= self.cfg.stall_after && seen.hint.is_none() {
                let hint = stall_hint(idle);
                seen.hint = Some(hint.clone());
                findings.push(Finding {
                    task_id: task.id.clone(),
                    kind: FindingKind::Stalled,
                    hint,
                    minutes: minutes(idle),
                });
            }
        }
        findings
    }

    /// `true` while our hint is on the card and the runner's progress text is
    /// still the one we flagged. Pump then leaves the hint in place instead of
    /// writing the same old text back.
    pub fn holds_hint(&self, task_id: &str, runner_text: &str) -> bool {
        self.seen
            .get(task_id)
            .is_some_and(|s| s.hint.is_some() && s.baseline.as_deref() == Some(runner_text))
    }

    /// One pass: end dead runs, then log and hint stalled or slow work.
    /// Returns the dead runs it ended and what it flagged.
    pub fn run_once(
        &mut self,
        conn: &Connection,
        steps: &[(String, Instant)],
        now: Instant,
    ) -> rusqlite::Result<(Vec<String>, Vec<Finding>)> {
        let ended = end_dead_runs(conn)?;
        for id in &ended {
            eprintln!("watchdog: {id} was running with no container; ended as done, failure {DEAD_RUN_FAILURE}, not re-queued");
        }
        let live = live_tasks(conn)?;
        let findings = self.check(&live, steps, now);
        for f in &findings {
            match f.kind {
                FindingKind::StepTimeout => eprintln!(
                    "watchdog: {} step past {}s; hint only, cancelled at {}s",
                    f.task_id,
                    self.cfg.hint_after().as_secs(),
                    self.cfg.step_timeout.as_secs()
                ),
                FindingKind::Stalled => eprintln!(
                    "watchdog: {} no progress for {}s; hint only, state unchanged",
                    f.task_id,
                    self.cfg.stall_after.as_secs()
                ),
            }
            progress::report(conn, &f.task_id, &f.progress())?;
        }
        Ok((ended, findings))
    }
}

/// End every dead run (`running`, `containerUp` false) as `done` with
/// [`DEAD_RUN_FAILURE`]. Compare-and-set: a row that moved meanwhile is left
/// alone. Never re-queues, never starts a container.
pub fn end_dead_runs(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, updated_at FROM tasks WHERE state = 'running' AND container_up = 0
             ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut ended = Vec::new();
    for (id, updated_at) in rows {
        if end_dead_run(conn, &id, &updated_at)? {
            ended.push(id);
        }
    }
    Ok(ended)
}

/// End one dead run. `true` when this call changed it.
pub(crate) fn end_dead_run(
    conn: &Connection,
    task_id: &str,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE tasks SET state = 'done', container_up = 0, failure = ?3, failure_data = ?4, progress = NULL,
                          updated_at = ?2
         WHERE id = ?1 AND state = 'running' AND container_up = 0",
        params![
            task_id,
            newer_than(updated_at),
            DEAD_RUN_FAILURE,
            crate::failure::Failure::no_result().json()
        ],
    )?;
    Ok(changed > 0)
}

fn live_tasks(conn: &Connection) -> rusqlite::Result<Vec<LiveTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, progress FROM tasks WHERE state = 'running' AND container_up = 1
         ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LiveTask {
            id: row.get(0)?,
            progress: row.get(1)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn cfg() -> WatchdogConfig {
        WatchdogConfig {
            step_timeout: 10 * MIN,
            stall_after: 15 * MIN,
        }
    }

    fn live(id: &str, progress: &str) -> LiveTask {
        LiveTask {
            id: id.into(),
            progress: Some(progress.into()),
        }
    }

    #[test]
    fn config_reads_env_and_falls_back() {
        let c = WatchdogConfig::from_vars(|name| match name {
            "ROBROZ_STEP_TIMEOUT_SECS" => Some("30".into()),
            "ROBROZ_STALL_SECS" => Some("0".into()),
            _ => None,
        });
        assert_eq!(c.step_timeout, Duration::from_secs(30));
        assert_eq!(c.stall_after, DEFAULT_STALL_AFTER, "zero keeps the default");
        assert_eq!(
            WatchdogConfig::from_vars(|_| Some("x".into())),
            WatchdogConfig::default()
        );
    }

    #[test]
    fn step_past_half_the_timeout_is_flagged_once() {
        let mut dog = Watchdog::new(cfg());
        assert_eq!(cfg().hint_after(), 5 * MIN);
        let t0 = Instant::now();
        let tasks = [live("t1", "在想怎么回")];
        let steps = [("t1".to_string(), t0)];
        assert!(dog.check(&tasks, &steps, t0 + 4 * MIN).is_empty());
        let found = dog.check(&tasks, &steps, t0 + 5 * MIN);
        assert_eq!(
            found,
            vec![Finding {
                task_id: "t1".into(),
                kind: FindingKind::StepTimeout,
                hint: "模型和工具还没回，已等 5 分钟".into(),
                minutes: 5,
            }]
        );
        // The card now shows the hint; the same step is not flagged again,
        // and a step is never flagged as a stall.
        let shown = [live("t1", &found[0].hint)];
        assert!(dog.check(&shown, &steps, t0 + 60 * MIN).is_empty());
        let tool = [live("t1", "在用 web_search")];
        assert!(dog.check(&tool, &steps, t0 + 61 * MIN).is_empty());
    }

    #[test]
    fn a_new_step_can_be_flagged_again() {
        let mut dog = Watchdog::new(cfg());
        let t0 = Instant::now();
        let tasks = [live("t1", "在想怎么回")];
        assert_eq!(
            dog.check(&tasks, &[("t1".into(), t0)], t0 + 5 * MIN).len(),
            1
        );
        let t1 = t0 + 20 * MIN;
        assert_eq!(
            dog.check(&tasks, &[("t1".into(), t1)], t1 + 5 * MIN).len(),
            1
        );
    }

    #[test]
    fn container_without_progress_is_flagged_as_stalled() {
        let mut dog = Watchdog::new(cfg());
        let t0 = Instant::now();
        let tasks = [live("t1", "在沙箱里跑")];
        assert!(dog.check(&tasks, &[], t0).is_empty());
        assert!(dog.check(&tasks, &[], t0 + 14 * MIN).is_empty());
        let found = dog.check(&tasks, &[], t0 + 15 * MIN);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, FindingKind::Stalled);
        assert_eq!(found[0].hint, "15 分钟没有新进度");
        assert!(dog.holds_hint("t1", "在沙箱里跑"));
        assert!(!dog.holds_hint("t1", "写到第二步"));
        // Our own hint on the card is not progress, and is not repeated.
        let shown = [live("t1", &found[0].hint)];
        assert!(dog.check(&shown, &[], t0 + 40 * MIN).is_empty());
    }

    #[test]
    fn real_progress_resets_the_stall_clock() {
        let mut dog = Watchdog::new(cfg());
        let t0 = Instant::now();
        dog.check(&[live("t1", "第一步")], &[], t0);
        assert!(dog
            .check(&[live("t1", "第二步")], &[], t0 + 14 * MIN)
            .is_empty());
        assert!(dog
            .check(&[live("t1", "第二步")], &[], t0 + 28 * MIN)
            .is_empty());
        assert_eq!(
            dog.check(&[live("t1", "第二步")], &[], t0 + 29 * MIN).len(),
            1
        );
        assert!(!dog.holds_hint("t1", "第三步"));
    }

    #[test]
    fn finished_tasks_are_forgotten() {
        let mut dog = Watchdog::new(cfg());
        let t0 = Instant::now();
        dog.check(&[live("t1", "x")], &[], t0);
        dog.check(&[], &[], t0 + MIN);
        // Seen again (a new run): the clock starts over.
        assert!(dog.check(&[live("t1", "x")], &[], t0 + 20 * MIN).is_empty());
    }

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::init_schema(&conn).unwrap();
        conn
    }

    fn insert(conn: &Connection, id: &str, state: &str, up: bool, progress: Option<&str>) {
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, container_up, created_at, updated_at, progress)
             VALUES (?1, 'g', 'b', 'p', ?2, ?3, '2026-11-24T10:00:00.000Z', '2026-11-24T10:00:00.000Z', ?4)",
            params![id, state, up as i64, progress],
        )
        .unwrap();
    }

    fn row(conn: &Connection, id: &str) -> (String, bool, Option<String>, Option<String>) {
        conn.query_row(
            "SELECT state, container_up, failure, progress FROM tasks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    #[test]
    fn dead_run_ends_done_with_failure_and_is_not_requeued() {
        let conn = db();
        insert(&conn, "dead", "running", false, None);
        insert(&conn, "live", "running", true, Some("在沙箱里跑"));
        insert(&conn, "waiting", "blocked", false, None);
        insert(&conn, "next", "queued", false, None);
        let mut dog = Watchdog::new(cfg());
        let t0 = Instant::now();
        let (ended, _) = dog.run_once(&conn, &[], t0).unwrap();
        assert_eq!(ended, vec!["dead".to_string()]);
        assert_eq!(
            row(&conn, "dead"),
            ("done".into(), false, Some("没写出结果".into()), None)
        );
        assert_eq!(row(&conn, "live").0, "running");
        assert_eq!(
            row(&conn, "waiting").0,
            "blocked",
            "blocked is a person's turn, not a stall"
        );
        assert_eq!(row(&conn, "next").0, "queued");
        // Later passes leave it done: no requeue, nothing else to end.
        for i in 1..4 {
            let (ended, _) = dog.run_once(&conn, &[], t0 + i * MIN).unwrap();
            assert!(ended.is_empty());
        }
        assert_eq!(row(&conn, "dead").0, "done");
        let queued: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE state = 'queued'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(queued, 1, "only the task that was already queued");
    }

    #[test]
    fn hints_only_change_progress_text() {
        let conn = db();
        insert(&conn, "slow", "running", true, Some("在想怎么回"));
        insert(&conn, "stuck", "running", true, Some("在沙箱里跑"));
        let mut dog = Watchdog::new(cfg());
        let t0 = Instant::now();
        let steps = [("slow".to_string(), t0)];
        dog.run_once(&conn, &steps, t0).unwrap();
        let (ended, found) = dog.run_once(&conn, &steps, t0 + 15 * MIN).unwrap();
        assert!(ended.is_empty());
        assert_eq!(found.len(), 2);
        assert_eq!(
            row(&conn, "slow"),
            (
                "running".into(),
                true,
                None,
                Some("模型和工具还没回，已等 15 分钟".into())
            )
        );
        assert_eq!(
            row(&conn, "stuck"),
            (
                "running".into(),
                true,
                None,
                Some("15 分钟没有新进度".into())
            )
        );
    }
}
