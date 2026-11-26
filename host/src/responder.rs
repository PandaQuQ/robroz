//! Single responder: a group message that names nobody gets one answering
//! bro, not a turn for every bro.
//!
//! `POST /api/groups/{id}/say` calls [`plan`] for a group message with no
//! leading `@name`. The bro is picked by the first rule that applies:
//!
//! 1. **Follow-up.** Exactly one bro is waiting on the person here: it has a
//!    still-open task (`queued`, `running` or `blocked`, visible) or an
//!    unanswered choice card. The message continues that bro's work. When
//!    several bros are waiting, the choice below is limited to them.
//! 2. **Only bro.** The group has one bro.
//! 3. **Model.** The router/review model gets a short prompt (bro names, a
//!    one-line profile each, the message) and must answer one bro id as
//!    strict JSON. It has [`SELECT_TIMEOUT`] and [`SELECT_MAX_TOKENS`].
//! 4. **Fallback.** On a model error, a timeout, an unusable answer, or no
//!    model: the bro that most recently delivered a reply in the group, else
//!    the first bro by join order.
//!
//! The picked bro is treated like an `@name` bro: its task is visible at
//! once, the message joins its open task if it has one, and `Said.task`
//! carries it. At most one task is made per message. `@name` messages, DMs,
//! and `ROBROZ_SINGLE_RESPONDER=0` keep the old behaviour (every bro takes a
//! turn). The ids of the group and bro and the rule are logged; message text
//! never is.
//!
//! The pick is made before the message is stored and without holding the
//! database lock, so a slow model never blocks other requests.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use robroz_model::{AssistantTurn, ChatMessage, Model};
use rusqlite::{params, Connection};

/// The environment variable that turns the feature off (`0`, `false`, `off`,
/// `no`). Anything else, or unset, leaves it on.
pub const ENV_SINGLE_RESPONDER: &str = "ROBROZ_SINGLE_RESPONDER";

/// How long the model may take to pick a bro.
pub const SELECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Cap on the model's answer: `{"bro":"<id>"}` is short.
pub const SELECT_MAX_TOKENS: u32 = 48;

/// Longest profile line given to the model, in characters.
const PROFILE_MAX: usize = 120;

/// Longest part of the message given to the model, in characters.
const MESSAGE_MAX: usize = 1000;

const SYSTEM: &str = "You route one chat message to exactly one bro. Reply with JSON only.";

/// Read [`ENV_SINGLE_RESPONDER`] from the process environment.
pub fn enabled_from_env() -> bool {
    parse_flag(std::env::var(ENV_SINGLE_RESPONDER).ok().as_deref())
}

pub(crate) fn parse_flag(value: Option<&str>) -> bool {
    !matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

/// A bro the message may go to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub id: String,
    pub name: String,
    /// One line from the bro's kept profile memory, if it has one.
    pub profile: Option<String>,
}

/// Which rule picked the bro. `as_str` is what the log shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rule {
    FollowUp,
    OnlyBro,
    Model,
    LastAnswered,
    FirstJoined,
}

impl Rule {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Rule::FollowUp => "follow_up",
            Rule::OnlyBro => "only_bro",
            Rule::Model => "model",
            Rule::LastAnswered => "last_answered",
            Rule::FirstJoined => "first_joined",
        }
    }
}

/// Why the model did not pick (rule 4 then does). For the log only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelFailure {
    NoModel,
    Error,
    Timeout,
    InvalidId,
}

impl ModelFailure {
    fn as_str(self) -> &'static str {
        match self {
            ModelFailure::NoModel => "no_model",
            ModelFailure::Error => "error",
            ModelFailure::Timeout => "timeout",
            ModelFailure::InvalidId => "invalid_id",
        }
    }
}

/// The bro picked, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    pub bot_id: String,
    pub rule: Rule,
    pub model_failure: Option<ModelFailure>,
}

impl Decision {
    fn by(bot_id: &str, rule: Rule) -> Self {
        Self {
            bot_id: bot_id.to_string(),
            rule,
            model_failure: None,
        }
    }

    /// One log line: ids and the rule, never message text.
    pub(crate) fn line(&self, group_id: &str) -> String {
        match self.model_failure {
            Some(why) => format!(
                "responder: group {group_id} bro {} by {} (model: {})",
                self.bot_id,
                self.rule.as_str(),
                why.as_str()
            ),
            None => format!(
                "responder: group {group_id} bro {} by {}",
                self.bot_id,
                self.rule.as_str()
            ),
        }
    }

    pub(crate) fn log(&self, group_id: &str) {
        eprintln!("{}", self.line(group_id));
    }
}

/// What the host knows about a group when it has to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Context {
    /// Members in join order.
    pub members: Vec<Candidate>,
    /// Members waiting on the person (rule 1), in join order.
    pub waiting: Vec<String>,
    /// Member ids by most recent delivery first.
    pub recent: Vec<String>,
}

/// Picks a bro. A trait so tests need no model.
pub(crate) trait Selector: Send + Sync {
    /// The raw answer: the id of one of `candidates`.
    fn pick(&self, candidates: &[Candidate], message: &str) -> Result<String, String>;
}

/// [`Selector`] over a chat model: strict JSON `{"bro":"<id>"}`.
pub(crate) struct ModelSelector {
    model: Arc<dyn Model>,
}

impl ModelSelector {
    pub(crate) fn new(model: Arc<dyn Model>) -> Self {
        Self { model }
    }
}

impl Selector for ModelSelector {
    fn pick(&self, candidates: &[Candidate], message: &str) -> Result<String, String> {
        let turn = self
            .model
            .chat(
                &[
                    ChatMessage::System(SYSTEM.into()),
                    ChatMessage::User(prompt(candidates, message)),
                ],
                &[],
            )
            .map_err(|err| err.0)?;
        match turn {
            AssistantTurn::Text(text) => answer_id(&text).ok_or_else(|| "not strict JSON".into()),
            AssistantTurn::ToolCalls(_) => Err("tool call".into()),
        }
    }
}

fn clip(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max).collect();
    out.push('…');
    out
}

/// The prompt: bros (id, name, one profile line each) and the message.
pub(crate) fn prompt(candidates: &[Candidate], message: &str) -> String {
    let mut text = String::from("Pick the one bro best placed to answer the message. Bros:\n");
    for c in candidates {
        text.push_str(&format!("- id: {} | name: {}", c.id, clip(&c.name, 60)));
        if let Some(profile) = &c.profile {
            text.push_str(&format!(" | about: {profile}"));
        }
        text.push('\n');
    }
    text.push_str(&format!(
        "\nMessage:\n{}\n\nReply with one JSON object and nothing else: {{\"bro\":\"<id>\"}}\n",
        clip(message, MESSAGE_MAX)
    ));
    text
}

/// The `bro` string of a reply that is exactly one JSON object (code fences
/// and surrounding whitespace aside). Anything else is `None`.
pub(crate) fn answer_id(text: &str) -> Option<String> {
    let mut body = text.trim();
    if let Some(rest) = body.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        body = rest.trim().strip_suffix("```")?.trim();
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    Some(object.get("bro")?.as_str()?.trim().to_string())
}

/// `answer` when it is one of `candidates`.
fn valid(candidates: &[Candidate], answer: &str) -> bool {
    candidates.iter().any(|c| c.id == answer)
}

/// Rule 4: the bro that most recently delivered, else the first one.
fn fallback<'a>(pool: &'a [Candidate], recent: &[String]) -> (&'a Candidate, Rule) {
    for id in recent {
        if let Some(c) = pool.iter().find(|c| &c.id == id) {
            return (c, Rule::LastAnswered);
        }
    }
    (&pool[0], Rule::FirstJoined)
}

/// Rules 1 and 2, which need no model. `Err` is what rule 3 chooses among.
fn without_model(ctx: &Context) -> Result<Decision, Vec<Candidate>> {
    let waiting: Vec<&Candidate> = ctx
        .members
        .iter()
        .filter(|c| ctx.waiting.contains(&c.id))
        .collect();
    if let [only] = waiting.as_slice() {
        return Ok(Decision::by(&only.id, Rule::FollowUp));
    }
    if let [only] = ctx.members.as_slice() {
        return Ok(Decision::by(&only.id, Rule::OnlyBro));
    }
    Err(if waiting.is_empty() {
        ctx.members.clone()
    } else {
        waiting.into_iter().cloned().collect()
    })
}

/// Ask `selector` with a time limit. The call runs on its own thread, so a
/// model that hangs cannot hold the request past `timeout`.
fn ask(
    selector: Arc<dyn Selector>,
    pool: Vec<Candidate>,
    message: String,
    timeout: Duration,
) -> Result<String, ModelFailure> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(selector.pick(&pool, &message));
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(_)) => Err(ModelFailure::Error),
        Err(_) => Err(ModelFailure::Timeout),
    }
}

/// Pick the bro for `message`. Blocks up to `timeout` on the selector.
pub(crate) fn choose(
    ctx: &Context,
    message: &str,
    selector: Option<&Arc<dyn Selector>>,
    timeout: Duration,
) -> Decision {
    let pool = match without_model(ctx) {
        Ok(decision) => return decision,
        Err(pool) => pool,
    };
    let failure = match selector {
        None => ModelFailure::NoModel,
        Some(selector) => match ask(selector.clone(), pool.clone(), message.to_string(), timeout) {
            Ok(answer) if valid(&pool, answer.trim()) => {
                return Decision::by(answer.trim(), Rule::Model)
            }
            Ok(_) => ModelFailure::InvalidId,
            Err(failure) => failure,
        },
    };
    let (bro, rule) = fallback(&pool, &ctx.recent);
    Decision {
        bot_id: bro.id.clone(),
        rule,
        model_failure: Some(failure),
    }
}

/// What the handler has to do before it stores the message.
pub(crate) enum Plan {
    /// Not a message this applies to (a DM, an `@name`, a bro-less group, or
    /// the feature is off): the old behaviour.
    Off,
    /// Picked without a model.
    Decided(Decision),
    /// Needs [`choose`], off the database lock.
    Ask(Context),
}

/// Look at the group. `mentioned` is whether a leading `@name` matched.
pub(crate) fn plan(
    conn: &Connection,
    enabled: bool,
    group_id: &str,
    mentioned: bool,
) -> rusqlite::Result<Plan> {
    if !enabled || mentioned || crate::bros::dm_bot(conn, group_id)?.is_some() {
        return Ok(Plan::Off);
    }
    let ctx = context(conn, group_id)?;
    if ctx.members.is_empty() {
        return Ok(Plan::Off);
    }
    Ok(match without_model(&ctx) {
        Ok(decision) => Plan::Decided(decision),
        Err(_) => Plan::Ask(ctx),
    })
}

fn profile_of(conn: &Connection, bot_id: &str) -> rusqlite::Result<Option<String>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT text FROM memories
             WHERE scope = ?1 AND kind = 'profile' AND status = 'kept' AND forgotten_at IS NULL
             ORDER BY updated_at DESC, rowid DESC LIMIT 1",
            [format!("bot:{bot_id}")],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(text
        .map(|t| clip(&t, PROFILE_MAX))
        .filter(|t| !t.is_empty()))
}

pub(crate) fn context(conn: &Connection, group_id: &str) -> rusqlite::Result<Context> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.name FROM memberships m JOIN bots b ON b.id = m.bot_id
         WHERE m.group_id = ?1 ORDER BY m.rowid",
    )?;
    let pairs: Vec<(String, String)> = stmt
        .query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut members = Vec::with_capacity(pairs.len());
    for (id, name) in pairs {
        let profile = profile_of(conn, &id)?;
        members.push(Candidate { id, name, profile });
    }
    let ids = |sql: &str| -> rusqlite::Result<Vec<String>> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![group_id], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    let mut waiting = ids("SELECT DISTINCT assignee_bot_id FROM tasks
         WHERE group_id = ?1 AND turn_hidden = 0 AND state IN ('queued', 'running', 'blocked')")?;
    waiting.extend(ids(
        "SELECT DISTINCT bot_id FROM choices WHERE group_id = ?1 AND answered_at IS NULL",
    )?);
    waiting.retain(|id| members.iter().any(|m| &m.id == id));
    let recent = ids(
        "SELECT t.assignee_bot_id FROM deliveries d JOIN tasks t ON t.id = d.task_id
         WHERE d.group_id = ?1 AND d.failure IS NULL
         ORDER BY d.created_at DESC, d.rowid DESC LIMIT 50",
    )?;
    Ok(Context {
        members,
        waiting,
        recent,
    })
}

/// The selector to use: a test override, else the host's fixed model, else
/// the model configured in settings or env (with the short timeout and
/// token cap). `None` without any model.
pub(crate) fn selector_for(
    conn: &Connection,
    over: Option<&Arc<dyn Selector>>,
    model: Option<&Arc<dyn Model>>,
) -> Option<Arc<dyn Selector>> {
    if let Some(selector) = over {
        return Some(selector.clone());
    }
    if let Some(model) = model {
        return Some(Arc::new(ModelSelector::new(model.clone())));
    }
    match crate::model_settings::live_model(conn) {
        Ok(Some(cfg)) => Some(Arc::new(ModelSelector::new(Arc::new(
            robroz_model::OpenAiCompat::with_timeout(cfg, SELECT_TIMEOUT)
                .with_max_tokens(SELECT_MAX_TOKENS),
        )))),
        Ok(None) => None,
        Err(err) => {
            eprintln!("responder: model config: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
