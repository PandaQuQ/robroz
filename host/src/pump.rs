use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use robroz_policy::{blocking_effect, meter, tier, OverQuota, SideEffect, Tier};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::progress;
use crate::sandbox::{container_name, result_rel, ContainerPhase, RunRequest, Sandbox};
use crate::steps::StepRunner;
use crate::watchdog::{Watchdog, WatchdogConfig, DEAD_RUN_FAILURE};

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
    /// `None`: no live model. See [`Self::echo_prompt`].
    pub model: Option<&'a dyn robroz_model::Model>,
    /// When `model` is `None`, `true` keeps the runner echoing the prompt
    /// (`ROBROZ_MODEL=echo` and the unit-test `pump` helper). `false` writes
    /// 「还没设置模型」and sets `needsSetup: "model"`.
    pub echo_prompt: bool,
    /// Tools the model may call. Empty: the request has no `tools` field.
    pub tools: &'a crate::tools::ToolRegistry,
    /// MCP servers enabled for a task's group. `None` skips MCP registration.
    pub mcp: Option<Arc<dyn crate::mcp_tools::McpRuntime>>,
}

#[derive(Clone)]
struct TaskRow {
    id: String,
    group_id: String,
    assignee_bot_id: String,
    prompt: String,
    updated_at: String,
    briefing: bool,
    get_count: i64,
}

/// What pump decided for an exited container, written back under the lock.
enum Outcome {
    /// With the host-computed blocking line, when there is one.
    Blocked(Option<robroz_policy::BlockingLine>),
    /// Done without the work, with the task's `failure` (e.g. over the GET
    /// quota). No delivery, never blocked.
    Failed(String),
    /// Exited without a result, or vanished: `done` + [`DEAD_RUN_FAILURE`].
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
            echo_prompt: true,
            tools: crate::tools::ToolRegistry::empty(),
            mcp: None,
        },
    )
}

/// One pass. Reads under the lock, drops it for docker, re-locks to write.
/// The model step runs inline on the calling thread, so one call finishes the
/// whole start. Tests use this; the binary uses [`pump_offloaded`].
pub fn pump_with(
    db: &impl Db,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
) -> rusqlite::Result<()> {
    pump_inner(db, sandbox, cfg, None)
}

/// State that outlives one pump pass when steps run off the pump thread: the
/// steps in flight and the [`Watchdog`] that watches them.
pub(crate) struct Offload {
    steps: StepRunner<StepDone>,
    watchdog: Watchdog,
}

impl Offload {
    pub(crate) fn new(cfg: WatchdogConfig) -> Self {
        Self {
            steps: StepRunner::new(),
            watchdog: Watchdog::new(cfg),
        }
    }

    /// One Watchdog pass at `now` over the database and the steps in flight.
    pub(crate) fn watchdog_once(
        &mut self,
        conn: &Connection,
        now: Instant,
    ) -> rusqlite::Result<()> {
        let steps: Vec<(String, Instant)> = self
            .steps
            .in_flight()
            .map(|(id, at)| (id.to_string(), at))
            .collect();
        self.watchdog.run_once(conn, &steps, now).map(|_| ())
    }
}

/// A step that finished off the pump thread, with what pump needs to go on.
struct StepDone {
    started: Started,
    reply: Reply,
}

/// What the off-thread mode needs: a shareable DB for progress lines, an
/// owned model, and the state kept between passes.
struct OffloadCtx<'s> {
    db: Arc<Mutex<Connection>>,
    /// `None` without a live model: nothing to hand off this pass, but steps
    /// from earlier passes are still drained and never collected as dead.
    model: Option<Arc<dyn robroz_model::Model>>,
    state: &'s mut Offload,
}

/// One pass where the model and its tool calls run on their own threads. The
/// pump thread only claims tasks, hands the step off, starts containers for
/// steps that finished, and collects exits. `cfg.model` must be `model`.
pub(crate) fn pump_offloaded(
    db: &Arc<Mutex<Connection>>,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
    model: Option<Arc<dyn robroz_model::Model>>,
    state: &mut Offload,
) -> rusqlite::Result<()> {
    let ctx = OffloadCtx {
        db: db.clone(),
        model,
        state,
    };
    pump_inner(&**db, sandbox, cfg, Some(ctx))
}

fn pump_inner(
    db: &impl Db,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
    mut offload: Option<OffloadCtx<'_>>,
) -> rusqlite::Result<()> {
    if let Some(off) = offload.as_mut() {
        for (_, done) in off.state.steps.drain() {
            let still_running = db.with(|conn| is_live(conn, &done.started.task.id))?;
            if !still_running {
                eprintln!(
                    "pump: {} moved while its step ran; dropping the reply",
                    done.started.task.id
                );
                continue;
            }
            finish_start(db, sandbox, cfg, &done.started, Some(done.reply))?;
        }
    }

    let queued = db.with(|conn| load_tasks(conn, "queued"))?;
    for task in queued {
        start_task(db, sandbox, cfg, offload.as_mut(), &task)?;
    }

    let running = db.with(|conn| load_tasks(conn, "running"))?;
    for task in running {
        let watchdog = match offload.as_ref() {
            // No container yet: its step is still running.
            Some(off) if off.state.steps.is_running(&task.id) => continue,
            Some(off) => Some(&off.state.watchdog),
            None => None,
        };
        collect_task(db, sandbox, cfg, watchdog, &task)?;
    }
    Ok(())
}

fn is_live(conn: &Connection, task_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE id = ?1 AND state = 'running' AND container_up = 1",
        [task_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
}

/// A claimed task, between the claim and `docker run`.
struct Started {
    task: TaskRow,
    claimed_at: String,
    mcp_resume: bool,
    control_dir: PathBuf,
    group_dir: PathBuf,
}

fn start_task(
    db: &impl Db,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
    offload: Option<&mut OffloadCtx<'_>>,
    task: &TaskRow,
) -> rusqlite::Result<()> {
    let Some(group_dir) = ensure_dir(&cfg.volumes_root.join(&task.group_id)) else {
        return Ok(());
    };
    let Some(control_dir) = ensure_dir(&cfg.control_root.join(&task.id)) else {
        return Ok(());
    };
    if let Err(err) = write_prompt(&control_dir, &task.prompt) {
        eprintln!(
            "pump: write prompt for {}: {err}; task stays queued",
            task.id
        );
        return Ok(());
    }
    let mcp_resume = crate::mcp_tools::pending_is_approved(&control_dir);
    // A leftover intent would make this run look blocked again.
    if let Err(err) = remove_intent(&group_dir, &task.id) {
        eprintln!(
            "pump: clear old intent for {}: {err}; task stays queued",
            task.id
        );
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
    let first = if cfg.model.is_some() {
        progress::THINKING
    } else {
        progress::STARTED
    };
    let (claimed, memory_section) = db.with(|conn| -> rusqlite::Result<_> {
        let section = if cfg.model.is_some() {
            system_extra(
                crate::memories::prompt_section(&crate::memories::kept_for(conn, &task.assignee_bot_id)?),
                crate::skills::prompt_section(&crate::skills::kept_for(conn, &task.assignee_bot_id, &task.group_id)?),
            )
        } else {
            None
        };
        let claimed = if mcp_resume {
            conn.execute(
                "UPDATE tasks SET state = 'running', container_up = 1, updated_at = ?1, failure = NULL, progress = ?3
                 WHERE id = ?2 AND state = 'queued'",
                params![claimed_at, task.id, first],
            )?
        } else {
            conn.execute(
                "UPDATE tasks SET state = 'running', container_up = 1, updated_at = ?1, failure = NULL, progress = ?3,
                                  get_count = 0, needs_setup = NULL
                 WHERE id = ?2 AND state = 'queued'",
                params![claimed_at, task.id, first],
            )?
        };
        Ok((claimed, section))
    })?;
    if claimed == 0 {
        return Ok(());
    }
    let started = Started {
        task: task.clone(),
        claimed_at,
        mcp_resume,
        control_dir,
        group_dir,
    };
    let Some(model) = cfg.model else {
        return finish_start(db, sandbox, cfg, &started, None);
    };
    let mut tools = cfg.tools.clone();
    if let Some(mcp) = cfg.mcp.clone() {
        let servers = db.with(|conn| crate::mcp::ready_servers_for_group(conn, &task.group_id))?;
        crate::mcp_tools::register_tools(&mut tools, &servers, mcp);
    }
    let offload = offload.and_then(|off| off.model.clone().map(|model| (off, model)));
    match offload {
        None => {
            let mut on_progress = |line: &str| {
                if let Err(err) = db.with(|conn| progress::set_progress(conn, &task.id, line)) {
                    eprintln!("pump: progress for {}: {err}", task.id);
                }
            };
            let reply = write_reply(
                model,
                &started.control_dir,
                &task.id,
                &task.prompt,
                memory_section.as_deref(),
                cfg.get_limit,
                &tools,
                &mut on_progress,
            );
            finish_start(db, sandbox, cfg, &started, Some(reply))
        }
        Some((off, model)) => {
            // The slow part leaves the pump thread; a later pass picks it up.
            let shared = off.db.clone();
            let get_limit = cfg.get_limit;
            let task_id = task.id.clone();
            let spawned = off.state.steps.spawn(&task.id, move || {
                let mut on_progress = |line: &str| {
                    if let Err(err) =
                        shared.with(|conn| progress::set_progress(conn, &task_id, line))
                    {
                        eprintln!("pump: progress for {task_id}: {err}");
                    }
                };
                let reply = write_reply(
                    &*model,
                    &started.control_dir,
                    &started.task.id,
                    &started.task.prompt,
                    memory_section.as_deref(),
                    get_limit,
                    &tools,
                    &mut on_progress,
                );
                StepDone { started, reply }
            });
            if !spawned {
                // Could not hand it off: put the claim back, try next pass.
                let reverted_at = newer_than(&task.updated_at);
                db.with(|conn| {
                    conn.execute(
                        "UPDATE tasks SET state = 'queued', container_up = 0, updated_at = ?1, progress = NULL
                         WHERE id = ?2 AND state = 'running'",
                        params![reverted_at, task.id],
                    )
                })?;
            }
            Ok(())
        }
    }
}

/// After the model step (or right away without a model): record the reply,
/// block or fail on it if needed, otherwise `docker run`.
fn finish_start(
    db: &impl Db,
    sandbox: &mut dyn Sandbox,
    cfg: &PumpConfig<'_>,
    started: &Started,
    reply: Option<Reply>,
) -> rusqlite::Result<()> {
    let Started {
        task,
        claimed_at,
        mcp_resume,
        control_dir,
        group_dir,
    } = started;
    let (claimed_at, mcp_resume) = (claimed_at.as_str(), *mcp_resume);
    let mut reply_progress = None;
    if let Some(reply) = reply {
        reply_progress = reply.progress.clone();
        db.with(|conn| {
            conn.execute(
                "UPDATE tasks SET get_count = ?1, needs_setup = ?2, failure = COALESCE(?3, failure) WHERE id = ?4",
                params![reply.get_count as i64, reply.needs_setup, reply.failure, task.id],
            )
        })?;
        if let Some(pending) = reply.pending {
            let updated_at = newer_than(claimed_at);
            if let Err(err) = crate::mcp_tools::save_pending(control_dir, &pending) {
                eprintln!("pump: save mcp pending for {}: {err}", task.id);
                db.with(|conn| {
                    write_outcome(
                        conn,
                        task,
                        &Outcome::Failed(format!("无法保存审批状态：{err}")),
                        reply.get_count as i64,
                        &updated_at,
                        &crate::sandbox::result_rel(&task.id),
                        &[],
                    )
                })?;
                return Ok(());
            }
            db.with(|conn| {
                block_mcp_task(conn, task, &pending, &updated_at, reply.get_count as i64)
            })?;
            return Ok(());
        }
        if mcp_resume {
            let _ = crate::mcp_tools::remove_pending(control_dir);
            db.with(|conn| {
                conn.execute(
                    "UPDATE tasks SET intent_kind = NULL, intent_line = NULL, intent_hash = NULL WHERE id = ?1",
                    [&task.id],
                )
            })?;
        }
        if reply.over_quota {
            let updated_at = newer_than(claimed_at);
            db.with(|conn| {
                write_outcome(
                    conn,
                    task,
                    &Outcome::Failed(reply.failure.clone().unwrap_or_else(|| "出网超额".into())),
                    reply.get_count as i64,
                    &updated_at,
                    &crate::sandbox::result_rel(&task.id),
                    &[],
                )
            })?;
            return Ok(());
        }
    } else if !cfg.echo_prompt {
        let path = robroz_runner::reply_path(control_dir);
        let _ = std::fs::remove_file(&path);
        if let Err(err) = std::fs::write(&path, crate::model_settings::NOT_CONFIGURED) {
            eprintln!("pump: write needs-setup reply for {}: {err}", task.id);
        }
        db.with(|conn| {
            conn.execute(
                "UPDATE tasks SET needs_setup = ?1 WHERE id = ?2",
                params!["model", task.id],
            )
        })?;
    }

    let name = container_name(&task.id);
    let started = sandbox.start(&RunRequest {
        name: &name,
        image: cfg.image,
        group_dir,
        control_dir,
        task_id: &task.id,
    });
    if let Err(err) = started {
        eprintln!(
            "pump: start {} failed: {}; task stays queued",
            task.id, err.message
        );
        let reverted_at = newer_than(claimed_at);
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
    watchdog: Option<&Watchdog>,
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
                // Keep the Watchdog's hint up until the runner says something new.
                if !watchdog.is_some_and(|w| w.holds_hint(&task.id, &text)) {
                    db.with(|conn| progress::set_progress(conn, &task.id, &text))?;
                }
            }
            return Ok(());
        }
        ContainerPhase::Exited { code } => code,
        // The container vanished mid-run (host or docker restart, manual rm).
        // Never re-queue: the run may have half-changed the volume, and a rerun
        // would repeat it. End it as a dead run: `done` with 「没写出结果」.
        ContainerPhase::Missing => {
            eprintln!(
                "pump: container for {} is gone; ending as {DEAD_RUN_FAILURE}, not rerunning",
                task.id
            );
            let updated_at = newer_than(&task.updated_at);
            let rel = result_rel(&task.id);
            return db
                .with(|conn| write_outcome(conn, task, &Outcome::Dead, 0, &updated_at, &rel, &[]));
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
    let claims_blocked =
        std::fs::symlink_metadata(robroz_runner::intent_path(&group_dir, &task.id)).is_ok();
    let stop = if claims_blocked {
        let approved = robroz_runner::read_takeover(&cfg.control_root.join(&task.id));
        robroz_policy::blocking_line(&task.prompt, &approved)
    } else {
        None
    };
    // Tier Metered: GET only, counted per task.
    let (get_count, over_quota) =
        outbound_record(&group_dir, &task.id, task.get_count as u64, cfg.get_limit);

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
        // Container is gone and there is no successful result: a dead run,
        // ended as `done` with 「没写出结果」. Not blocked, not re-queued.
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
    db.with(|conn| {
        write_outcome(
            conn,
            task,
            &outcome,
            get_count,
            &updated_at,
            &rel,
            &proposals,
        )
    })?;
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
            if let Some(stop) = stop
                .as_ref()
                .filter(|s| changed > 0 && crate::approvals::needs_approval(s.kind))
            {
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
        // Compare-and-set on `running`: a late writer cannot end it twice.
        // No delivery, never re-queued; same shape as `Failed`.
        Outcome::Dead => {
            conn.execute(
                "UPDATE tasks SET state = 'done', container_up = 0, get_count = ?1, updated_at = ?2, failure = ?4, progress = NULL
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id, DEAD_RUN_FAILURE],
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
                    "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at, failure, needs_setup)
                     VALUES (?1, ?2, ?3, '已写入结果', ?4, ?5, ?6,
                             (SELECT failure FROM tasks WHERE id = ?3),
                             (SELECT needs_setup FROM tasks WHERE id = ?3))",
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
    let live_only = if state == "running" {
        " AND container_up = 1"
    } else {
        ""
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, group_id, assignee_bot_id, prompt, updated_at, briefing, get_count FROM tasks
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
            get_count: row.get(6)?,
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
    let parts: Vec<String> = [memory, skill]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
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
/// Runs on the pump thread or on a step thread: it touches the database only
/// through `on_progress`.
#[allow(clippy::too_many_arguments)]
fn write_reply(
    model: &dyn robroz_model::Model,
    control_dir: &Path,
    task_id: &str,
    prompt: &str,
    memory_section: Option<&str>,
    get_limit: u64,
    tools: &crate::tools::ToolRegistry,
    on_progress: &mut dyn FnMut(&str),
) -> Reply {
    let path = robroz_runner::reply_path(control_dir);
    let _ = std::fs::remove_file(&path);
    let mut ctx = crate::tools::ToolCtx::new(get_limit);
    let resume = crate::mcp_tools::pending_is_approved(control_dir);
    if resume {
        if let Ok(pending) = crate::mcp_tools::load_approved(control_dir) {
            ctx.used = pending.used;
            ctx.needs_setup = pending.needs_setup.clone();
        }
    }
    let mut on_using = |name: &str| on_progress(&format!("在用 {name}"));
    let (mut text, failure, over_quota, pending) = if resume {
        match crate::mcp_tools::load_approved(control_dir) {
            Ok(pending) => {
                match crate::tools::complete_resume(
                    model,
                    pending.messages,
                    &pending.remaining,
                    tools,
                    &mut ctx,
                    &mut on_using,
                    pending.round,
                ) {
                    Ok(text) => (text, None, false, None),
                    Err(crate::tools::CompleteError::NeedsApproval(pending)) => {
                        (String::new(), None, false, Some(*pending))
                    }
                    Err(err) => {
                        eprintln!("pump: model for {task_id}: {}", err.failure_text());
                        let text = err.failure_text();
                        let over = matches!(err, crate::tools::CompleteError::OverQuota(_));
                        (text.clone(), Some(text), over, None)
                    }
                }
            }
            Err(err) => {
                eprintln!("pump: resume mcp for {task_id}: {err}");
                let text = format!("无法继续已批准的调用：{err}");
                (text.clone(), Some(text), false, None)
            }
        }
    } else if tools.is_empty() {
        match model.reply(prompt, memory_section) {
            Ok(text) => (text, None, false, None),
            Err(err) => {
                eprintln!("pump: model for {task_id}: {err}");
                let text = format!("模型没回：{err}");
                (text.clone(), Some(text), false, None)
            }
        }
    } else {
        match crate::tools::complete(
            model,
            prompt,
            memory_section,
            tools,
            &mut ctx,
            &mut on_using,
        ) {
            Ok(text) => (text, None, false, None),
            Err(crate::tools::CompleteError::NeedsApproval(pending)) => {
                (String::new(), None, false, Some(*pending))
            }
            Err(err) => {
                eprintln!("pump: model for {task_id}: {}", err.failure_text());
                let text = err.failure_text();
                let over = matches!(err, crate::tools::CompleteError::OverQuota(_));
                (text.clone(), Some(text), over, None)
            }
        }
    };
    if ctx.needs_setup.as_deref() == Some("search") && !text.contains(crate::search::NOT_CONFIGURED)
    {
        if text.is_empty() {
            text = crate::search::NOT_CONFIGURED.to_string();
        } else {
            text = format!("{}\n{text}", crate::search::NOT_CONFIGURED);
        }
    }
    let progress = robroz_runner::last_progress(&text).map(str::to_string);
    if pending.is_none() {
        if let Err(err) = std::fs::write(&path, text) {
            eprintln!("pump: write reply for {task_id}: {err}; runner will echo");
        }
    }
    Reply {
        failure,
        progress,
        get_count: ctx.used,
        needs_setup: ctx.needs_setup,
        over_quota,
        pending,
    }
}

/// What [`write_reply`] hands back to pump.
struct Reply {
    failure: Option<String>,
    progress: Option<String>,
    get_count: u64,
    needs_setup: Option<String>,
    over_quota: bool,
    pending: Option<crate::tools::PendingApproval>,
}

fn block_mcp_task(
    conn: &Connection,
    task: &TaskRow,
    pending: &crate::tools::PendingApproval,
    updated_at: &str,
    get_count: i64,
) -> rusqlite::Result<()> {
    let hash = crate::mcp_tools::intent_hash(&task.id, pending);
    let line = format!("在用 {}", pending.display_name);
    let changed = conn.execute(
        "UPDATE tasks SET state = 'blocked', container_up = 0, get_count = ?1, updated_at = ?2, progress = ?4,
                          intent_kind = 'mcp', intent_line = ?5, intent_hash = ?6
         WHERE id = ?3 AND state = 'running'",
        params![
            get_count,
            updated_at,
            task.id,
            line,
            pending.display_name.as_str(),
            hash.as_str(),
        ],
    )?;
    if changed > 0 {
        if let Some(mcp) = &pending.mcp {
            let summary =
                crate::mcp_tools::approval_summary(&mcp.server_name, &mcp.tool_name, &pending.args);
            crate::approvals::gate_mcp_task(
                conn,
                &task.id,
                &summary,
                &hash,
                updated_at,
                &mcp.server_name,
                &mcp.tool_name,
            )?;
        }
    }
    Ok(())
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
fn outbound_record(
    group_dir: &Path,
    task_id: &str,
    already_used: u64,
    limit: u64,
) -> (i64, Option<OverQuota>) {
    let text = std::fs::read_to_string(robroz_runner::outbound_path(group_dir, task_id))
        .unwrap_or_default();
    let sandbox: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let host: Vec<&str> = vec!["GET"; already_used as usize];
    let methods: Vec<&str> = host.into_iter().chain(sandbox).collect();
    let gets = methods.iter().filter(|m| **m == "GET").count() as i64;
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
