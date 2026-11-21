use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "done")]
    Done,
}

/// Why a blocked task is waiting: an unanswered approval card, or a person
/// taking over (captcha and the like). Only present when `state` is `blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaitingFor {
    #[serde(rename = "approval")]
    Approval,
    #[serde(rename = "takeover")]
    Takeover,
}

impl TaskState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "排队",
            Self::Running => "进行",
            Self::Blocked => "卡住",
            Self::Done => "完成",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageKind {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "image")]
    Image,
    #[serde(rename = "file")]
    File,
}

impl MessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroup {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMember {
    pub bot_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMessage {
    pub kind: MessageKind,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTask {
    pub assignee_bot_id: String,
    pub prompt: String,
}

/// `POST /api/groups/{id}/say` body: what the person typed. The host stores
/// it as a text message and, in a DM or after `@名字`, queues a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Say {
    pub body: String,
}

/// `POST /api/groups/{id}/say` reply. `task` is `null` when nobody was asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Said {
    pub message: Message,
    pub task: Option<Task>,
}

/// `@名字 rest` → `(member id, rest)`. `members` is `(id, name)`. The longest
/// matching name wins (`@bro 2` beats `@bro`), the name must end at a space or
/// the end, full-width `＠` works, and an empty `rest` means no task.
pub fn route_mention<'a>(body: &str, members: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<(String, String)> {
    let body = body.trim();
    let after = body.strip_prefix('@').or_else(|| body.strip_prefix('＠'))?;
    let mut best: Option<(&str, &str, &str)> = None;
    for (id, name) in members {
        let Some(rest) = after.strip_prefix(name) else { continue };
        let boundary = rest.is_empty() || rest.starts_with(char::is_whitespace);
        if !name.is_empty() && boundary && best.is_none_or(|(_, prev, _)| name.len() > prev.len()) {
            best = Some((id, name, rest));
        }
    }
    let (id, _, rest) = best?;
    let prompt = rest.trim();
    (!prompt.is_empty()).then(|| (id.to_string(), prompt.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub tenant_id: String,
}

/// `GET /api/groups` body, oldest group first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Groups {
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub group_id: String,
    pub bot_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub kind: MessageKind,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub assignee_bot_id: String,
    pub prompt: String,
    pub state: TaskState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSchedule {
    pub group_id: String,
    pub assignee_bot_id: String,
    pub cron: String,
    pub prompt: String,
}

/// `lastTaskId` is always present and is `null` until the first run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub group_id: String,
    pub assignee_bot_id: String,
    pub cron: String,
    pub prompt: String,
    pub next_run_at: String,
    pub last_task_id: Option<String>,
}

/// A schedule as listed for the routines view: the frozen [`Schedule`] fields
/// plus `paused`. `POST /api/schedules` still returns a plain [`Schedule`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEntry {
    #[serde(flatten)]
    pub schedule: Schedule,
    pub paused: bool,
}

/// `GET /api/groups/{id}/schedules` body, oldest schedule first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedules {
    pub schedules: Vec<ScheduleEntry>,
}

/// `PATCH /api/schedules/{id}` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSchedule {
    pub paused: bool,
}

/// One option on a choice card. `id` is what comes back in `choice`.
/// `primary` marks the bot's recommendation; at most one, ignored when `multi`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub primary: bool,
}

/// `POST /api/groups/{id}/choices` body: a bot in the group asks the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChoice {
    pub bot_id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub multi: bool,
}

/// `POST /api/choices/{id}/answer` body: option `id`s, always an array.
/// Single choice sends exactly one; multi sends one or more, no repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerChoice {
    pub choice: Vec<String>,
    /// Who answered, as the client names itself (device or person). Optional.
    #[serde(default, rename = "answeredBy", skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
}

/// Optional body of `POST /api/approvals/{id}/approve` and `/deny`. No body
/// (or `{}`) is the old request and still works.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
}

pub const ANSWERED_BY_MAX: usize = 40;

/// `answeredBy` as stored: trimmed, empty is `None`, one line, at most
/// [`ANSWERED_BY_MAX`] characters.
pub fn clean_answered_by(raw: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    if text.contains(['\n', '\r']) || text.chars().count() > ANSWERED_BY_MAX {
        return Err("answeredBy 只能一行，最多 40 个字");
    }
    Ok(Some(text.to_string()))
}

/// What an approval card asks to do. Shown as the card's verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalAction {
    Delete,
    Send,
    Spend,
}

/// The person's answer on an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

/// `POST /api/groups/{id}/approvals` body: a bot in the group asks before it
/// acts. `path` is display only. Answered with `POST /api/approvals/{id}/approve`
/// or `/deny`, once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApproval {
    pub bot_id: String,
    pub action: ApprovalAction,
    pub summary: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// Approval summaries are one line on the card.
pub const APPROVAL_SUMMARY_MAX: usize = 200;

/// Choice cards hold 2 to 4 options with distinct, non-empty values.
pub const CHOICE_OPTIONS_MIN: usize = 2;
pub const CHOICE_OPTIONS_MAX: usize = 4;

/// A bro is an agent (`bots` row). In the UI every agent is called a bro;
/// field names elsewhere (`botId`, `botName`) stay as they are.
/// `dmGroupId` is its private chat: a group whose only member is this bro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bro {
    pub id: String,
    pub name: String,
    pub dm_group_id: Option<String>,
}

/// `GET /api/bros`, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bros {
    pub bros: Vec<Bro>,
}

/// `POST /api/bros` body. No name means the next default: `bro`, `bro 2`, `bro 3`…
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateBro {
    #[serde(default)]
    pub name: Option<String>,
}

/// `PATCH /api/bros/{id}` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameBro {
    pub name: String,
}

pub const BRO_DEFAULT_NAME: &str = "bro";
pub const BRO_NAME_MAX: usize = 64;

/// The default name for a new bro given the names already taken: `bro` if
/// free, else the smallest free `bro N` with N ≥ 2.
pub fn next_bro_name<'a>(taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: std::collections::HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(BRO_DEFAULT_NAME) {
        return BRO_DEFAULT_NAME.to_string();
    }
    (2..)
        .map(|n| format!("{BRO_DEFAULT_NAME} {n}"))
        .find(|name| !taken.contains(name.as_str()))
        .expect("unbounded range")
}

/// Unread state of one group for the conversation list.
/// `unread` counts what a bro sent since `readAt`: deliveries, choice and approval cards,
/// and tasks that got stuck and need you. Your own messages and routine status
/// moves (queued, running, done) do not count; the delivery does.
/// `lastActivityAt` is the newest change of any card, for sorting; `null` when empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unread {
    pub group_id: String,
    pub read_at: Option<String>,
    pub unread: u64,
    pub last_activity_at: Option<String>,
}

/// `GET /api/unread`: every group, most recently active first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unreads {
    pub groups: Vec<Unread>,
}

/// `POST /api/groups/{id}/read` body. `readAt` defaults to now; the read
/// position only moves forward, an older value is ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkRead {
    #[serde(default)]
    pub read_at: Option<String>,
}

/// What a memory is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// Who the person is.
    Profile,
    /// How the person likes work done.
    Habit,
    /// Notes on a project.
    Project,
}

impl MemoryKind {
    pub const ALL: [MemoryKind; 3] = [MemoryKind::Profile, MemoryKind::Habit, MemoryKind::Project];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Profile => "profile",
            MemoryKind::Habit => "habit",
            MemoryKind::Project => "project",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == value)
    }
}

/// Who wrote a memory down: the person, or a bro (a proposal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemorySource {
    User,
    Bot,
}

impl MemorySource {
    pub const ALL: [MemorySource; 2] = [MemorySource::User, MemorySource::Bot];

    pub fn as_str(self) -> &'static str {
        match self {
            MemorySource::User => "user",
            MemorySource::Bot => "bot",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

/// `pending`: a bro proposed it, waiting for the person. `kept`: in use.
/// `rejected`: the person said no; kept as a record, never used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    Pending,
    Kept,
    Rejected,
}

impl MemoryStatus {
    pub const ALL: [MemoryStatus; 3] = [MemoryStatus::Pending, MemoryStatus::Kept, MemoryStatus::Rejected];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Pending => "pending",
            MemoryStatus::Kept => "kept",
            MemoryStatus::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

/// Who a memory is for. On the wire it is a string: `user` (every bro) or
/// `bot:<botId>` (only that bro). Parse with [`MemoryScope::parse`]; later
/// scopes (e.g. `group:<id>`) are added here only.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MemoryScope {
    User,
    Bot(String),
}

impl MemoryScope {
    pub const BOT_PREFIX: &'static str = "bot:";

    /// Trims surrounding whitespace. `bot:` needs a non-empty id without whitespace.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw == "user" {
            return Some(MemoryScope::User);
        }
        let id = raw.strip_prefix(Self::BOT_PREFIX)?;
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return None;
        }
        Some(MemoryScope::Bot(id.to_string()))
    }

    pub fn for_bot(bot_id: &str) -> Self {
        MemoryScope::Bot(bot_id.to_string())
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryScope::User => f.write_str("user"),
            MemoryScope::Bot(id) => write!(f, "{}{id}", Self::BOT_PREFIX),
        }
    }
}

/// One memory. `scope` is the wire string of [`MemoryScope`]. Forgotten
/// memories are never returned, so there is no `forgottenAt` here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub scope: String,
    pub kind: MemoryKind,
    pub text: String,
    pub source: MemorySource,
    pub status: MemoryStatus,
    pub created_at: String,
    pub updated_at: String,
    /// Added with confirm cards: the id of the unanswered `confirm` card for a
    /// `pending` memory. Answer it with `POST /api/approvals/{confirmId}/approve`
    /// (keep) or `/deny` (reject); that is the only way a status changes.
    /// Absent otherwise, so old JSON is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_id: Option<String>,
}

/// `GET /api/memories`, newest `updatedAt` first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memories {
    pub memories: Vec<Memory>,
}

/// `POST /api/memories` body: the person writes a memory down. It is `kept` at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMemory {
    pub scope: String,
    pub kind: MemoryKind,
    pub text: String,
}

/// `PATCH /api/memories/{id}` body. Only the fields given change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMemory {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub kind: Option<MemoryKind>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Longest memory text, in characters, after trimming.
pub const MEMORY_TEXT_MAX: usize = 2000;

/// Longest skill instruction, in characters, after trimming.
pub const SKILL_INSTRUCTION_MAX: usize = 8000;
/// Longest skill title, in characters, after trimming.
pub const SKILL_TITLE_MAX: usize = 80;
/// Longest skill slug, in characters.
pub const SKILL_SLUG_MAX: usize = 64;

/// A kebab-case slug: lowercase letters, digits and single hyphens, not
/// starting or ending with a hyphen. `None` if it does not fit.
pub fn parse_skill_slug(raw: &str) -> Option<&str> {
    let slug = raw.trim();
    if slug.is_empty() || slug.chars().count() > SKILL_SLUG_MAX {
        return None;
    }
    let mut need_alnum = true;
    for c in slug.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            need_alnum = false;
        } else if c == '-' && !need_alnum {
            need_alnum = true;
        } else {
            return None;
        }
    }
    if need_alnum {
        return None;
    }
    Some(slug)
}

/// One skill. `scope` is the wire string of [`MemoryScope`]. Forgotten
/// skills are never returned, so there is no `forgottenAt` here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub scope: String,
    pub slug: String,
    pub title: String,
    pub instruction: String,
    pub source: MemorySource,
    pub status: MemoryStatus,
    pub created_at: String,
    pub updated_at: String,
    /// The unanswered `confirm` card for a `pending` skill, if any. Answer it
    /// with `POST /api/approvals/{confirmId}/approve` (keep) or `/deny`.
    /// Absent otherwise, so old JSON is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_id: Option<String>,
}

/// `GET /api/skills` or `GET /api/groups/{id}/skills`, newest `updatedAt` first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skills {
    pub skills: Vec<Skill>,
}

/// `POST /api/skills` body: the person writes a skill down. It is `kept` at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSkill {
    pub scope: String,
    pub slug: String,
    pub title: String,
    pub instruction: String,
}

/// `PATCH /api/skills/{slug}` body. Only the fields given change. `slug` and
/// `status` are not here: the slug is the identity, and status only changes
/// through the confirm card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkill {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// What a `confirm` card asks the person to keep: a memory or a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfirmSubject {
    Memory,
    Skill,
}

impl ConfirmSubject {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfirmSubject::Memory => "memory",
            ConfirmSubject::Skill => "skill",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [ConfirmSubject::Memory, ConfirmSubject::Skill].into_iter().find(|s| s.as_str() == value)
    }
}

/// A bro proposes a memory: one `robroz-memory:{json}` line. `scope` defaults
/// to the bro that ran the task. Text limits are checked by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub kind: MemoryKind,
    pub text: String,
}

/// A bro proposes a skill: one `robroz-skill:{json}` line. `scope` defaults
/// to the bro that ran the task. Length and slug rules are checked by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub slug: String,
    pub title: String,
    pub instruction: String,
}

/// One line of `tasks/{taskId}/proposals.jsonl`, tagged on `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Proposal {
    Memory(MemoryProposal),
    Skill(SkillProposal),
}

/// At most this many proposals are taken from one task.
pub const PROPOSALS_PER_TASK_MAX: usize = 20;

impl Proposal {
    /// One `proposals.jsonl` line. `None` for bad JSON, an unknown `type`, an
    /// unknown `kind` or a missing field: such lines are skipped, never an error.
    pub fn parse_line(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        serde_json::from_str(line).ok()
    }

    /// The `proposals.jsonl` line for a `robroz-memory:` payload: the JSON
    /// object with `"type":"memory"` set. `None` when the payload is not a JSON
    /// object. Fields are not checked here; the host checks when it reads.
    pub fn memory_line(payload: &str) -> Option<String> {
        let serde_json::Value::Object(mut fields) = serde_json::from_str(payload.trim()).ok()? else {
            return None;
        };
        fields.insert("type".into(), serde_json::Value::String("memory".into()));
        Some(serde_json::Value::Object(fields).to_string())
    }

    /// The `proposals.jsonl` line for a `robroz-skill:` payload, with
    /// `"type":"skill"` set. `None` when the payload is not a JSON object.
    pub fn skill_line(payload: &str) -> Option<String> {
        let serde_json::Value::Object(mut fields) = serde_json::from_str(payload.trim()).ok()? else {
            return None;
        };
        fields.insert("type".into(), serde_json::Value::String("skill".into()));
        Some(serde_json::Value::Object(fields).to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

/// Internally tagged on `type` so message, status, and delivery round-trip
/// with the JSON already returned by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum Event {
    #[serde(rename = "message")]
    Message {
        id: String,
        kind: MessageKind,
        body: String,
        created_at: String,
    },
    #[serde(rename = "status")]
    Status {
        id: String,
        task_id: String,
        bot_id: String,
        bot_name: String,
        state: TaskState,
        label: String,
        created_at: String,
        updated_at: String,
        container_up: bool,
        /// Added after the freeze: a `done` task that did not get done (model
        /// failed, approval denied). Absent otherwise, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<String>,
        /// Added after the freeze: why a `blocked` task is waiting.
        /// `approval` if an unanswered gated card still matches this intent,
        /// otherwise `takeover`. Absent unless blocked, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        waiting_for: Option<WaitingFor>,
        /// Added after the freeze: what the bro is doing right now, updated in
        /// place on this card. Only while `running` with a live container;
        /// absent otherwise, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<String>,
    },
    #[serde(rename = "delivery")]
    Delivery {
        id: String,
        summary: String,
        path: String,
        briefing: bool,
        created_at: String,
        /// Added after the freeze: why the task did not get done (e.g. the
        /// model call failed). Absent on a normal result, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<String>,
    },
    /// Added after the node-1 freeze; the three types above are unchanged.
    /// `choice` and `answeredAt` are always present and `null` until answered.
    #[serde(rename = "choice")]
    Choice {
        id: String,
        bot_id: String,
        bot_name: String,
        prompt: String,
        options: Vec<ChoiceOption>,
        multi: bool,
        choice: Option<Vec<String>>,
        created_at: String,
        answered_at: Option<String>,
        /// Added later: the `answeredBy` sent with the answer, else `null`.
        #[serde(default)]
        answered_by: Option<String>,
    },
    /// Added after `choice`. `path`, `decision` and `answeredAt` are always
    /// present; the last two are `null` until answered.
    #[serde(rename = "approval")]
    Approval {
        id: String,
        bot_id: String,
        bot_name: String,
        action: ApprovalAction,
        summary: String,
        path: Option<String>,
        decision: Option<ApprovalDecision>,
        created_at: String,
        answered_at: Option<String>,
        /// Added later: the `answeredBy` sent with the answer, else `null`.
        #[serde(default)]
        answered_by: Option<String>,
        /// Added with confirm cards: the task that raised this card (a blocked
        /// task asking to write outside). Absent for cards a bot posted itself,
        /// so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
    },
    /// Added with memories: a bro proposed something to remember; the person
    /// keeps it (`POST /api/approvals/{id}/approve`) or not (`/deny`), once.
    /// Everything the card shows is here, so no second request is needed.
    /// `decision`, `answeredAt` and `answeredBy` behave as on `approval`.
    /// Memory cards carry `memoryId`, `kind`, `text`, `scope`; skill cards
    /// carry `slug`, `title`, `instruction` and `scope`. Absent fields are omitted.
    #[serde(rename = "confirm")]
    Confirm {
        id: String,
        bot_id: String,
        bot_name: String,
        subject: ConfirmSubject,
        /// The task whose run proposed it.
        task_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<MemoryKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instruction: Option<String>,
        decision: Option<ApprovalDecision>,
        created_at: String,
        answered_at: Option<String>,
        #[serde(default)]
        answered_by: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Events {
    pub events: Vec<Event>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_scope_and_enums_round_trip() {
        use super::{CreateMemory, MemoryKind, MemoryScope, MemoryStatus};
        assert_eq!(MemoryScope::parse(" user "), Some(MemoryScope::User));
        assert_eq!(MemoryScope::parse("bot:b1"), Some(MemoryScope::Bot("b1".into())));
        for bad in ["", "users", "bot:", "bot: x", "group:g1", "User", "bot"] {
            assert_eq!(MemoryScope::parse(bad), None, "{bad:?}");
        }
        assert_eq!(MemoryScope::Bot("b1".into()).to_string(), "bot:b1");
        assert_eq!(MemoryScope::User.to_string(), "user");
        for k in MemoryKind::ALL {
            assert_eq!(MemoryKind::parse(k.as_str()), Some(k));
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(k.as_str()));
        }
        for s in MemoryStatus::ALL {
            assert_eq!(MemoryStatus::parse(s.as_str()), Some(s));
        }
        let body: CreateMemory =
            serde_json::from_value(serde_json::json!({ "scope": "user", "kind": "habit", "text": "x" })).unwrap();
        assert_eq!(body.kind, MemoryKind::Habit);
        assert!(serde_json::from_value::<CreateMemory>(serde_json::json!({ "scope": "user", "kind": "mood", "text": "x" })).is_err());
    }

    #[test]
    fn mention_routing() {
        use super::route_mention;
        let m = [("b1", "bro"), ("b2", "bro 2"), ("b3", "Ada")];
        assert_eq!(route_mention("@bro 2 写 README", m), Some(("b2".into(), "写 README".into())));
        assert_eq!(route_mention("  ＠bro 看看日志 ", m), Some(("b1".into(), "看看日志".into())));
        assert_eq!(route_mention("@Ada\t整理", m), Some(("b3".into(), "整理".into())));
        for none in ["大家好", "@bro", "@bro   ", "@Bob 你好", "@brother 你好", "问 @Ada 一下", "@"] {
            assert_eq!(route_mention(none, m), None, "{none:?}");
        }
        assert_eq!(route_mention("@ 你好", [("x", "")]), None, "empty name never matches");
    }

    #[test]
    fn bro_default_names() {
        use super::next_bro_name;
        assert_eq!(next_bro_name([]), "bro");
        assert_eq!(next_bro_name(["bro"]), "bro 2");
        assert_eq!(next_bro_name(["bro", "bro 2"]), "bro 3");
        assert_eq!(next_bro_name(["bro", "bro 3"]), "bro 2", "fills the first gap");
        assert_eq!(next_bro_name(["bro 2", "小明"]), "bro", "bro itself was renamed away");
        let json = serde_json::to_value(super::Bro { id: "b".into(), name: "bro".into(), dm_group_id: Some("g".into()) }).unwrap();
        assert_eq!(json, serde_json::json!({ "id": "b", "name": "bro", "dmGroupId": "g" }));
    }

    use super::*;

    fn assert_round_trip(event: &Event, expected: serde_json::Value) {
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value, expected);
        let back: Event = serde_json::from_value(value).unwrap();
        assert_eq!(&back, event);
    }

    #[test]
    fn approval_task_id_is_optional() {
        let mut json = serde_json::json!({
            "type": "approval", "id": "a1", "botId": "b", "botName": "B", "action": "send",
            "summary": "发邮件", "path": null, "decision": null,
            "createdAt": "2026-11-20T08:30:00.000Z", "answeredAt": null, "answeredBy": null
        });
        let old: Event = serde_json::from_value(json.clone()).unwrap();
        assert!(matches!(old, Event::Approval { task_id: None, .. }), "old JSON parses");
        assert_eq!(serde_json::to_value(&old).unwrap(), json, "absent stays absent");
        json["taskId"] = "t1".into();
        assert_round_trip(&serde_json::from_value(json.clone()).unwrap(), json);
    }

    #[test]
    fn confirm_event_json() {
        let card = Event::Confirm {
            id: "c1".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            subject: ConfirmSubject::Memory,
            task_id: Some("t1".into()),
            memory_id: Some("m1".into()),
            kind: Some(MemoryKind::Habit),
            text: Some("周报用中文".into()),
            scope: Some("bot:bot-ada".into()),
            slug: None,
            title: None,
            instruction: None,
            decision: None,
            created_at: "2026-11-20T08:30:00.000Z".into(),
            answered_at: None,
            answered_by: None,
        };
        assert_round_trip(
            &card,
            serde_json::json!({
                "type": "confirm", "id": "c1", "botId": "bot-ada", "botName": "Ada",
                "subject": "memory", "taskId": "t1", "memoryId": "m1", "kind": "habit",
                "text": "周报用中文", "scope": "bot:bot-ada", "decision": null,
                "createdAt": "2026-11-20T08:30:00.000Z", "answeredAt": null, "answeredBy": null
            }),
        );
        let skill: Event = serde_json::from_value(serde_json::json!({
            "type": "confirm", "id": "c2", "botId": "b", "botName": "B", "subject": "skill",
            "taskId": null, "slug": "weekly", "title": "写周报", "instruction": "先列三点",
            "decision": "approved", "createdAt": "2026-11-20T08:30:00.000Z",
            "answeredAt": "2026-11-20T08:31:00.000Z"
        }))
        .unwrap();
        assert!(matches!(skill, Event::Confirm { subject: ConfirmSubject::Skill, answered_by: None, .. }));
    }

    #[test]
    fn proposal_lines_parse_or_are_skipped() {
        let line = Proposal::memory_line(r#"{"kind":"habit","text":"周报用中文","type":"skill"}"#).unwrap();
        let Some(Proposal::Memory(m)) = Proposal::parse_line(&line) else { panic!("{line}") };
        assert_eq!((m.kind, m.text.as_str(), m.scope), (MemoryKind::Habit, "周报用中文", None));
        assert_eq!(Proposal::memory_line("不是 json"), None);
        assert_eq!(Proposal::memory_line("[1]"), None);
        for bad in [
            "",
            "{",
            r#"{"type":"skill","slug":"x"}"#,
            r#"{"type":"memory","kind":"mood","text":"x"}"#,
            r#"{"type":"memory","text":"x"}"#,
            r#"{"kind":"habit","text":"x"}"#,
        ] {
            assert_eq!(Proposal::parse_line(bad), None, "{bad}");
        }
        let scoped = Proposal::parse_line(r#"{"type":"memory","scope":"user","kind":"profile","text":"叫 Panda"}"#);
        assert!(matches!(scoped, Some(Proposal::Memory(MemoryProposal { scope: Some(_), .. }))));
        let skill = Proposal::skill_line(r#"{"slug":"weekly-report","title":"写周报","instruction":"先列三点","type":"memory"}"#).unwrap();
        let Some(Proposal::Skill(s)) = Proposal::parse_line(&skill) else { panic!("{skill}") };
        assert_eq!((s.slug.as_str(), s.title.as_str(), s.instruction.as_str(), s.scope.as_deref()), ("weekly-report", "写周报", "先列三点", None));
        assert_eq!(Proposal::skill_line("不是 json"), None);
        assert_eq!(Proposal::skill_line("[1]"), None);
        let scoped_skill = Proposal::parse_line(r#"{"type":"skill","slug":"weekly-report","title":"写周报","instruction":"先列三点","scope":"user"}"#);
        assert!(matches!(scoped_skill, Some(Proposal::Skill(SkillProposal { scope: Some(_), .. }))));
    }

    #[test]
    fn skill_slug_and_confirm_id_round_trip() {
        use super::{parse_skill_slug, CreateSkill, Skill, MemorySource, MemoryStatus, SKILL_SLUG_MAX};
        assert_eq!(parse_skill_slug(" weekly-report "), Some("weekly-report"));
        assert_eq!(parse_skill_slug("a"), Some("a"));
        assert_eq!(parse_skill_slug(&"a".repeat(SKILL_SLUG_MAX)), Some(&*"a".repeat(SKILL_SLUG_MAX)));
        for bad in ["", "Weekly", "weekly_report", "-weekly", "weekly-", "weekly--report", " weekly report ", &"a".repeat(SKILL_SLUG_MAX + 1)] {
            assert_eq!(parse_skill_slug(bad), None, "{bad:?}");
        }
        let body: CreateSkill = serde_json::from_value(serde_json::json!({
            "scope": "user", "slug": "weekly-report", "title": "写周报", "instruction": "先列三点"
        })).unwrap();
        assert_eq!(body.slug, "weekly-report");
        let json = serde_json::json!({
            "id": "s1", "scope": "user", "slug": "weekly-report", "title": "写周报",
            "instruction": "先列三点", "source": "user", "status": "kept",
            "createdAt": "2026-11-20T08:30:00.000Z", "updatedAt": "2026-11-20T08:30:00.000Z"
        });
        let old: Skill = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.confirm_id, None);
        assert_eq!(old.source, MemorySource::User);
        assert_eq!(old.status, MemoryStatus::Kept);
        assert_eq!(serde_json::to_value(&old).unwrap(), json, "absent stays absent");
    }

    #[test]
    fn memory_confirm_id_is_optional() {
        let json = serde_json::json!({
            "id": "m1", "scope": "user", "kind": "habit", "text": "x", "source": "bot",
            "status": "pending", "createdAt": "2026-11-20T08:30:00.000Z", "updatedAt": "2026-11-20T08:30:00.000Z"
        });
        let old: Memory = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.confirm_id, None);
        assert_eq!(serde_json::to_value(&old).unwrap(), json, "absent stays absent");
    }

    #[test]
    fn approval_event_json() {
        assert_round_trip(
            &Event::Approval {
                id: "a1".into(),
                bot_id: "bot-ops".into(),
                bot_name: "运维".into(),
                action: ApprovalAction::Delete,
                summary: "删掉旧的构建缓存".into(),
                path: Some("target/".into()),
                decision: None,
                created_at: "2026-11-20T08:30:00.000Z".into(),
                answered_at: None,
                answered_by: None,
                task_id: None,
            },
            serde_json::json!({
                "type": "approval",
                "id": "a1",
                "botId": "bot-ops",
                "botName": "运维",
                "action": "delete",
                "summary": "删掉旧的构建缓存",
                "path": "target/",
                "decision": null,
                "createdAt": "2026-11-20T08:30:00.000Z",
                "answeredAt": null,
                "answeredBy": null,
            }),
        );
        let request: CreateApproval = serde_json::from_value(serde_json::json!({
            "botId": "bot-ops", "action": "spend", "summary": "买域名"
        }))
        .unwrap();
        assert_eq!(request.path, None);
        assert_eq!(serde_json::to_value(ApprovalDecision::Denied).unwrap(), "denied");
    }

    #[test]
    fn message_event_json() {
        assert_round_trip(
            &Event::Message {
                id: "m1".into(),
                kind: MessageKind::Text,
                body: "hello".into(),
                created_at: "2026-10-03T19:00:00.000Z".into(),
            },
            serde_json::json!({
                "type": "message",
                "id": "m1",
                "kind": "text",
                "body": "hello",
                "createdAt": "2026-10-03T19:00:00.000Z"
            }),
        );
    }

    #[test]
    fn queued_status_event_json() {
        assert_round_trip(
            &Event::Status {
                id: "t1".into(),
                task_id: "t1".into(),
                bot_id: "bot-ada".into(),
                bot_name: "Ada".into(),
                state: TaskState::Queued,
                label: TaskState::Queued.label().into(),
                created_at: "2026-10-03T19:00:01.000Z".into(),
                updated_at: "2026-10-03T19:00:01.000Z".into(),
                container_up: false,
                failure: None,
                waiting_for: None,
                progress: None,
            },
            serde_json::json!({
                "type": "status",
                "id": "t1",
                "taskId": "t1",
                "botId": "bot-ada",
                "botName": "Ada",
                "state": "queued",
                "label": "排队",
                "createdAt": "2026-10-03T19:00:01.000Z",
                "updatedAt": "2026-10-03T19:00:01.000Z",
                "containerUp": false
            }),
        );
    }

    #[test]
    fn status_waiting_for_is_optional() {
        let blocked = Event::Status {
            id: "t2".into(),
            task_id: "t2".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            state: TaskState::Blocked,
            label: TaskState::Blocked.label().into(),
            created_at: "2026-10-03T19:00:01.000Z".into(),
            updated_at: "2026-10-03T19:00:01.000Z".into(),
            container_up: false,
            failure: None,
            waiting_for: Some(WaitingFor::Approval),
            progress: None,
        };
        let json = serde_json::to_value(&blocked).unwrap();
        assert_eq!(json["waitingFor"], "approval");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), blocked);
        let takeover = Event::Status {
            id: "t3".into(),
            task_id: "t3".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            state: TaskState::Blocked,
            label: TaskState::Blocked.label().into(),
            created_at: "2026-10-03T19:00:01.000Z".into(),
            updated_at: "2026-10-03T19:00:01.000Z".into(),
            container_up: false,
            failure: None,
            waiting_for: Some(WaitingFor::Takeover),
            progress: None,
        };
        assert_eq!(serde_json::to_value(&takeover).unwrap()["waitingFor"], "takeover");
        let old = serde_json::json!({
            "type": "status",
            "id": "t1",
            "taskId": "t1",
            "botId": "bot-ada",
            "botName": "Ada",
            "state": "queued",
            "label": "排队",
            "createdAt": "2026-10-03T19:00:01.000Z",
            "updatedAt": "2026-10-03T19:00:01.000Z",
            "containerUp": false
        });
        let Event::Status { waiting_for, .. } = serde_json::from_value(old).unwrap() else { panic!() };
        assert_eq!(waiting_for, None, "old JSON without the field still parses");
    }

    #[test]
    fn status_progress_is_optional() {
        let running = Event::Status {
            id: "t1".into(),
            task_id: "t1".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            state: TaskState::Running,
            label: TaskState::Running.label().into(),
            created_at: "2026-10-03T19:00:01.000Z".into(),
            updated_at: "2026-10-03T19:00:02.000Z".into(),
            container_up: true,
            failure: None,
            waiting_for: None,
            progress: Some("在沙箱里跑".into()),
        };
        let json = serde_json::to_value(&running).unwrap();
        assert_eq!(json["progress"], "在沙箱里跑");
        assert!(json.get("waitingFor").is_none());
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), running);

        let old = serde_json::json!({
            "type": "status",
            "id": "t1",
            "taskId": "t1",
            "botId": "bot-ada",
            "botName": "Ada",
            "state": "running",
            "label": "进行",
            "createdAt": "2026-10-03T19:00:01.000Z",
            "updatedAt": "2026-10-03T19:00:01.000Z",
            "containerUp": true
        });
        let parsed: Event = serde_json::from_value(old).unwrap();
        let Event::Status { progress, .. } = &parsed else { panic!("status") };
        assert_eq!(progress, &None, "old JSON without the field still parses");
        assert!(serde_json::to_value(&parsed).unwrap().get("progress").is_none(), "absent stays absent");
    }

    #[test]
    fn delivery_failure_is_optional() {
        let failed = Event::Delivery {
            id: "d2".into(),
            summary: "已写入结果".into(),
            path: "tasks/t1/result.txt".into(),
            briefing: false,
            created_at: "2026-10-03T19:00:02.000Z".into(),
            failure: Some("模型没回：模型服务返回 500".into()),
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["failure"], "模型没回：模型服务返回 500");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), failed);
        let old = serde_json::json!({
            "type": "delivery", "id": "d1", "summary": "草图", "path": "out/sketch.png",
            "briefing": false, "createdAt": "2026-10-03T19:00:02.000Z"
        });
        let Event::Delivery { failure, .. } = serde_json::from_value(old).unwrap() else { panic!() };
        assert_eq!(failure, None, "old JSON without the field still parses");
    }

    #[test]
    fn delivery_event_json() {
        assert_round_trip(
            &Event::Delivery {
                id: "d1".into(),
                summary: "草图".into(),
                path: "out/sketch.png".into(),
                briefing: true,
                created_at: "2026-10-03T19:00:02.000Z".into(),
                failure: None,
            },
            serde_json::json!({
                "type": "delivery",
                "id": "d1",
                "summary": "草图",
                "path": "out/sketch.png",
                "briefing": true,
                "createdAt": "2026-10-03T19:00:02.000Z"
            }),
        );
    }

    #[test]
    fn schedule_json() {
        let request: CreateSchedule = serde_json::from_value(serde_json::json!({
            "groupId": "g1",
            "assigneeBotId": "bot-ada",
            "cron": "0 9 * * *",
            "prompt": "早报"
        }))
        .unwrap();
        assert_eq!(request.group_id, "g1");
        assert_eq!(request.assignee_bot_id, "bot-ada");

        let schedule = Schedule {
            id: "s1".into(),
            group_id: "g1".into(),
            assignee_bot_id: "bot-ada".into(),
            cron: "0 9 * * *".into(),
            prompt: "早报".into(),
            next_run_at: "2026-10-08T09:00:00.000Z".into(),
            last_task_id: None,
        };
        let value = serde_json::to_value(&schedule).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": "s1",
                "groupId": "g1",
                "assigneeBotId": "bot-ada",
                "cron": "0 9 * * *",
                "prompt": "早报",
                "nextRunAt": "2026-10-08T09:00:00.000Z",
                "lastTaskId": null
            })
        );
        let back: Schedule = serde_json::from_value(value).unwrap();
        assert_eq!(back, schedule);

        let ran = Schedule {
            last_task_id: Some("t1".into()),
            ..schedule
        };
        assert_eq!(serde_json::to_value(&ran).unwrap()["lastTaskId"], "t1");
    }

    #[test]
    fn choice_event_json() {
        let options = vec![
            ChoiceOption { id: "a".into(), label: "方案 A".into(), primary: true },
            ChoiceOption { id: "b".into(), label: "方案 B".into(), primary: false },
        ];
        assert_round_trip(
            &Event::Choice {
                id: "c1".into(),
                bot_id: "bot-ada".into(),
                bot_name: "Ada".into(),
                prompt: "用哪个方案？".into(),
                options: options.clone(),
                multi: false,
                choice: None,
                created_at: "2026-10-03T19:00:03.000Z".into(),
                answered_at: None,
                answered_by: None,
            },
            serde_json::json!({
                "type": "choice",
                "id": "c1",
                "botId": "bot-ada",
                "botName": "Ada",
                "prompt": "用哪个方案？",
                "options": [
                    { "id": "a", "label": "方案 A", "primary": true },
                    { "id": "b", "label": "方案 B", "primary": false }
                ],
                "multi": false,
                "choice": null,
                "createdAt": "2026-10-03T19:00:03.000Z",
                "answeredAt": null,
                "answeredBy": null
            }),
        );
        let request: CreateChoice = serde_json::from_value(serde_json::json!({
            "botId": "bot-ada",
            "prompt": "用哪个方案？",
            "options": [{ "id": "a", "label": "方案 A" }]
        }))
        .unwrap();
        assert_eq!(request.bot_id, "bot-ada");
        assert!(!request.multi && !request.options[0].primary, "multi and primary default to false");
        let answer: AnswerChoice = serde_json::from_value(serde_json::json!({ "choice": ["b"] })).unwrap();
        assert_eq!(answer.choice, vec!["b".to_string()]);
        assert_eq!(answer.answered_by, None, "answeredBy is optional");
    }

    #[test]
    fn answered_by_is_optional_everywhere() {
        // Old events without the field still parse.
        let old: Event = serde_json::from_value(serde_json::json!({
            "type": "approval", "id": "a1", "botId": "b", "botName": "B", "action": "send",
            "summary": "发邮件", "path": null, "decision": "approved",
            "createdAt": "2026-11-20T08:30:00.000Z", "answeredAt": "2026-11-20T08:31:00.000Z"
        }))
        .unwrap();
        assert!(matches!(old, Event::Approval { answered_by: None, .. }));
        let answer: Answer = serde_json::from_value(serde_json::json!({ "answeredBy": "Panda 的 Mac" })).unwrap();
        assert_eq!(answer.answered_by.as_deref(), Some("Panda 的 Mac"));
        assert_eq!(serde_json::to_value(Answer::default()).unwrap(), serde_json::json!({}));
        assert_eq!(clean_answered_by(None), Ok(None));
        assert_eq!(clean_answered_by(Some("  ")), Ok(None));
        assert_eq!(clean_answered_by(Some(" 手机 ")), Ok(Some("手机".into())));
        assert!(clean_answered_by(Some("a\nb")).is_err());
        assert!(clean_answered_by(Some(&"字".repeat(ANSWERED_BY_MAX + 1))).is_err());
    }

    #[test]
    fn schedule_entry_flattens_and_adds_paused() {
        let entry = ScheduleEntry {
            schedule: Schedule {
                id: "s1".into(),
                group_id: "g1".into(),
                assignee_bot_id: "bot-ada".into(),
                cron: "0 9 * * *".into(),
                prompt: "早报".into(),
                next_run_at: "2026-10-08T09:00:00.000Z".into(),
                last_task_id: None,
            },
            paused: true,
        };
        let value = serde_json::to_value(&Schedules { schedules: vec![entry.clone()] }).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "schedules": [{
                "id": "s1",
                "groupId": "g1",
                "assigneeBotId": "bot-ada",
                "cron": "0 9 * * *",
                "prompt": "早报",
                "nextRunAt": "2026-10-08T09:00:00.000Z",
                "lastTaskId": null,
                "paused": true
            }]})
        );
        let back: Schedules = serde_json::from_value(value).unwrap();
        assert_eq!(back.schedules[0], entry);
        let patch: UpdateSchedule = serde_json::from_value(serde_json::json!({ "paused": false })).unwrap();
        assert!(!patch.paused);
    }

    #[test]
    fn groups_json() {
        let groups = Groups {
            groups: vec![
                Group { id: "g1".into(), name: "设计群".into(), tenant_id: "local".into() },
                Group { id: "g2".into(), name: "运营群".into(), tenant_id: "local".into() },
            ],
        };
        let value = serde_json::to_value(&groups).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "groups": [
                    { "id": "g1", "name": "设计群", "tenantId": "local" },
                    { "id": "g2", "name": "运营群", "tenantId": "local" }
                ]
            })
        );
        let back: Groups = serde_json::from_value(value).unwrap();
        assert_eq!(back, groups);

        let empty = Groups { groups: vec![] };
        assert_eq!(serde_json::to_value(&empty).unwrap(), serde_json::json!({ "groups": [] }));
    }

    #[test]
    fn task_state_labels() {
        assert_eq!(TaskState::Queued.label(), "排队");
        assert_eq!(TaskState::Running.label(), "进行");
        assert_eq!(TaskState::Blocked.label(), "卡住");
        assert_eq!(TaskState::Done.label(), "完成");
        assert_eq!(serde_json::to_string(&TaskState::Queued).unwrap(), "\"queued\"");
        assert_eq!(serde_json::to_string(&TaskState::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&TaskState::Blocked).unwrap(), "\"blocked\"");
        assert_eq!(serde_json::to_string(&TaskState::Done).unwrap(), "\"done\"");
    }
}
