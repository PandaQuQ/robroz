use serde::{Deserialize, Serialize};

/// Client-visible text the host writes; see the module docs.
pub mod host_text;

/// Stable machine codes for task card states, results and failures.
pub mod codes;
/// Stable machine `code` for host error bodies.
pub mod error_codes;
/// Deprecated Chinese card copy that used to live in `robroz-card`.
pub mod legacy;

pub use codes::{split_failure, CardStatusCode, DeliveryBadge, FailureCode};

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
/// When the message is a follow-up that joined a bro's still-open task
/// instead of starting one, `task` is that task (its current state) and
/// `appendedToTaskId` is its id. Absent otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Said {
    pub message: Message,
    pub task: Option<Task>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appended_to_task_id: Option<String>,
}

/// `@名字 rest` → `(member id, rest)`. `members` is `(id, name)`. The longest
/// matching name wins (`@bro 2` beats `@bro`), the name must end at a space or
/// the end, full-width `＠` works, and an empty `rest` means no task.
pub fn route_mention<'a>(
    body: &str,
    members: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<(String, String)> {
    let body = body.trim();
    let after = body.strip_prefix('@').or_else(|| body.strip_prefix('＠'))?;
    let mut best: Option<(&str, &str, &str)> = None;
    for (id, name) in members {
        let Some(rest) = after.strip_prefix(name) else {
            continue;
        };
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
    /// The user who created the group. Only they or an admin may rename or
    /// delete it. Omitted for groups created before this field existed (then
    /// only an admin may) and for DMs. `createdBy` alone cannot tell a DM
    /// from an old group; use [`Self::dm`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// A bro DM (`groups.dm_bot_id` is set). Always serialized, so the
    /// client can hide rename and delete with
    /// `!group.dm && (me.role == admin || group.createdBy == me.userId)`.
    #[serde(default)]
    pub dm: bool,
}

/// `PATCH /api/groups/{id}` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameGroup {
    pub name: String,
}

/// `GET /api/groups/{id}/members`.
///
/// `users` is everyone connected to the group in the data model: the user
/// who created it, and the owner of every bro in it (users are not members
/// of a group on their own). For a DM it is the DM's user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMembers {
    pub group_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Bros in the group, in the order they were added.
    pub bros: Vec<GroupMemberBro>,
    /// Users, the creator first, then by name.
    pub users: Vec<GroupMemberUser>,
}

/// A bro in [`GroupMembers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberBro {
    pub bot_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
}

/// A user in [`GroupMembers`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberUser {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// This user created the group.
    pub creator: bool,
    /// The bros in this group that this user owns.
    pub bro_ids: Vec<String>,
}

/// `GET /api/groups/{id}/search?q=&limit=`: default and largest `limit`.
pub const GROUP_SEARCH_DEFAULT_LIMIT: usize = 20;
pub const GROUP_SEARCH_MAX_LIMIT: usize = 100;

/// `GET /api/groups/{id}/search` body, best match first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSearch {
    pub results: Vec<GroupSearchHit>,
}

/// One message that matches. `snippet` is plain text around the match (no
/// markup), cut with 「…」.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSearchHit {
    pub message_id: String,
    pub snippet: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
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
    /// v0.2: the person who sent it. Every stored message is a person's
    /// today; v0.1 rows are backfilled to the bootstrap admin. Omitted when
    /// unknown (a bro never writes a `message`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<String>,
    /// v0.2: that person's display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
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
    #[serde(
        default,
        rename = "answeredBy",
        skip_serializing_if = "Option::is_none"
    )]
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
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
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
    /// A model asked to call an MCP tool that is not read-only.
    Call,
    /// Confirm card for adding an allow rule (`POST /api/settings/review-rules`).
    /// `summary` is the rule sentence, verbatim. Explicit rename: `rename_all`
    /// would give `ruleadd`.
    #[serde(rename = "rule_add")]
    RuleAdd,
    /// Confirm card for deleting an allow rule
    /// (`DELETE /api/settings/review-rules/{id}`). `summary` is the rule
    /// sentence, verbatim.
    #[serde(rename = "rule_delete")]
    RuleDelete,
    /// An action value this client does not recognise. The client shows a
    /// generic approval card. The host must never emit this variant.
    #[serde(other)]
    Unknown,
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

/// `reason` on an expired approval card. The host writes this exact string;
/// the client compares `reason == APPROVAL_EXPIRED_REASON` and must not
/// hard-code the text.
pub const APPROVAL_EXPIRED_REASON: &str = "审批已过期";

/// Log-only `error` on a 403 `not_bro_owner`, or `approval_forbidden` with
/// `reason` `not_owner`. The client does not show this string; it picks
/// copy from `code` / `reason` and `ownerName` via its own i18n.
pub const NOT_BRO_OWNER: &str = "只有这个 bro 的主人能这样做";

/// An allow rule for auto-review, as listed by `GET /api/settings/review-rules`.
/// Only rules the owner confirmed on a `rule_add` card have this shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRule {
    pub id: String,
    /// The owner's sentence, trimmed, one line.
    pub text: String,
    pub created_at: String,
}

/// `GET /api/settings/review-rules`: confirmed rules, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRules {
    pub rules: Vec<ReviewRule>,
}

/// `POST /api/settings/review-rules` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateReviewRule {
    pub text: String,
}

/// 202 body of `POST` and `DELETE /api/settings/review-rules`: the confirm
/// card that must be approved before the list changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRuleCreated {
    pub approval_id: String,
}

/// `botId` on a rule card the owner raised from the settings page. That card
/// belongs to no bro and no chat: it is shown on the settings page, fetched
/// with `GET /api/approvals/{id}`, and never appears in a group's events.
pub const SETTINGS_APPROVAL_BOT_ID: &str = "settings";
/// `botName` on a settings-page rule card.
pub const SETTINGS_APPROVAL_BOT_NAME: &str = "设置";

/// A rule sentence is one line, at most this many characters (the same
/// limit as an approval summary, because the card shows it verbatim).
pub const REVIEW_RULE_MAX: usize = APPROVAL_SUMMARY_MAX;

/// `progress` on a group's status card while its task waits for an approval
/// card that the host sent to the owner's DM with that bro instead of the
/// group. Old clients already show `progress` under 「等你批准」.
/// v0.2 hosts no longer send it: group cards stay in the group and the DM
/// gets an [`Event::ApprovalReminder`]. Kept so older clients still map it.
pub const APPROVAL_SENT_TO_OWNER_DM_NOTICE: &str = "已发到主人私聊";

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
    /// Added with accounts (v0.2): the user who owns this bro (its creator).
    /// Only the owner may rename it, open its DM, add it to a group, or
    /// answer its cards. Absent from older hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
}

/// Host role of a user (v0.2). `admin` manages the host itself; it does not
/// own or edit other people's bros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    Member,
}

/// `GET /api/me`: the user the request is authenticated as. With the v0.1
/// setup (one `ROBROZ_TOKEN`, or loopback without a token) this is always
/// the bootstrap admin [`BOOTSTRAP_ADMIN_ID`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub id: String,
    pub name: String,
    pub role: UserRole,
}

/// Id of the admin the v0.2 migration creates. Existing bros, DMs and the
/// env token (`ROBROZ_TOKEN` / `ROBROZ_TOKEN_FILE`) belong to this user.
pub const BOOTSTRAP_ADMIN_ID: &str = "local-admin";
/// Display name of the bootstrap admin.
pub const BOOTSTRAP_ADMIN_NAME: &str = "admin";

/// `code` on every 403 from the bro routes (`PATCH /api/bros/{id}`,
/// `POST /api/bros/{id}/dm`, `POST /api/groups/{id}/members` with someone
/// else's bro): the caller is not the bro's owner. The body is
/// [`NotBroOwner`]. 401 stays "bad token".
pub const FORBIDDEN_NOT_BRO_OWNER: &str = "not_bro_owner";

/// `code` on a 403 from `POST /api/approvals/{id}/approve` or `/deny`.
/// The body is [`ApprovalForbidden`]; `reason` says why.
pub const APPROVAL_FORBIDDEN: &str = "approval_forbidden";

/// Why an approval answer was refused with 403.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalForbiddenReason {
    /// The caller is not the bro's owner (or, on a settings card, not an admin).
    NotOwner,
    /// The card was already approved or denied.
    Decided,
    /// The card expired (`reason` = [`APPROVAL_EXPIRED_REASON`]).
    Expired,
}

/// 403 body of approve / deny. `code` is always [`APPROVAL_FORBIDDEN`].
/// `error` is a host text ([`NOT_BRO_OWNER`],
/// [`host_text::ALREADY_ANSWERED`] or [`APPROVAL_EXPIRED_REASON`]) for
/// logs only; the client picks copy from `code` and `reason` (and
/// [`Self::owner_name`]) via its own i18n.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalForbidden {
    pub error: String,
    pub code: String,
    pub reason: ApprovalForbiddenReason,
    /// The bro's owner, for "Only {owner} can approve this.". Absent on
    /// settings cards (no bro).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
}

/// 403 body of the bro routes when the caller is not the owner
/// (`PATCH /api/bros/{id}`, `POST /api/bros/{id}/dm`,
/// `POST /api/groups/{id}/members`). `code` is always
/// [`FORBIDDEN_NOT_BRO_OWNER`]. `error` is for logs only; the client
/// shows "Only {owner} can change this bro." from [`Self::owner_name`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotBroOwner {
    pub error: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
}

/// State of an [`Event::ApprovalReminder`]: the linked card's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderState {
    Waiting,
    Approved,
    Denied,
    Expired,
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
    pub const ALL: [MemoryStatus; 3] = [
        MemoryStatus::Pending,
        MemoryStatus::Kept,
        MemoryStatus::Rejected,
    ];

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
        [ConfirmSubject::Memory, ConfirmSubject::Skill]
            .into_iter()
            .find(|s| s.as_str() == value)
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
        let serde_json::Value::Object(mut fields) = serde_json::from_str(payload.trim()).ok()?
        else {
            return None;
        };
        fields.insert("type".into(), serde_json::Value::String("memory".into()));
        Some(serde_json::Value::Object(fields).to_string())
    }

    /// The `proposals.jsonl` line for a `robroz-skill:` payload, with
    /// `"type":"skill"` set. `None` when the payload is not a JSON object.
    pub fn skill_line(payload: &str) -> Option<String> {
        let serde_json::Value::Object(mut fields) = serde_json::from_str(payload.trim()).ok()?
        else {
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

/// An error with a stable machine code next to the host text. Bro-owner
/// 403s use the richer [`NotBroOwner`] body. Clients that only read
/// `error` keep working; `error` on owner 403s is for logs only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodedError {
    pub error: String,
    pub code: String,
}

/// Where the effective search config comes from. Settings win over
/// environment variables. Absent when search is not set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchSettingsSource {
    Env,
    Settings,
}

impl SearchSettingsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SearchSettingsSource::Env => "env",
            SearchSettingsSource::Settings => "settings",
        }
    }
}

/// `GET /api/settings/search` and the `PUT` response. `provider` is `null`
/// when search is not set up. The key is never returned; only `hasKey` and
/// `keyLast4`. `source` is omitted when nothing is configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSettings {
    pub provider: Option<String>,
    pub url: Option<String>,
    pub has_key: bool,
    pub key_last4: Option<String>,
    /// `settings` if any search setting is saved, otherwise `env`.
    /// Absent when search is not set up, so old JSON is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SearchSettingsSource>,
}

/// `POST /api/settings/search/test`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTestResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where the effective model config comes from. Settings win over
/// environment variables. Absent when the model is not set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSettingsSource {
    Env,
    Settings,
}

impl ModelSettingsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelSettingsSource::Env => "env",
            ModelSettingsSource::Settings => "settings",
        }
    }
}

/// `GET /api/settings/model` and the `PUT` response. Unset fields are
/// `null`. The key is never returned; only `hasKey` and `keyLast4`.
/// `source` is omitted when nothing is configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub has_key: bool,
    pub key_last4: Option<String>,
    /// `settings` if any model setting is saved, otherwise `env`.
    /// Absent when the model is not set up, so old JSON is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ModelSettingsSource>,
}

/// `POST /api/settings/model/test` when the provider answers HTTP 401 or 403.
/// Other failures keep their existing messages. The frontend can match
/// `error == MODEL_TEST_BAD_KEY`.
pub const MODEL_TEST_BAD_KEY: &str = "密钥不对或没填";

/// `POST /api/settings/model/test`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTestResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Where an MCP secret is applied: a child-process environment variable, or an HTTP header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpSecretIn {
    Env,
    Header,
}

impl McpSecretIn {
    pub fn as_str(self) -> &'static str {
        match self {
            McpSecretIn::Env => "env",
            McpSecretIn::Header => "header",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "env" => Some(McpSecretIn::Env),
            "header" => Some(McpSecretIn::Header),
            _ => None,
        }
    }
}

/// A secret as the API returns it. The value itself is never included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSecretPublic {
    pub name: String,
    #[serde(rename = "in")]
    pub placement: McpSecretIn,
    pub has_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_last4: Option<String>,
}

/// One MCP server, minus secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub secrets: Vec<McpSecretPublic>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_count: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
    /// For stdio servers: whether the current command and args have been approved.
    /// Omitted for HTTP servers. New stdio servers start as `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_approved: Option<bool>,
    /// When the current command and args were approved. Omitted when not approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_approved_at: Option<String>,
}

/// `GET /api/mcp/servers` or `GET /api/groups/{id}/mcp`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServers {
    pub servers: Vec<McpServer>,
}

/// One secret in a create or patch body. `key` omitted leaves the stored value
/// alone (patch only); `null` clears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSecretInput {
    pub name: String,
    #[serde(default, rename = "in")]
    pub placement: Option<McpSecretIn>,
    #[serde(default, deserialize_with = "mcp_opt_opt_string")]
    pub key: Option<Option<String>>,
}

fn mcp_opt_opt_string<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

/// `POST /api/mcp/servers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMcpServer {
    pub name: String,
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub secrets: Option<Vec<McpSecretInput>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// `PATCH /api/mcp/servers/{id}`. Only given fields change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMcpServer {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default, deserialize_with = "mcp_opt_opt_string")]
    pub command: Option<Option<String>>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default, deserialize_with = "mcp_opt_opt_string")]
    pub url: Option<Option<String>>,
    #[serde(default)]
    pub secrets: Option<Vec<McpSecretInput>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// One tool from `GET /api/mcp/servers/{id}/tools`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub needs_approval: bool,
}

/// `GET /api/mcp/servers/{id}/tools`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTools {
    pub tools: Vec<McpTool>,
}

/// `POST /api/mcp/servers/{id}/test`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_count: Option<u32>,
}

/// Error code returned when a stdio MCP command has not been approved yet.
pub const MCP_COMMAND_NOT_APPROVED: &str = "command_not_approved";

/// `409` from `/test` or `/tools` (and any other path that would start an
/// unapproved stdio command).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCommandNotApproved {
    pub error: String,
    pub command: String,
    pub args: Vec<String>,
}

impl McpCommandNotApproved {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            error: MCP_COMMAND_NOT_APPROVED.to_string(),
            command: command.into(),
            args,
        }
    }
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
        /// v0.2: see [`Message::author_user_id`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author_user_id: Option<String>,
        /// v0.2: see [`Message::author_name`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author_name: Option<String>,
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
        /// v0.2: the code form of `label` (see [`CardStatusCode`]). The client
        /// renders its own text from it. Absent from an older host.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status_code: Option<CardStatusCode>,
        /// v0.2: the code form of `failure` (see [`FailureCode`]). Present
        /// exactly when `failure` is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_code: Option<FailureCode>,
        /// v0.2: the raw reason of `failure` without its prefix, so the
        /// client adds exactly one prefix. Absent when the code needs no
        /// detail (see [`FailureCode`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_detail: Option<String>,
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
        /// Added after the freeze: the delivery needs a settings page. `"search"`
        /// means the user has not set up web search. Absent otherwise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        needs_setup: Option<String>,
        /// Added after the freeze: the task this delivery belongs to.
        /// Absent when unknown, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        /// Added after the freeze: the bro that produced this delivery.
        /// Absent when unknown, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bot_id: Option<String>,
        /// Added after the freeze: the bro's display name, for attributing the card.
        /// Absent when unknown, so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bot_name: Option<String>,
        /// v0.2: the code form of the corner badge (see [`DeliveryBadge`]).
        /// Present exactly when `briefing` is true.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        badge: Option<DeliveryBadge>,
        /// v0.2: the code form of `failure`; see [`Event::Status`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_code: Option<FailureCode>,
        /// v0.2: the raw reason of `failure` without its prefix.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_detail: Option<String>,
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
        /// MCP tool call waiting for approval. Absent on older cards.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_server: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mcp_tool: Option<String>,
        /// Why review blocked this action, or why the card settled (for
        /// example [`APPROVAL_EXPIRED_REASON`]). Absent when there is none,
        /// so old JSON is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// When this approval card expires (RFC 3339, same style as
        /// `createdAt`). Absent on cards that do not expire, so old JSON
        /// is unchanged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<String>,
        /// Added with accounts (v0.2): the bro's owner, the only user who may
        /// approve or deny. Absent on settings cards and from older hosts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner_user_id: Option<String>,
        /// The owner's display name ("Waiting for {owner}").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner_name: Option<String>,
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
    /// Added with accounts (v0.2): one line in the owner's DM with a bro,
    /// pointing at that bro's approval card in a group. No buttons; the
    /// card is answered in the group (`groupId`, `approvalId`). `state`
    /// follows the card; `answeredAt` is set once it is decided or expired.
    /// `id` is `reminder:{approvalId}`. Counts as unread in the DM.
    #[serde(rename = "approval_reminder")]
    ApprovalReminder {
        id: String,
        approval_id: String,
        group_id: String,
        group_name: String,
        bot_id: String,
        bot_name: String,
        owner_user_id: String,
        owner_name: String,
        state: ReminderState,
        created_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered_at: Option<String>,
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
        assert_eq!(
            MemoryScope::parse("bot:b1"),
            Some(MemoryScope::Bot("b1".into()))
        );
        for bad in ["", "users", "bot:", "bot: x", "group:g1", "User", "bot"] {
            assert_eq!(MemoryScope::parse(bad), None, "{bad:?}");
        }
        assert_eq!(MemoryScope::Bot("b1".into()).to_string(), "bot:b1");
        assert_eq!(MemoryScope::User.to_string(), "user");
        for k in MemoryKind::ALL {
            assert_eq!(MemoryKind::parse(k.as_str()), Some(k));
            assert_eq!(
                serde_json::to_value(k).unwrap(),
                serde_json::json!(k.as_str())
            );
        }
        for s in MemoryStatus::ALL {
            assert_eq!(MemoryStatus::parse(s.as_str()), Some(s));
        }
        let body: CreateMemory = serde_json::from_value(
            serde_json::json!({ "scope": "user", "kind": "habit", "text": "x" }),
        )
        .unwrap();
        assert_eq!(body.kind, MemoryKind::Habit);
        assert!(serde_json::from_value::<CreateMemory>(
            serde_json::json!({ "scope": "user", "kind": "mood", "text": "x" })
        )
        .is_err());
    }

    #[test]
    fn mention_routing() {
        use super::route_mention;
        let m = [("b1", "bro"), ("b2", "bro 2"), ("b3", "Ada")];
        assert_eq!(
            route_mention("@bro 2 写 README", m),
            Some(("b2".into(), "写 README".into()))
        );
        assert_eq!(
            route_mention("  ＠bro 看看日志 ", m),
            Some(("b1".into(), "看看日志".into()))
        );
        assert_eq!(
            route_mention("@Ada\t整理", m),
            Some(("b3".into(), "整理".into()))
        );
        for none in [
            "大家好",
            "@bro",
            "@bro   ",
            "@Bob 你好",
            "@brother 你好",
            "问 @Ada 一下",
            "@",
        ] {
            assert_eq!(route_mention(none, m), None, "{none:?}");
        }
        assert_eq!(
            route_mention("@ 你好", [("x", "")]),
            None,
            "empty name never matches"
        );
    }

    #[test]
    fn bro_default_names() {
        use super::next_bro_name;
        assert_eq!(next_bro_name([]), "bro");
        assert_eq!(next_bro_name(["bro"]), "bro 2");
        assert_eq!(next_bro_name(["bro", "bro 2"]), "bro 3");
        assert_eq!(
            next_bro_name(["bro", "bro 3"]),
            "bro 2",
            "fills the first gap"
        );
        assert_eq!(
            next_bro_name(["bro 2", "小明"]),
            "bro",
            "bro itself was renamed away"
        );
        let json = serde_json::to_value(super::Bro {
            id: "b".into(),
            name: "bro".into(),
            dm_group_id: Some("g".into()),
            owner_user_id: None,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "id": "b", "name": "bro", "dmGroupId": "g" })
        );
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
        assert!(
            matches!(old, Event::Approval { task_id: None, .. }),
            "old JSON parses"
        );
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            json,
            "absent stays absent"
        );
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
        assert!(matches!(
            skill,
            Event::Confirm {
                subject: ConfirmSubject::Skill,
                answered_by: None,
                ..
            }
        ));
    }

    #[test]
    fn proposal_lines_parse_or_are_skipped() {
        let line = Proposal::memory_line(r#"{"kind":"habit","text":"周报用中文","type":"skill"}"#)
            .unwrap();
        let Some(Proposal::Memory(m)) = Proposal::parse_line(&line) else {
            panic!("{line}")
        };
        assert_eq!(
            (m.kind, m.text.as_str(), m.scope),
            (MemoryKind::Habit, "周报用中文", None)
        );
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
        let scoped = Proposal::parse_line(
            r#"{"type":"memory","scope":"user","kind":"profile","text":"叫 Panda"}"#,
        );
        assert!(matches!(
            scoped,
            Some(Proposal::Memory(MemoryProposal { scope: Some(_), .. }))
        ));
        let skill = Proposal::skill_line(
            r#"{"slug":"weekly-report","title":"写周报","instruction":"先列三点","type":"memory"}"#,
        )
        .unwrap();
        let Some(Proposal::Skill(s)) = Proposal::parse_line(&skill) else {
            panic!("{skill}")
        };
        assert_eq!(
            (
                s.slug.as_str(),
                s.title.as_str(),
                s.instruction.as_str(),
                s.scope.as_deref()
            ),
            ("weekly-report", "写周报", "先列三点", None)
        );
        assert_eq!(Proposal::skill_line("不是 json"), None);
        assert_eq!(Proposal::skill_line("[1]"), None);
        let scoped_skill = Proposal::parse_line(
            r#"{"type":"skill","slug":"weekly-report","title":"写周报","instruction":"先列三点","scope":"user"}"#,
        );
        assert!(matches!(
            scoped_skill,
            Some(Proposal::Skill(SkillProposal { scope: Some(_), .. }))
        ));
    }

    #[test]
    fn skill_slug_and_confirm_id_round_trip() {
        use super::{
            parse_skill_slug, CreateSkill, MemorySource, MemoryStatus, Skill, SKILL_SLUG_MAX,
        };
        assert_eq!(parse_skill_slug(" weekly-report "), Some("weekly-report"));
        assert_eq!(parse_skill_slug("a"), Some("a"));
        assert_eq!(
            parse_skill_slug(&"a".repeat(SKILL_SLUG_MAX)),
            Some(&*"a".repeat(SKILL_SLUG_MAX))
        );
        for bad in [
            "",
            "Weekly",
            "weekly_report",
            "-weekly",
            "weekly-",
            "weekly--report",
            " weekly report ",
            &"a".repeat(SKILL_SLUG_MAX + 1),
        ] {
            assert_eq!(parse_skill_slug(bad), None, "{bad:?}");
        }
        let body: CreateSkill = serde_json::from_value(serde_json::json!({
            "scope": "user", "slug": "weekly-report", "title": "写周报", "instruction": "先列三点"
        }))
        .unwrap();
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
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            json,
            "absent stays absent"
        );
    }

    #[test]
    fn memory_confirm_id_is_optional() {
        let json = serde_json::json!({
            "id": "m1", "scope": "user", "kind": "habit", "text": "x", "source": "bot",
            "status": "pending", "createdAt": "2026-11-20T08:30:00.000Z", "updatedAt": "2026-11-20T08:30:00.000Z"
        });
        let old: Memory = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.confirm_id, None);
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            json,
            "absent stays absent"
        );
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
                mcp_server: None,
                mcp_tool: None,
                reason: None,
                expires_at: None,
                owner_name: None,
                owner_user_id: None,
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
        assert_eq!(
            serde_json::to_value(ApprovalDecision::Denied).unwrap(),
            "denied"
        );
        assert_eq!(serde_json::to_value(ApprovalAction::Call).unwrap(), "call");

        assert_round_trip(
            &Event::Approval {
                id: "a2".into(),
                bot_id: "bot-ops".into(),
                bot_name: "运维".into(),
                action: ApprovalAction::Call,
                summary: "调用 notes.write_note：{\"title\":\"hi\"}".into(),
                path: None,
                decision: None,
                created_at: "2026-11-21T01:00:00.000Z".into(),
                answered_at: None,
                answered_by: None,
                task_id: Some("task-1".into()),
                mcp_server: Some("notes".into()),
                mcp_tool: Some("write_note".into()),
                reason: None,
                expires_at: None,
                owner_name: None,
                owner_user_id: None,
            },
            serde_json::json!({
                "type": "approval",
                "id": "a2",
                "botId": "bot-ops",
                "botName": "运维",
                "action": "call",
                "summary": "调用 notes.write_note：{\"title\":\"hi\"}",
                "path": null,
                "decision": null,
                "createdAt": "2026-11-21T01:00:00.000Z",
                "answeredAt": null,
                "answeredBy": null,
                "taskId": "task-1",
                "mcpServer": "notes",
                "mcpTool": "write_note",
            }),
        );
    }

    #[test]
    fn approval_action_is_forward_compatible() {
        for (value, action) in [
            ("delete", ApprovalAction::Delete),
            ("send", ApprovalAction::Send),
            ("spend", ApprovalAction::Spend),
            ("call", ApprovalAction::Call),
        ] {
            assert_eq!(serde_json::to_value(action).unwrap(), value);
            let back: ApprovalAction = serde_json::from_value(value.into()).unwrap();
            assert_eq!(back, action, "{value}");
        }

        let unknown: ApprovalAction = serde_json::from_value("rule_edit".into()).unwrap();
        assert_eq!(unknown, ApprovalAction::Unknown);
        let also: ApprovalAction = serde_json::from_value("something_else".into()).unwrap();
        assert_eq!(also, ApprovalAction::Unknown);

        let old = serde_json::json!({
            "type": "approval", "id": "a1", "botId": "b", "botName": "B", "action": "send",
            "summary": "发邮件", "path": null, "decision": null,
            "createdAt": "2026-11-20T08:30:00.000Z", "answeredAt": null, "answeredBy": null
        });
        let event: Event = serde_json::from_value(old.clone()).unwrap();
        assert!(
            matches!(
                event,
                Event::Approval {
                    reason: None,
                    expires_at: None,
                    ..
                }
            ),
            "old JSON without reason/expiresAt parses"
        );
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            old,
            "omitted fields stay absent"
        );
        // Field order is the Event struct order, with no reason/expiresAt keys.
        let expected = r#"{"type":"approval","id":"a1","botId":"b","botName":"B","action":"send","summary":"发邮件","path":null,"decision":null,"createdAt":"2026-11-20T08:30:00.000Z","answeredAt":null,"answeredBy":null}"#;
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            expected,
            "JSON without the new fields is unchanged"
        );

        let with_fields: Event = serde_json::from_value(serde_json::json!({
            "type": "approval", "id": "a1", "botId": "b", "botName": "B", "action": "send",
            "summary": "发邮件", "path": null, "decision": "denied",
            "createdAt": "2026-11-20T08:30:00.000Z", "answeredAt": "2026-11-21T08:30:00.000Z",
            "answeredBy": null, "reason": APPROVAL_EXPIRED_REASON,
            "expiresAt": "2026-11-21T08:30:00.000Z"
        }))
        .unwrap();
        assert!(matches!(
            with_fields,
            Event::Approval {
                reason: Some(ref r),
                expires_at: Some(_),
                ..
            } if r == APPROVAL_EXPIRED_REASON
        ));
        assert_eq!(APPROVAL_EXPIRED_REASON, "审批已过期");
    }

    #[test]
    fn rule_actions_use_snake_case_names() {
        for (value, action) in [
            ("rule_add", ApprovalAction::RuleAdd),
            ("rule_delete", ApprovalAction::RuleDelete),
        ] {
            assert_eq!(serde_json::to_value(action).unwrap(), value);
            let back: ApprovalAction = serde_json::from_value(value.into()).unwrap();
            assert_eq!(back, action);
        }
        // Not the `rename_all` spelling.
        let odd: ApprovalAction = serde_json::from_value("ruleadd".into()).unwrap();
        assert_eq!(odd, ApprovalAction::Unknown);
    }

    #[test]
    fn review_rule_types_are_camel_case() {
        let rule = ReviewRule {
            id: "r1".into(),
            text: "允许合并 CI 全绿的 PR".into(),
            created_at: "2026-11-24T10:00:00.000Z".into(),
        };
        assert_eq!(
            serde_json::to_value(ReviewRules {
                rules: vec![rule.clone()]
            })
            .unwrap(),
            serde_json::json!({"rules":[{"id":"r1","text":"允许合并 CI 全绿的 PR","createdAt":"2026-11-24T10:00:00.000Z"}]})
        );
        assert_eq!(
            serde_json::to_value(ReviewRuleCreated {
                approval_id: "a1".into()
            })
            .unwrap(),
            serde_json::json!({"approvalId":"a1"})
        );
        assert_eq!(SETTINGS_APPROVAL_BOT_ID, "settings");
        assert_eq!(APPROVAL_SENT_TO_OWNER_DM_NOTICE, "已发到主人私聊");
        assert_eq!(SETTINGS_APPROVAL_BOT_NAME, "设置");
        let body: CreateReviewRule =
            serde_json::from_value(serde_json::json!({"text":"x"})).unwrap();
        assert_eq!(body.text, "x");
    }

    #[test]
    fn message_event_carries_its_author_and_old_json_still_reads() {
        assert_round_trip(
            &Event::Message {
                id: "m1".into(),
                kind: MessageKind::Text,
                body: "hello".into(),
                created_at: "2026-10-03T19:00:00.000Z".into(),
                author_user_id: Some("local-admin".into()),
                author_name: Some("admin".into()),
            },
            serde_json::json!({
                "type": "message",
                "id": "m1",
                "kind": "text",
                "body": "hello",
                "createdAt": "2026-10-03T19:00:00.000Z",
                "authorUserId": "local-admin",
                "authorName": "admin"
            }),
        );
    }

    #[test]
    fn message_event_json() {
        assert_round_trip(
            &Event::Message {
                id: "m1".into(),
                kind: MessageKind::Text,
                body: "hello".into(),
                created_at: "2026-10-03T19:00:00.000Z".into(),
                author_name: None,
                author_user_id: None,
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
                status_code: None,
                failure_code: None,
                failure_detail: None,
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
            status_code: None,
            failure_code: None,
            failure_detail: None,
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
            status_code: None,
            failure_code: None,
            failure_detail: None,
        };
        assert_eq!(
            serde_json::to_value(&takeover).unwrap()["waitingFor"],
            "takeover"
        );
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
        let Event::Status { waiting_for, .. } = serde_json::from_value(old).unwrap() else {
            panic!()
        };
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
            status_code: None,
            failure_code: None,
            failure_detail: None,
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
        let Event::Status { progress, .. } = &parsed else {
            panic!("status")
        };
        assert_eq!(progress, &None, "old JSON without the field still parses");
        assert!(
            serde_json::to_value(&parsed)
                .unwrap()
                .get("progress")
                .is_none(),
            "absent stays absent"
        );
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
            needs_setup: None,
            task_id: None,
            bot_id: None,
            bot_name: None,
            badge: None,
            failure_code: None,
            failure_detail: None,
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["failure"], "模型没回：模型服务返回 500");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), failed);
        let old = serde_json::json!({
            "type": "delivery", "id": "d1", "summary": "草图", "path": "out/sketch.png",
            "briefing": false, "createdAt": "2026-10-03T19:00:02.000Z"
        });
        let Event::Delivery {
            failure,
            needs_setup,
            ..
        } = serde_json::from_value(old).unwrap()
        else {
            panic!()
        };
        assert_eq!(failure, None, "old JSON without the field still parses");
        assert_eq!(needs_setup, None);
    }

    #[test]
    fn delivery_needs_setup_is_optional() {
        let event = Event::Delivery {
            id: "d3".into(),
            summary: "已写入结果".into(),
            path: "tasks/t1/result.txt".into(),
            briefing: false,
            created_at: "2026-10-03T19:00:02.000Z".into(),
            failure: None,
            needs_setup: Some("search".into()),
            task_id: None,
            bot_id: None,
            bot_name: None,
            badge: None,
            failure_code: None,
            failure_detail: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["needsSetup"], "search");
        assert_eq!(serde_json::from_value::<Event>(json).unwrap(), event);
    }

    #[test]
    fn status_event_carries_codes_and_old_json_still_reads() {
        let failed = Event::Status {
            id: "t1".into(),
            task_id: "t1".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            state: TaskState::Done,
            label: TaskState::Done.label().into(),
            created_at: "2026-10-03T19:00:01.000Z".into(),
            updated_at: "2026-10-03T19:00:02.000Z".into(),
            container_up: false,
            failure: Some("模型没回：模型服务返回 500".into()),
            waiting_for: None,
            progress: None,
            status_code: Some(CardStatusCode::Done),
            failure_code: Some(FailureCode::ModelNoReply),
            failure_detail: Some("模型服务返回 500".into()),
        };
        assert_round_trip(
            &failed,
            serde_json::json!({
                "type": "status", "id": "t1", "taskId": "t1", "botId": "bot-ada",
                "botName": "Ada", "state": "done", "label": "完成",
                "createdAt": "2026-10-03T19:00:01.000Z",
                "updatedAt": "2026-10-03T19:00:02.000Z", "containerUp": false,
                "failure": "模型没回：模型服务返回 500",
                "statusCode": "done", "failureCode": "model_no_reply",
                "failureDetail": "模型服务返回 500"
            }),
        );
        let old = serde_json::json!({
            "type": "status", "id": "t1", "taskId": "t1", "botId": "bot-ada",
            "botName": "Ada", "state": "done", "label": "完成",
            "createdAt": "2026-10-03T19:00:01.000Z",
            "updatedAt": "2026-10-03T19:00:02.000Z", "containerUp": false,
            "failure": "模型没回：模型服务返回 500"
        });
        let Event::Status {
            status_code,
            failure_code,
            failure_detail,
            ..
        } = serde_json::from_value(old).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (status_code, failure_code, failure_detail),
            (None, None, None)
        );
    }

    #[test]
    fn delivery_event_carries_badge_and_failure_codes() {
        assert_round_trip(
            &Event::Delivery {
                id: "d1".into(),
                summary: "草图".into(),
                path: "out/sketch.png".into(),
                briefing: true,
                created_at: "2026-10-03T19:00:02.000Z".into(),
                failure: Some("没写出结果".into()),
                needs_setup: None,
                task_id: None,
                bot_id: None,
                bot_name: None,
                badge: Some(DeliveryBadge::Briefing),
                failure_code: Some(FailureCode::NoResult),
                failure_detail: None,
            },
            serde_json::json!({
                "type": "delivery", "id": "d1", "summary": "草图",
                "path": "out/sketch.png", "briefing": true,
                "createdAt": "2026-10-03T19:00:02.000Z",
                "failure": "没写出结果", "badge": "briefing",
                "failureCode": "no_result"
            }),
        );
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
                needs_setup: None,
                task_id: None,
                bot_id: None,
                bot_name: None,
                badge: None,
                failure_code: None,
                failure_detail: None,
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
    fn delivery_bot_fields_are_optional() {
        let old = serde_json::json!({
            "type": "delivery",
            "id": "d1",
            "summary": "草图",
            "path": "out/sketch.png",
            "briefing": true,
            "createdAt": "2026-10-03T19:00:02.000Z"
        });
        let event: Event = serde_json::from_value(old.clone()).unwrap();
        let Event::Delivery {
            task_id,
            bot_id,
            bot_name,
            ..
        } = event.clone()
        else {
            panic!("expected a delivery");
        };
        assert_eq!(task_id, None);
        assert_eq!(bot_id, None);
        assert_eq!(bot_name, None);
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value, old,
            "old JSON without the fields still deserializes and re-serializes unchanged"
        );
        // Canonical (sorted-key) bytes match; the new keys are not added.
        assert_eq!(
            serde_json::to_vec(&value).unwrap(),
            serde_json::to_vec(&old).unwrap()
        );
        let text = serde_json::to_string(&event).unwrap();
        assert!(!text.contains("taskId") && !text.contains("botId") && !text.contains("botName"));

        let mut with = old.clone();
        with["taskId"] = "t1".into();
        with["botId"] = "bot-ada".into();
        with["botName"] = "Ada".into();
        let event: Event = serde_json::from_value(with.clone()).unwrap();
        let Event::Delivery {
            task_id,
            bot_id,
            bot_name,
            ..
        } = &event
        else {
            panic!("expected a delivery");
        };
        assert_eq!(task_id.as_deref(), Some("t1"));
        assert_eq!(bot_id.as_deref(), Some("bot-ada"));
        assert_eq!(bot_name.as_deref(), Some("Ada"));
        assert_round_trip(&event, with);
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
            ChoiceOption {
                id: "a".into(),
                label: "方案 A".into(),
                primary: true,
            },
            ChoiceOption {
                id: "b".into(),
                label: "方案 B".into(),
                primary: false,
            },
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
        assert!(
            !request.multi && !request.options[0].primary,
            "multi and primary default to false"
        );
        let answer: AnswerChoice =
            serde_json::from_value(serde_json::json!({ "choice": ["b"] })).unwrap();
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
        assert!(matches!(
            old,
            Event::Approval {
                answered_by: None,
                ..
            }
        ));
        let answer: Answer =
            serde_json::from_value(serde_json::json!({ "answeredBy": "Panda 的 Mac" })).unwrap();
        assert_eq!(answer.answered_by.as_deref(), Some("Panda 的 Mac"));
        assert_eq!(
            serde_json::to_value(Answer::default()).unwrap(),
            serde_json::json!({})
        );
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
        let value = serde_json::to_value(&Schedules {
            schedules: vec![entry.clone()],
        })
        .unwrap();
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
        let patch: UpdateSchedule =
            serde_json::from_value(serde_json::json!({ "paused": false })).unwrap();
        assert!(!patch.paused);
    }

    #[test]
    fn groups_json() {
        let groups = Groups {
            groups: vec![
                Group {
                    id: "g1".into(),
                    name: "设计群".into(),
                    tenant_id: "local".into(),
                    created_by: None,
                    dm: false,
                },
                Group {
                    id: "g2".into(),
                    name: "运营群".into(),
                    tenant_id: "local".into(),
                    created_by: None,
                    dm: false,
                },
            ],
        };
        let value = serde_json::to_value(&groups).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "groups": [
                    { "id": "g1", "name": "设计群", "tenantId": "local", "dm": false },
                    { "id": "g2", "name": "运营群", "tenantId": "local", "dm": false }
                ]
            })
        );
        let back: Groups = serde_json::from_value(value).unwrap();
        assert_eq!(back, groups);

        let empty = Groups { groups: vec![] };
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            serde_json::json!({ "groups": [] })
        );
    }

    #[test]
    fn task_state_labels() {
        assert_eq!(TaskState::Queued.label(), "排队");
        assert_eq!(TaskState::Running.label(), "进行");
        assert_eq!(TaskState::Blocked.label(), "卡住");
        assert_eq!(TaskState::Done.label(), "完成");
        assert_eq!(
            serde_json::to_string(&TaskState::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::Blocked).unwrap(),
            "\"blocked\""
        );
        assert_eq!(serde_json::to_string(&TaskState::Done).unwrap(), "\"done\"");
    }

    #[test]
    fn mcp_command_approval_fields_are_optional() {
        let json = serde_json::json!({
            "id": "s1",
            "name": "git",
            "transport": "stdio",
            "command": "npx",
            "args": ["-y", "pkg"],
            "secrets": [],
            "enabled": true,
            "createdAt": "t0",
            "updatedAt": "t0"
        });
        let server: McpServer = serde_json::from_value(json).unwrap();
        assert_eq!(server.command_approved, None);
        assert_eq!(server.command_approved_at, None);

        let with_flag = McpServer {
            id: "s1".into(),
            name: "git".into(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: Some(vec!["-y".into()]),
            url: None,
            secrets: vec![],
            enabled: true,
            tool_count: None,
            created_at: "t0".into(),
            updated_at: "t0".into(),
            command_approved: Some(false),
            command_approved_at: None,
        };
        let value = serde_json::to_value(&with_flag).unwrap();
        assert_eq!(value["commandApproved"], false);
        assert!(value.get("commandApprovedAt").is_none());

        let denied = McpCommandNotApproved::new("npx", vec!["-y".into(), "pkg".into()]);
        assert_eq!(
            serde_json::to_value(&denied).unwrap(),
            serde_json::json!({
                "error": "command_not_approved",
                "command": "npx",
                "args": ["-y", "pkg"]
            })
        );
    }

    #[test]
    fn group_management_types_are_camel_case_and_optional() {
        use serde_json::json;
        let old: Group =
            serde_json::from_value(json!({"id":"g","name":"n","tenantId":"local"})).unwrap();
        assert_eq!(old.created_by, None);
        assert!(!old.dm, "missing dm defaults to false");
        let value = serde_json::to_value(&old).unwrap();
        assert!(value.get("createdBy").is_none());
        assert_eq!(value["dm"], false, "dm is always serialized");
        let dm = Group {
            id: "d".into(),
            name: "Ada".into(),
            tenant_id: "local".into(),
            created_by: None,
            dm: true,
        };
        assert_eq!(
            serde_json::to_value(&dm).unwrap(),
            json!({"id":"d","name":"Ada","tenantId":"local","dm":true})
        );
        let members = GroupMembers {
            group_id: "g".into(),
            created_by: Some("u".into()),
            bros: vec![GroupMemberBro {
                bot_id: "b".into(),
                name: "Ada".into(),
                owner_user_id: Some("u".into()),
                owner_name: None,
            }],
            users: vec![GroupMemberUser {
                user_id: "u".into(),
                name: Some("Mei".into()),
                creator: true,
                bro_ids: vec!["b".into()],
            }],
        };
        assert_eq!(
            serde_json::to_value(&members).unwrap(),
            json!({
                "groupId": "g",
                "createdBy": "u",
                "bros": [{"botId": "b", "name": "Ada", "ownerUserId": "u"}],
                "users": [{"userId": "u", "name": "Mei", "creator": true, "broIds": ["b"]}]
            })
        );
        let hit = GroupSearchHit {
            message_id: "m".into(),
            snippet: "…周报…".into(),
            created_at: "t".into(),
            author_user_id: None,
            author_name: None,
        };
        assert_eq!(
            serde_json::to_value(GroupSearch { results: vec![hit] }).unwrap(),
            json!({"results": [{"messageId": "m", "snippet": "…周报…", "createdAt": "t"}]})
        );
    }
}
