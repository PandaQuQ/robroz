use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use robroz_policy::{blocking_effect, meter, tier, OverQuota, SideEffect, Tier};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::progress;
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
    /// `None`: no model, the runner echoes the prompt.
    pub model: Option<&'a dyn robroz_model::Model>,
}

struct TaskRow {
    id: String,
    group_id: String,
    assignee_bot_id: String,
    prompt: String,
    updated_at: String,
    briefing: bool,
}

/// What pump decided for an exited container, written back under the lock.
enum Outcome {
    /// With the host-computed blocking line, when there is one.
    Blocked(Option<robroz_policy::BlockingLine>),
    /// Done without the work, with the task's `failure` (e.g. over the GET
    /// quota). No delivery, never blocked.
    Failed(String),
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
            model: None,
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
    if let Err(err) = write_prompt(&control_dir, &task.prompt) {
        eprintln!("pump: write prompt for {}: {err}; task stays queued", task.id);
        return Ok(());
    }
    // A leftover intent would make this run look blocked again.
    if let Err(err) = remove_intent(&group_dir, &task.id) {
        eprintln!("pump: clear old intent for {}: {err}; task stays queued", task.id);
        return Ok(());
    }
    // A leftover progress file would put the last run's text on the new card.
    if let Err(err) = remove_progress(&group_dir, &task.id) {
        eprintln!("pump: clear old progress for {}: {err}", task.id);
    }
    // Same for proposals: only this run's lines may raise confirm cards.
    if let Err(err) = crate::proposals::remove(&group_dir, &task.id) {
        eprintln!("pump: clear old proposals for {}: {err}", task.id);
    }

    // Claim before docker run so a second pass can't pick the task again.
    // The first progress rides on the claim itself: one status event, not two.
    // Memories and skills are loaded in the same lock, before the claim, so a
    // load error leaves the task queued.
    let claimed_at = newer_than(&task.updated_at);
    let first = if cfg.model.is_some() { progress::THINKING } else { progress::STARTED };
    let (claimed, memory_section) = db.with(|conn| -> rusqlite::Result<_> {
        let section = if cfg.model.is_some() {
            system_extra(
                crate::memories::prompt_section(&crate::memories::kept_for(conn, &task.assignee_bot_id)?),
                crate::skills::prompt_section(&crate::skills::kept_for(conn, &task.assignee_bot_id, &task.group_id)?),
            )
        } else {
            None
        };
        let claimed = conn.execute(
            "UPDATE tasks SET state = 'running', container_up = 1, updated_at = ?1, failure = NULL, progress = ?3
             WHERE id = ?2 AND state = 'queued'",
            params![claimed_at, task.id, first],
        )?;
        Ok((claimed, section))
    })?;
    if claimed == 0 {
        return Ok(());
    }
    let mut reply_progress = None;
    if let Some(model) = cfg.model {
        let reply = write_reply(&control_dir, &task.id, &task.prompt, memory_section.as_deref(), model);
        reply_progress = reply.progress;
        if let Some(failure) = reply.failure {
            db.with(|conn| {
                conn.execute("UPDATE tasks SET failure = ?1 WHERE id = ?2", params![failure, task.id])
            })?;
        }
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
                "UPDATE tasks SET state = 'queued', container_up = 0, updated_at = ?1, progress = NULL
                 WHERE id = ?2 AND state = 'running'",
                params![reverted_at, task.id],
            )
        })?;
        return Ok(());
    }
    // Key moment: the sandbox is up. The model's own progress line wins.
    let text = reply_progress.as_deref().unwrap_or(progress::IN_SANDBOX);
    db.with(|conn| progress::set_progress(conn, &task.id, text))?;
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
    let code = match phase {
        // Still going: pass on what the runner last wrote, if it changed.
        ContainerPhase::Running => {
            let group_dir = group_volume(cfg.volumes_root, &task.group_id);
            if let Some(text) = read_progress(&group_dir, &task.id) {
                db.with(|conn| progress::set_progress(conn, &task.id, &text))?;
            }
            return Ok(());
        }
        ContainerPhase::Exited { code } => code,
        // The container vanished mid-run (host or docker restart, manual rm).
        // Never re-queue: the run may have half-changed the volume, and a rerun
        // would repeat it. Mark it a dead run and leave it for a person.
        ContainerPhase::Missing => {
            eprintln!("pump: container for {} is gone; marking interrupted, not rerunning", task.id);
            let updated_at = newer_than(&task.updated_at);
            let rel = result_rel(&task.id);
            return db.with(|conn| write_outcome(conn, task, &Outcome::Dead, 0, &updated_at, &rel, &[]));
        }
    };

    // Tier Allow: workspace reads of the group volume, nobody is told.
    if tier(SideEffect::WorkspaceRead) != Tier::Allow {
        eprintln!("pump: workspace read denied for {}", task.id);
        return Ok(());
    }

    let group_dir = group_volume(cfg.volumes_root, &task.group_id);
    let rel = result_rel(&task.id);
    let result_file = group_dir.join(&rel);
    // `intent` only signals "blocked"; its content is never read. Where the task
    // stopped is recomputed here from the stored prompt and the host's takeover
    // hashes, with the same function the runner uses.
    let claims_blocked = std::fs::symlink_metadata(robroz_runner::intent_path(&group_dir, &task.id)).is_ok();
    let stop = if claims_blocked {
        let approved = robroz_runner::read_takeover(&cfg.control_root.join(&task.id));
        robroz_policy::blocking_line(&task.prompt, &approved)
    } else {
        None
    };
    // Tier Metered: GET only, counted per task.
    let (get_count, over_quota) = outbound_record(&group_dir, &task.id, cfg.get_limit);

    // Tier Takeover: external writes and captchas.
    let block = stop
        .as_ref()
        .and_then(|stop| blocking_effect(stop.kind))
        .is_some_and(|effect| tier(effect) == Tier::Takeover);
    // Claims to be blocked, but every blocking line is already taken over (or there
    // are none): not blocked, a run that produced no result.
    let bogus_block = claims_blocked && stop.is_none();

    if let Err(err) = sandbox.remove(&name) {
        eprintln!("pump: remove {}: {}", task.id, err.message);
        return Ok(());
    }

    let outcome = if block {
        Outcome::Blocked(stop)
    } else if let Some(over) = over_quota {
        Outcome::Failed(over.failure())
    } else if bogus_block || code != 0 || !result_file.is_file() {
        // Container is gone and there is no successful result. Stay running
        // with containerUp false so the card uses the dead-run line. Not blocked.
        Outcome::Dead
    } else if tier(SideEffect::WorkspaceWrite) != Tier::Allow {
        Outcome::Blocked(None)
    } else {
        Outcome::Done
    };

    // What the bro proposed to remember; only a finished run counts. Read
    // here, outside the lock; stored with the outcome.
    let proposals = if matches!(outcome, Outcome::Done) {
        crate::proposals::read(&group_dir, &task.id)
    } else {
        Vec::new()
    };
    let updated_at = newer_than(&task.updated_at);
    db.with(|conn| write_outcome(conn, task, &outcome, get_count, &updated_at, &rel, &proposals))?;
    if let Err(err) = crate::proposals::remove(&group_dir, &task.id) {
        // The next start clears it again before docker run.
        eprintln!("pump: remove proposals for {}: {err}", task.id);
    }
    if let Err(err) = remove_intent(&group_dir, &task.id) {
        // The next start clears it again before docker run.
        eprintln!("pump: remove intent for {}: {err}", task.id);
    }
    Ok(())
}

fn write_outcome(
    conn: &Connection,
    task: &TaskRow,
    outcome: &Outcome,
    get_count: i64,
    updated_at: &str,
    rel: &str,
    proposals: &[robroz_contract::Proposal],
) -> rusqlite::Result<()> {
    match outcome {
        Outcome::Blocked(stop) => {
            let tx = conn.unchecked_transaction()?;
            let changed = tx.execute(
                "UPDATE tasks SET state = 'blocked', container_up = 0, get_count = ?1, updated_at = ?2, progress = NULL,
                                  intent_kind = ?4, intent_line = ?5, intent_hash = ?6
                 WHERE id = ?3 AND state = 'running'",
                params![
                    get_count,
                    updated_at,
                    task.id,
                    stop.as_ref().map(|v| v.kind),
                    stop.as_ref().map(|v| v.line.as_str()),
                    stop.as_ref().map(|v| v.hash.as_str()),
                ],
            )?;
            // Writing outside asks first: an approval card the person answers.
            if let Some(stop) = stop.as_ref().filter(|s| changed > 0 && crate::approvals::needs_approval(s.kind)) {
                crate::approvals::gate_task(&tx, &task.id, &task.prompt, &stop.hash, updated_at)?;
            }
            tx.commit()?;
        }
        Outcome::Failed(failure) => {
            conn.execute(
                "UPDATE tasks SET state = 'done', container_up = 0, get_count = ?1, updated_at = ?2, failure = ?4, progress = NULL
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id, failure],
            )?;
        }
        Outcome::Dead => {
            conn.execute(
                "UPDATE tasks SET container_up = 0, get_count = ?1, updated_at = ?2, progress = NULL
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
        }
        Outcome::Done => {
            let delivery_id = Uuid::new_v4().to_string();
            let tx = conn.unchecked_transaction()?;
            let changed = tx.execute(
                "UPDATE tasks SET state = 'done', container_up = 0, get_count = ?1, updated_at = ?2, progress = NULL
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
            if changed > 0 {
                tx.execute(
                    "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at, failure)
                     VALUES (?1, ?2, ?3, '已写入结果', ?4, ?5, ?6, (SELECT failure FROM tasks WHERE id = ?3))",
                    params![delivery_id, task.group_id, task.id, rel, task.briefing as i64, updated_at],
                )?;
                // One pending memory and one confirm card per good proposal.
                crate::proposals::ingest(&tx, &task.id, proposals, updated_at)?;
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
        "SELECT id, group_id, assignee_bot_id, prompt, updated_at, briefing FROM tasks
         WHERE state = ?1{live_only} ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([state], |row| {
        Ok(TaskRow {
            id: row.get(0)?,
            group_id: row.get(1)?,
            assignee_bot_id: row.get(2)?,
            prompt: row.get(3)?,
            updated_at: row.get(4)?,
            briefing: row.get::<_, i64>(5)? != 0,
        })
    })?;
    rows.collect()
}

/// The runner reads `/control/prompt.txt`: the task's control dir, outside the
/// group volume and mounted read-only, so no container can change it.
fn write_prompt(control_dir: &Path, prompt: &str) -> std::io::Result<()> {
    std::fs::write(robroz_runner::prompt_path(control_dir), prompt)
}

/// Delete `tasks/{taskId}/intent` from the volume. Missing is fine.
/// Join the memory and skill paragraphs, in that order. Missing ones are omitted.
fn system_extra(memory: Option<String>, skill: Option<String>) -> Option<String> {
    let parts: Vec<String> = [memory, skill].into_iter().flatten().filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Ask the model outside the DB lock and leave its answer for the runner.
/// A failed call becomes the result text, so the task still finishes and the
/// person sees why; the reason is also returned so the delivery can say the
/// task was not done. Old replies are removed first so a retry never reuses one.
/// The reply's last `robroz-progress:` line is returned for the status card.
fn write_reply(
    control_dir: &Path,
    task_id: &str,
    prompt: &str,
    memory_section: Option<&str>,
    model: &dyn robroz_model::Model,
) -> Reply {
    let path = robroz_runner::reply_path(control_dir);
    let _ = std::fs::remove_file(&path);
    let (text, failure) = match model.reply(prompt, memory_section) {
        Ok(text) => (text, None),
        Err(err) => {
            eprintln!("pump: model for {task_id}: {err}");
            (format!("模型没回：{err}"), Some(format!("模型没回：{err}")))
        }
    };
    let progress = robroz_runner::last_progress(&text).map(str::to_string);
    if let Err(err) = std::fs::write(&path, text) {
        eprintln!("pump: write reply for {task_id}: {err}; runner will echo");
    }
    Reply { failure, progress }
}

/// What [`write_reply`] hands back to pump.
struct Reply {
    failure: Option<String>,
    progress: Option<String>,
}

/// The runner's latest progress text, if it wrote one.
fn read_progress(group_dir: &Path, task_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(robroz_runner::progress_path(group_dir, task_id)).ok()?;
    progress::normalize(&text)
}

fn remove_progress(group_dir: &Path, task_id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(robroz_runner::progress_path(group_dir, task_id)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

fn remove_intent(group_dir: &Path, task_id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(robroz_runner::intent_path(group_dir, task_id)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
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

/// Read `outbound.txt` and ask [`meter`]: the GET count, and why the task
/// fails, if it does (over the limit, or not a GET). Never blocks.
fn outbound_record(group_dir: &Path, task_id: &str, limit: u64) -> (i64, Option<OverQuota>) {
    let text = std::fs::read_to_string(robroz_runner::outbound_path(group_dir, task_id)).unwrap_or_default();
    let methods = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let gets = methods.clone().filter(|m| *m == "GET").count() as i64;
    match meter(methods, limit) {
        Ok(_) => (gets, None),
        Err(over) => (gets, Some(over)),
    }
}

pub(crate) fn newer_than(previous: &str) -> String {
    let now = Utc::now();
    let prev = DateTime::parse_from_rfc3339(previous)
        .map(|stamp| stamp.with_timezone(&Utc))
        .unwrap_or(now);
    // Compare at millisecond precision, the precision we store: `now` a
    // fraction of a millisecond after `prev` would print the same stamp.
    let stamped = if now.timestamp_millis() <= prev.timestamp_millis() {
        prev + Duration::milliseconds(1)
    } else {
        now
    };
    stamped.to_rfc3339_opts(SecondsFormat::Millis, true)
}
