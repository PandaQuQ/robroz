use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hive_policy::{outbound, side_effect, Outbound, SideEffect, SideEffectDecision};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::sandbox::{container_name, result_rel, ContainerPhase, RunRequest, Sandbox};

struct TaskRow {
    id: String,
    group_id: String,
    prompt: String,
    created_at: String,
    briefing: bool,
}

pub fn pump(
    conn: &Connection,
    sandbox: &mut dyn Sandbox,
    volumes_root: &Path,
    image: &str,
    get_limit: u64,
) -> rusqlite::Result<()> {
    let queued = load_tasks(conn, "queued")?;
    for task in queued {
        let Some(group_dir) = ensure_group_dir(volumes_root, &task.group_id) else {
            continue;
        };
        let name = container_name(&task.id);
        let started = sandbox.start(&RunRequest {
            name: &name,
            image,
            group_dir: &group_dir,
            task_id: &task.id,
            prompt: &task.prompt,
        });
        if started.is_err() {
            eprintln!("pump: start {} failed; task stays queued", task.id);
            continue;
        }
        let updated_at = newer_than(&task.created_at);
        conn.execute(
            "UPDATE tasks SET state = 'running', container_up = 1, updated_at = ?1
             WHERE id = ?2 AND state = 'queued'",
            params![updated_at, task.id],
        )?;
    }

    let running = load_tasks(conn, "running")?;
    for task in running {
        let name = container_name(&task.id);
        let phase = match sandbox.phase(&name) {
            Ok(phase) => phase,
            Err(err) => {
                eprintln!("pump: inspect {}: {}", task.id, err.message);
                continue;
            }
        };
        let ContainerPhase::Exited { code } = phase else {
            continue;
        };

        // Workspace reads of the group volume are allowed. This is not a scheduler.
        if side_effect(SideEffect::WorkspaceRead) != SideEffectDecision::Allow {
            eprintln!("pump: workspace read denied for {}", task.id);
            continue;
        }

        let group_dir = group_volume(volumes_root, &task.group_id);
        let rel = result_rel(&task.id);
        let result_file = group_dir.join(&rel);
        let intent = read_intent(&group_dir, &task.id);
        let (get_count, quota_fail) = outbound_record(&group_dir, &task.id, get_limit);

        let block = match intent.as_deref() {
            Some("external") => {
                side_effect(SideEffect::ExternalWrite) == SideEffectDecision::BlockAndDropContainer
            }
            Some("captcha") => {
                side_effect(SideEffect::Captcha) == SideEffectDecision::BlockAndDropContainer
            }
            _ => false,
        };

        if let Err(err) = sandbox.remove(&name) {
            eprintln!("pump: remove {}: {}", task.id, err.message);
            continue;
        }

        let updated_at = newer_than(&task.created_at);
        if block {
            conn.execute(
                "UPDATE tasks SET state = 'blocked', container_up = 0, get_count = ?1, updated_at = ?2
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
            continue;
        }

        let dead = quota_fail || code != 0 || !result_file.is_file();
        if dead {
            // Container is gone and there is no successful result. Stay running
            // with containerUp false so the card uses the dead-run line. Not blocked.
            conn.execute(
                "UPDATE tasks SET container_up = 0, get_count = ?1, updated_at = ?2
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
            continue;
        }

        if side_effect(SideEffect::WorkspaceWrite) != SideEffectDecision::Allow {
            conn.execute(
                "UPDATE tasks SET state = 'blocked', container_up = 0, get_count = ?1, updated_at = ?2
                 WHERE id = ?3 AND state = 'running'",
                params![get_count, updated_at, task.id],
            )?;
            continue;
        }

        let delivery_id = Uuid::new_v4().to_string();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET state = 'done', container_up = 0, get_count = ?1, updated_at = ?2
             WHERE id = ?3 AND state = 'running'",
            params![get_count, updated_at, task.id],
        )?;
        tx.execute(
            "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at)
             VALUES (?1, ?2, ?3, '已写入结果', ?4, ?5, ?6)",
            params![delivery_id, task.group_id, task.id, rel, task.briefing as i64, updated_at],
        )?;
        tx.commit()?;
    }
    Ok(())
}

fn load_tasks(conn: &Connection, state: &str) -> rusqlite::Result<Vec<TaskRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_id, prompt, created_at, briefing FROM tasks WHERE state = ?1 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([state], |row| {
        Ok(TaskRow {
            id: row.get(0)?,
            group_id: row.get(1)?,
            prompt: row.get(2)?,
            created_at: row.get(3)?,
            briefing: row.get::<_, i64>(4)? != 0,
        })
    })?;
    rows.collect()
}

fn ensure_group_dir(volumes_root: &Path, group_id: &str) -> Option<PathBuf> {
    let dir = volumes_root.join(group_id);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("pump: create {}: {err}", dir.display());
        return None;
    }
    match std::fs::canonicalize(&dir) {
        Ok(dir) => Some(dir),
        Err(err) => {
            eprintln!("pump: canonicalize {}: {err}", dir.display());
            None
        }
    }
}

fn group_volume(volumes_root: &Path, group_id: &str) -> PathBuf {
    let dir = volumes_root.join(group_id);
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

fn read_intent(group_dir: &Path, task_id: &str) -> Option<String> {
    let path = group_dir.join("tasks").join(task_id).join("intent");
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
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
