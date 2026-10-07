use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hive_policy::{outbound, side_effect, Outbound, SideEffect, SideEffectDecision};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::sandbox::{container_name, result_rel, ContainerPhase, RunRequest, Sandbox};

/// Short, scoped access to the database. `pump` only holds it for SQL and never
/// across a sandbox (docker) call, so the HTTP handlers are not stalled.
pub trait Db {
    fn with<R>(&self, f: impl FnOnce(&Connection) -> R) -> R;
}

impl Db for Connection {
    fn with<R>(&self, f: impl FnOnce(&Connection) -> R) -> R {
        f(self)
    }
}

impl Db for Mutex<Connection> {
    fn with<R>(&self, f: impl FnOnce(&Connection) -> R) -> R {
        let conn = self.lock().expect("sqlite connection lock poisoned");
        f(&conn)
    }
}

/// Where pump finds the group volumes and per-task control dirs.
pub struct PumpConfig<'a> {
    pub volumes_root: &'a Path,
    pub control_root: &'a Path,
    pub image: &'a str,
    pub get_limit: u64,
}

struct TaskRow {
    id: String,
    group_id: String,
    prompt: String,
    updated_at: String,
    briefing: bool,
}

/// What pump decided for an exited container, written back under the lock.
enum Outcome {
    Blocked,
    Dead,
    Done,
}

/// One pass over queued and running tasks on a plain connection (tests).
pub fn pump(
    conn: &Connection,
    sandbox: &mut dyn Sandbox,
    volumes_root: &Path,
    control_root: &Path,
    image: &str,
    get_limit: u64,
) -> rusqlite::Result<()> {
    pump_with(
        conn,
        sandbox,
        &PumpConfig {
            volumes_root,
            control_root,
            image,
            get_limit,
        },
    )
}

/// One pass. Reads under the lock, drops it for docker, re-locks to write.
pub fn pump_with(db: &impl Db, sandbox: &mut dyn Sandbox, cfg: &PumpConfig<'_>) -> rusqlite::Result<()> {
    let queued = db.with(|conn| load_tasks(conn, "queued"))?;
    for task in queued {
        start_task(db, sandbox, cfg, &task)?;
    }

    let running = db.with(|conn| load_tasks(conn, "running"))?;
    for task in running {
        collect_task(db, sandbox, cfg, &task)?;
    }
    Ok(())
}

fn start_task(
    db: &impl Db,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
    task: &TaskRow,
) -> rusqlite::Result<()> {
    let Some(group_dir) = ensure_dir(&cfg.volumes_root.join(&task.group_id)) else {
        return Ok(());
    };
    let Some(control_dir) = ensure_dir(&cfg.control_root.join(&task.id)) else {
        return Ok(());
    };
    if let Err(err) = write_prompt(&group_dir, &task.id, &task.prompt) {
        eprintln!("pump: write prompt for {}: {err}; task stays queued", task.id);
        return Ok(());
    }

    // Claim before docker run so a second pass can't pick the task again.
    let claimed_at = newer_than(&task.updated_at);
    let claimed = db.with(|conn| {
        conn.execute(
            "UPDATE tasks SET state = 'running', container_up = 1, updated_at = ?1
             WHERE id = ?2 AND state = 'queued'",
            params![claimed_at, task.id],
        )
    })?;
    if claimed == 0 {
        return Ok(());
    }

    let name = container_name(&task.id);
    let started = sandbox.start(&RunRequest {
        name: &name,
        image: cfg.image,
        group_dir: &group_dir,
        control_dir: &control_dir,
        task_id: &task.id,
    });
    if let Err(err) = started {
        eprintln!("pump: start {} failed: {}; task stays queued", task.id, err.message);
        let reverted_at = newer_than(&claimed_at);
        db.with(|conn| {
            conn.execute(
                "UPDATE tasks SET state = 'queued', container_up = 0, updated_at = ?1
                 WHERE id = ?2 AND state = 'running'",
                params![reverted_at, task.id],
            )
        })?;
    }
    Ok(())
}

fn collect_task(
    db: &impl Db,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
    task: &TaskRow,
) -> rusqlite::Result<()> {
    let name = container_name(&task.id);
    let phase = match sandbox.phase(&name) {
        Ok(phase) => phase,
        Err(err) => {
            eprintln!("pump: inspect {}: {}", task.id, err.message);
            return Ok(());
        }
    };
    let ContainerPhase::Exited { code } = phase else {
        return Ok(());
    };

    // Workspace reads of the group volume are allowed. This is not a scheduler.
    if side_effect(SideEffect::WorkspaceRead) != SideEffectDecision::Allow {
        eprintln!("pump: workspace read denied for {}", task.id);
        return Ok(());
    }

    let group_dir = group_volume(cfg.volumes_root, &task.group_id);
    let rel = result_rel(&task.id);
    let result_file = group_dir.join(&rel);
    let intent = read_intent(&group_dir, &task.id);
    let (get_count, quota_fail) = outbound_record(&group_dir, &task.id, cfg.get_limit);

    let block = match intent.as_deref() {
        Some("external") => {
            side_effect(SideEffect::ExternalWrite) == SideEffectDecision::BlockAndDropContainer
        }
        Some("captcha") => side_effect(SideEffect::Captcha) == SideEffectDecision::BlockAndDropContainer,
        _ => false,
    };

    if let Err(err) = sandbox.remove(&name) {
        eprintln!("pump: remove {}: {}", task.id, err.message);
        return Ok(());
    }

    let outcome = if block {
        Outcome::Blocked
    } else if quota_fail || code != 0 || !result_file.is_file() {
        // Container is gone and there is no successful result. Stay running
        // with containerUp false so the card uses the dead-run line. Not blocked.
        Outcome::Dead
    } else if side_effect(SideEffect::WorkspaceWrite) != SideEffectDecision::Allow {
        Outcome::Blocked
    } else {
        Outcome::Done
    };

    let updated_at = newer_than(&task.updated_at);
    db.with(|conn| write_outcome(conn, task, &outcome, get_count, &updated_at, &rel))
}

fn write_outcome(
    conn: &Connection,
    task: &TaskRow,
    outcome: &Outcome,
    get_count: i64,
    updated_at: &str,
    rel: &str,
) -> rusqlite::Result<()> {
    match outcome {
        Outcome::Blocked => {
            conn.execute(
                "UPDATE tasks SET state = 'blocked', container_up = 0, get_count = ?1, updated_at = ?2
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
        }
        Outcome::Dead => {
            conn.execute(
                "UPDATE tasks SET container_up = 0, get_count = ?1, updated_at = ?2
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
        }
        Outcome::Done => {
            let delivery_id = Uuid::new_v4().to_string();
            let tx = conn.unchecked_transaction()?;
            let changed = tx.execute(
                "UPDATE tasks SET state = 'done', container_up = 0, get_count = ?1, updated_at = ?2
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
            if changed > 0 {
                tx.execute(
                    "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at)
                     VALUES (?1, ?2, ?3, '已写入结果', ?4, ?5, ?6)",
                    params![delivery_id, task.group_id, task.id, rel, task.briefing as i64, updated_at],
                )?;
            }
            tx.commit()?;
        }
    }
    Ok(())
}

/// Running tasks with a live container. Dead runs (containerUp false) have
/// no container left to inspect.
fn load_tasks(conn: &Connection, state: &str) -> rusqlite::Result<Vec<TaskRow>> {
    let live_only = if state == "running" { " AND container_up = 1" } else { "" };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, group_id, prompt, updated_at, briefing FROM tasks
         WHERE state = ?1{live_only} ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([state], |row| {
        Ok(TaskRow {
            id: row.get(0)?,
            group_id: row.get(1)?,
            prompt: row.get(2)?,
            updated_at: row.get(3)?,
            briefing: row.get::<_, i64>(4)? != 0,
        })
    })?;
    rows.collect()
}

/// The runner reads `/workspace/tasks/{taskId}/prompt.txt`. Rewritten on every
/// start so the volume copy always matches the stored prompt.
fn write_prompt(group_dir: &Path, task_id: &str, prompt: &str) -> std::io::Result<()> {
    let path = hive_runner::prompt_path(group_dir, task_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, prompt)
}

fn ensure_dir(dir: &Path) -> Option<PathBuf> {
    if let Err(err) = std::fs::create_dir_all(dir) {
        eprintln!("pump: create {}: {err}", dir.display());
        return None;
    }
    match std::fs::canonicalize(dir) {
        Ok(dir) => Some(dir),
        Err(err) => {
            eprintln!("pump: canonicalize {}: {err}", dir.display());
            None
        }
    }
}

pub(crate) fn group_volume(volumes_root: &Path, group_id: &str) -> PathBuf {
    let dir = volumes_root.join(group_id);
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

/// Kind of the blocking action, from the first line of `intent`.
fn read_intent(group_dir: &Path, task_id: &str) -> Option<String> {
    let path = hive_runner::intent_path(group_dir, task_id);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| hive_runner::parse_intent(&text))
        .map(|intent| intent.kind)
}

/// Count `GET` lines and ask hive-policy. A non-GET line is Fail and is not blocked.
fn outbound_record(group_dir: &Path, task_id: &str, limit: u64) -> (i64, bool) {
    let path = group_dir.join("tasks").join(task_id).join("outbound.txt");
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return (0, false),
    };
    let mut gets: u64 = 0;
    let mut non_get_fail = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "GET" {
            gets += 1;
        } else if outbound(line, 0, limit) == Outbound::Fail {
            non_get_fail = true;
        }
    }
    let over = gets > 0 && outbound("GET", gets, limit) == Outbound::Fail;
    (gets as i64, over || non_get_fail)
}

fn newer_than(previous: &str) -> String {
    let now = Utc::now();
    let prev = DateTime::parse_from_rfc3339(previous)
        .map(|stamp| stamp.with_timezone(&Utc))
        .unwrap_or(now);
    let stamped = if now <= prev {
        prev + Duration::milliseconds(1)
    } else {
        now
    };
    stamped.to_rfc3339_opts(SecondsFormat::Millis, true)
}
