//! Host-side schedules. Cron is evaluated here, in UTC. Containers never see cron.

use chrono::{DateTime, SecondsFormat, Utc};
use croner::parser::{CronParser, Seconds, Year};
use croner::Cron;
use robroz_policy::should_start_container;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

/// Accepts 5 fields (`min hour dom month dow`) or 6 fields
/// (`sec min hour dom month dow`). No year field and no `@nicknames`.
pub fn parse_cron(pattern: &str) -> Option<Cron> {
    let fields = pattern.split_whitespace().count();
    if !(5..=6).contains(&fields) || pattern.contains('@') {
        return None;
    }
    let parser = CronParser::builder()
        .seconds(Seconds::Optional)
        .year(Year::Disallowed)
        .build();
    let cron = parser.parse(pattern).ok()?;
    // Reject patterns that never fire, such as 30 February.
    cron.find_next_occurrence(&Utc::now(), false).ok()?;
    Some(cron)
}

/// Next occurrence strictly after `now`, in UTC.
pub fn next_after(cron: &Cron, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    cron.find_next_occurrence(&now, false).ok()
}

pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

struct DueSchedule {
    id: String,
    group_id: String,
    assignee_bot_id: String,
    cron: String,
    prompt: String,
    next_run_at: String,
    last_task_id: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TickReport {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
}

/// One scheduler pass at an explicit `now`. Creates queued briefing tasks only;
/// containers are started later by `pump`, never here.
pub fn tick(conn: &Connection, now: DateTime<Utc>) -> rusqlite::Result<TickReport> {
    let mut report = TickReport::default();
    let schedules = {
        let mut stmt = conn.prepare(
            "SELECT id, group_id, assignee_bot_id, cron, prompt, next_run_at, last_task_id
             FROM schedules WHERE paused_at IS NULL ORDER BY next_run_at, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DueSchedule {
                id: row.get(0)?,
                group_id: row.get(1)?,
                assignee_bot_id: row.get(2)?,
                cron: row.get(3)?,
                prompt: row.get(4)?,
                next_run_at: row.get(5)?,
                last_task_id: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for schedule in schedules {
        let Ok(next_run_at) = DateTime::parse_from_rfc3339(&schedule.next_run_at) else {
            eprintln!("tick: schedule {} has a bad nextRunAt", schedule.id);
            continue;
        };
        let due = next_run_at.with_timezone(&Utc) <= now;
        if !due {
            continue;
        }
        let Some(cron) = parse_cron(&schedule.cron) else {
            eprintln!("tick: schedule {} has an invalid cron", schedule.id);
            continue;
        };
        let Some(following) = next_after(&cron, now) else {
            eprintln!("tick: schedule {} has no next occurrence", schedule.id);
            continue;
        };

        let already_running = match &schedule.last_task_id {
            Some(task_id) => last_task_is_live(conn, task_id)?,
            None => false,
        };

        let tx = conn.unchecked_transaction()?;
        if should_start_container(due, already_running) {
            let task_id = Uuid::new_v4().to_string();
            let created_at = stamp(now);
            tx.execute(
                "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at,
                                    container_up, get_count, briefing, schedule_id)
                 VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?5, 0, 0, 1, ?6)",
                params![
                    task_id,
                    schedule.group_id,
                    schedule.assignee_bot_id,
                    schedule.prompt,
                    created_at,
                    schedule.id
                ],
            )?;
            tx.execute(
                "UPDATE schedules SET last_task_id = ?1, next_run_at = ?2 WHERE id = ?3",
                params![task_id, stamp(following), schedule.id],
            )?;
            report.created.push(task_id);
        } else {
            // Previous run still queued or live: skip this occurrence, no backfill.
            tx.execute(
                "UPDATE schedules SET next_run_at = ?1 WHERE id = ?2",
                params![stamp(following), schedule.id],
            )?;
            report.skipped.push(schedule.id.clone());
        }
        tx.commit()?;
    }
    Ok(report)
}

/// Queued, or running with a live container.
fn last_task_is_live(conn: &Connection, task_id: &str) -> rusqlite::Result<bool> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT state, container_up FROM tasks WHERE id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((state, _)) if state == "queued" => true,
        Some((state, up)) if state == "running" => up != 0,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn accepts_five_and_six_fields_only() {
        assert!(parse_cron("* * * * *").is_some());
        assert!(parse_cron("0 9 * * 1-5").is_some());
        assert!(parse_cron("*/30 * * * * *").is_some());
        assert!(parse_cron("").is_none());
        assert!(parse_cron("not a cron").is_none());
        assert!(parse_cron("61 * * * *").is_none());
        assert!(parse_cron("* * * *").is_none());
        assert!(parse_cron("0 0 0 * * * 2026").is_none());
        assert!(parse_cron("@daily").is_none());
        assert!(parse_cron("0 0 30 2 *").is_none());
    }

    #[test]
    fn next_is_strictly_after_now_in_utc() {
        let cron = parse_cron("0 9 * * *").unwrap();
        let at_nine = Utc.with_ymd_and_hms(2026, 10, 8, 9, 0, 0).unwrap();
        assert_eq!(
            next_after(&cron, at_nine).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 9, 9, 0, 0).unwrap()
        );
        let before = Utc.with_ymd_and_hms(2026, 10, 8, 8, 59, 59).unwrap();
        assert_eq!(next_after(&cron, before).unwrap(), at_nine);
    }
}
