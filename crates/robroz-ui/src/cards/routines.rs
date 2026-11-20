//! Routines list view model (SPEC §7) built from robroz#15's
//! `ScheduleEntry`, with optimistic pause toggling.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use robroz_contract::ScheduleEntry;

use super::status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRow {
    pub id: String,
    /// The schedule's prompt; the contract has no separate name.
    pub name: String,
    /// Unix seconds; `None` if the host sent something unparsable.
    pub next_run_at: Option<i64>,
    pub paused: bool,
    pub group_id: String,
    pub assignee_bot_id: String,
}

impl RoutineRow {
    pub fn from_entry(entry: &ScheduleEntry) -> Self {
        let s = &entry.schedule;
        Self {
            id: s.id.clone(),
            name: s.prompt.clone(),
            next_run_at: DateTime::parse_from_rfc3339(&s.next_run_at)
                .ok()
                .map(|t| t.timestamp()),
            paused: entry.paused,
            group_id: s.group_id.clone(),
            assignee_bot_id: s.assignee_bot_id.clone(),
        }
    }

    /// Right-hand text: next run in `tz`, or `None` when paused (the view
    /// shows the 已暂停 tag instead).
    pub fn next_run_text_in<Tz: TimeZone>(&self, now: i64, tz: &Tz) -> Option<String> {
        if self.paused {
            return None;
        }
        let local = |t: i64| -> Option<NaiveDateTime> {
            tz.timestamp_opt(t, 0).single().map(|d| d.naive_local())
        };
        Some(match (self.next_run_at.and_then(local), local(now)) {
            (Some(next), Some(now)) => status::format_next_run(next, now),
            _ => "—".into(),
        })
    }

    pub fn next_run_text(&self, now: i64) -> Option<String> {
        self.next_run_text_in(now, &Local)
    }
}

pub const PAUSED_TAG: &str = "已暂停";
pub const TITLE: &str = "例行任务";
pub const EMPTY_TITLE: &str = "还没有例行任务";
pub const EMPTY_HINT: &str = "在群里对 bro 说「每天 8 点给我发简报」就会出现在这里";

/// Rows plus in-flight pause toggles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routines {
    pub rows: Vec<RoutineRow>,
    pub loaded: bool,
    /// Ids whose PATCH is in flight.
    pub pending: Vec<String>,
    /// Ids whose last PATCH failed; shows 没改成功 until cleared.
    pub failed: Vec<String>,
}

impl Routines {
    /// Replace the list from a fresh GET, keeping optimistic values for
    /// rows whose PATCH is still in flight.
    pub fn load(&mut self, mut rows: Vec<RoutineRow>) {
        for row in &mut rows {
            if self.pending.contains(&row.id) {
                if let Some(old) = self.rows.iter().find(|r| r.id == row.id) {
                    row.paused = old.paused;
                }
            }
        }
        self.rows = rows;
        self.loaded = true;
    }

    /// Flip a row now. Returns the new `paused` to PATCH, or `None` if the
    /// row is unknown or already in flight.
    pub fn toggle(&mut self, id: &str) -> Option<bool> {
        if self.pending.iter().any(|p| p == id) {
            return None;
        }
        let row = self.rows.iter_mut().find(|r| r.id == id)?;
        row.paused = !row.paused;
        self.pending.push(id.to_string());
        self.failed.retain(|f| f != id);
        Some(row.paused)
    }

    /// PATCH succeeded: take the host's row.
    pub fn confirm(&mut self, row: RoutineRow) {
        self.pending.retain(|p| p != &row.id);
        if let Some(slot) = self.rows.iter_mut().find(|r| r.id == row.id) {
            *slot = row;
        }
    }

    /// PATCH failed: snap back and mark the row.
    pub fn revert(&mut self, id: &str, wanted_paused: bool) {
        self.pending.retain(|p| p != id);
        if let Some(row) = self.rows.iter_mut().find(|r| r.id == id) {
            row.paused = !wanted_paused;
        }
        if !self.failed.iter().any(|f| f == id) {
            self.failed.push(id.to_string());
        }
    }

    pub fn clear_failed(&mut self, id: &str) {
        self.failed.retain(|f| f != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use robroz_contract::Schedule;

    fn entry(id: &str, next: &str, paused: bool) -> ScheduleEntry {
        ScheduleEntry {
            schedule: Schedule {
                id: id.into(),
                group_id: "g1".into(),
                assignee_bot_id: "bot-dee".into(),
                cron: "0 8 * * *".into(),
                prompt: "每日简报".into(),
                next_run_at: next.into(),
                last_task_id: None,
            },
            paused,
        }
    }

    #[test]
    fn row_from_contract_entry() {
        let row = RoutineRow::from_entry(&entry("s1", "2026-11-21T08:00:00.000Z", false));
        assert_eq!(row.name, "每日简报");
        assert_eq!(row.group_id, "g1");
        let now = DateTime::parse_from_rfc3339("2026-11-20T01:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            row.next_run_text_in(now, &Utc).as_deref(),
            Some("明天 08:00")
        );
        let paused = RoutineRow::from_entry(&entry("s2", "2026-11-21T08:00:00.000Z", true));
        assert_eq!(paused.next_run_text_in(now, &Utc), None);
        let bad = RoutineRow::from_entry(&entry("s3", "garbage", false));
        assert_eq!(bad.next_run_text_in(now, &Utc).as_deref(), Some("—"));
    }

    #[test]
    fn optimistic_toggle_confirm_and_revert() {
        let mut r = Routines::default();
        r.load(vec![
            RoutineRow::from_entry(&entry("s1", "2026-11-21T08:00:00Z", false)),
            RoutineRow::from_entry(&entry("s2", "2026-11-21T08:00:00Z", false)),
        ]);
        assert_eq!(r.toggle("s1"), Some(true));
        assert!(r.rows[0].paused, "flips immediately");
        assert_eq!(r.toggle("s1"), None, "one PATCH at a time per row");
        // A refresh while in flight keeps the optimistic value.
        r.load(vec![
            RoutineRow::from_entry(&entry("s1", "2026-11-21T08:00:00Z", false)),
            RoutineRow::from_entry(&entry("s2", "2026-11-21T08:00:00Z", false)),
        ]);
        assert!(r.rows[0].paused);
        r.revert("s1", true);
        assert!(!r.rows[0].paused, "snaps back");
        assert_eq!(r.failed, ["s1"]);
        assert_eq!(r.toggle("s1"), Some(true));
        assert!(r.failed.is_empty());
        let mut host = r.rows[0].clone();
        host.paused = true;
        r.confirm(host);
        assert!(r.pending.is_empty());
        assert!(r.rows[0].paused);
        assert_eq!(r.toggle("nope"), None);
    }
}
