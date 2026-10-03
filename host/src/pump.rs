use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::sandbox::{container_name, result_rel, ContainerPhase, RunRequest, Sandbox};

struct TaskRow {
    id: String,
    group_id: String,
    prompt: String,
    created_at: String,
}

pub fn pump(
    conn: &Connection,
    sandbox: &mut dyn Sandbox,
    volumes_root: &Path,
    image: &str,
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
            "UPDATE tasks SET state = 'running', updated_at = ?1 WHERE id = ?2 AND state = 'queued'",
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
        let rel = result_rel(&task.id);
        let file = volumes_root.join(&task.group_id).join(&rel);
        let success = code == 0 && file.is_file();
        if let Err(err) = sandbox.remove(&name) {
            eprintln!("pump: remove {}: {}", task.id, err.message);
            continue;
        }
        if !success {
            // Non-zero exit or a missing result file. The container is gone.
            // This node has no blocked state and does not resume; leave running.
            eprintln!("pump: task {} finished without a result; left running", task.id);
            continue;
        }
        let updated_at = newer_than(&task.created_at);
        let delivery_id = Uuid::new_v4().to_string();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tasks SET state = 'done', updated_at = ?1 WHERE id = ?2 AND state = 'running'",
            params![updated_at, task.id],
        )?;
        tx.execute(
            "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at)
             VALUES (?1, ?2, ?3, '已写入结果', ?4, 0, ?5)",
            params![delivery_id, task.group_id, task.id, rel, updated_at],
        )?;
        tx.commit()?;
    }
    Ok(())
}

fn load_tasks(conn: &Connection, state: &str) -> rusqlite::Result<Vec<TaskRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_id, prompt, created_at FROM tasks WHERE state = ?1 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([state], |row| {
        Ok(TaskRow {
            id: row.get(0)?,
            group_id: row.get(1)?,
            prompt: row.get(2)?,
            created_at: row.get(3)?,
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
