//! The Router: before a visible task runs, the bro's own model may split it
//! into up to [`MAX_STEPS`] Worker steps. Steps run one after another inside
//! the same task's model phase; each sees the task prompt and the results of
//! the steps before it. The task keeps one status card and gets one merged
//! delivery. While a split task runs, its `progress` is exactly
//! [`robroz_contract::host_text::router_step_progress`].
//!
//! A split that errors, has no model, can't be parsed, or gives fewer than 2
//! steps falls back to a single Worker, exactly as before this module. That
//! Worker is not a step and shows no step progress.
//!
//! The plan and the next step index live in `router_plans`, so a step that
//! stops on an approval card resumes after approval (and after a restart),
//! then the remaining steps run. The row is removed when the task ends.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// At most this many Worker steps per task; extra steps from the model are dropped.
pub const MAX_STEPS: usize = 3;

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS router_plans (
        task_id TEXT PRIMARY KEY,
        plan TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
";

/// A task split into steps, and how far it got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    /// One instruction per step, 2 to [`MAX_STEPS`].
    pub steps: Vec<String>,
    /// Results of the finished steps, in order. Its length is the next step.
    pub results: Vec<String>,
}

impl Plan {
    pub fn next(&self) -> usize {
        self.results.len()
    }

    pub fn is_finished(&self) -> bool {
        self.results.len() >= self.steps.len()
    }

    /// Card `progress` for step `index` (0-based).
    pub fn progress(&self, index: usize) -> String {
        robroz_contract::host_text::router_step_progress(
            (index + 1).min(self.steps.len()),
            self.steps.len(),
        )
    }

    /// The one delivery: every step's result, in order, separated by a blank line.
    pub fn merged(&self) -> String {
        self.results
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Text sent only to the model: asks for the split as JSON.
pub(crate) fn split_prompt(task_prompt: &str) -> String {
    format!(
        "You split tasks into steps. Split the task below into at most {MAX_STEPS} steps \
         that run in order. If the task is simple and one step is enough, give 1 step. \
         Write each step in the language of the task.\n\
         Reply with one JSON object and nothing else: \
         {{\"steps\":[\"what step 1 does\",\"what step 2 does\"]}}\n\n\
         Task:\n{task_prompt}\n"
    )
}

/// Text sent only to the model: one Worker step.
pub(crate) fn step_prompt(task_prompt: &str, plan: &Plan, index: usize) -> String {
    let mut text = format!(
        "{task_prompt}\n\nThis task is split into {} steps. Do only step {} now: {}\n",
        plan.steps.len(),
        index + 1,
        plan.steps[index]
    );
    if index > 0 {
        text.push_str("\nResults of the earlier steps:\n");
        for (i, result) in plan.results.iter().take(index).enumerate() {
            text.push_str(&format!("Step {}:\n{}\n", i + 1, result.trim()));
        }
    }
    text.push_str(
        "\nDo only this step and give its result directly. \
         Reply in the language the user wrote in.\n",
    );
    text
}

/// The steps in the model's answer: a JSON object `{"steps": [...]}` or a bare
/// JSON array of strings, possibly wrapped in other text. `None` when it can't
/// be read or gives fewer than 2 steps. More than [`MAX_STEPS`] are cut.
pub(crate) fn parse_steps(answer: &str) -> Option<Vec<String>> {
    let value = json_in(answer, '{', '}')
        .and_then(|v| v.get("steps").cloned())
        .or_else(|| json_in(answer, '[', ']'))?;
    let steps: Vec<String> = value
        .as_array()?
        .iter()
        .map(|s| s.as_str().map(|s| s.trim().to_string()))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .filter(|s| !s.is_empty())
        .take(MAX_STEPS)
        .collect();
    (steps.len() >= 2).then_some(steps)
}

fn json_in(text: &str, open: char, close: char) -> Option<serde_json::Value> {
    let start = text.find(open)?;
    let end = text.rfind(close)?;
    if end < start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

/// Ask `model` to split `task_prompt`. Any error or unusable answer is `None`.
pub(crate) fn split(
    model: &dyn robroz_model::Model,
    task_id: &str,
    task_prompt: &str,
) -> Option<Plan> {
    match model.reply(&split_prompt(task_prompt), None) {
        Ok(answer) => parse_steps(&answer).map(|steps| Plan {
            steps,
            results: Vec::new(),
        }),
        Err(err) => {
            eprintln!("router: split for {task_id} failed, single worker: {err}");
            None
        }
    }
}

pub(crate) fn load(conn: &Connection, task_id: &str) -> rusqlite::Result<Option<Plan>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT plan FROM router_plans WHERE task_id = ?1",
            [task_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|raw| match serde_json::from_str(&raw) {
        Ok(plan) => Some(plan),
        Err(err) => {
            eprintln!("router: unreadable plan for {task_id}: {err}");
            None
        }
    }))
}

pub(crate) fn save(
    conn: &Connection,
    task_id: &str,
    plan: &Plan,
    at: &str,
) -> rusqlite::Result<()> {
    let raw = serde_json::to_string(plan).expect("plan serializes");
    conn.execute(
        "INSERT INTO router_plans (task_id, plan, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(task_id) DO UPDATE SET plan = excluded.plan, updated_at = excluded.updated_at",
        params![task_id, raw, at],
    )?;
    Ok(())
}

/// Forget the plan. Returns whether the task had one (it was split).
pub(crate) fn clear(conn: &Connection, task_id: &str) -> rusqlite::Result<bool> {
    Ok(conn.execute("DELETE FROM router_plans WHERE task_id = ?1", [task_id])? > 0)
}

/// A denied card on a split task: the blocked task ends `done` with
/// 「没写出结果」 and no delivery. Caller holds the answer transaction.
pub(crate) fn end_denied(
    conn: &Connection,
    task_id: &str,
    hash: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tasks SET state = 'done', container_up = 0, updated_at = ?1, failure = ?2, failure_data = ?5,
                          progress = NULL, intent_kind = NULL, intent_line = NULL, intent_hash = NULL
         WHERE id = ?3 AND state = 'blocked' AND intent_hash = ?4",
        params![
            now,
            robroz_contract::legacy::NO_RESULT,
            task_id,
            hash,
            crate::failure::Failure::no_result().json()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
