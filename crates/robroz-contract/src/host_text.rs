//! Chinese text that `robroz-host` sends to the client: HTTP `error`
//! bodies, card `progress` / `failure` / `reason` / `summary`, settings test
//! errors and Watchdog hints. The host writes these exact strings, so a
//! client can map them to another language by constant instead of by copied
//! text. Text sent only to the model (prompts, tool results, tool
//! descriptions) and log lines are not here.
//!
//! A `*_PREFIX` constant is followed by runtime text (an error, a name, a
//! number); a `*_SUFFIX` constant ends such a line. The functions below build
//! the few lines that mix fixed text with a number or a name.
//!
//! [`HOST_UI_STRINGS`] lists every constant in this module plus the older
//! host-written constants at the crate root, so a client can check that it
//! maps all of them.

/// Any group route: the group id does not exist (404).
pub const GROUP_NOT_FOUND: &str = "群不存在";
/// The bro named in the request is not a member of the group.
pub const BOT_NOT_IN_GROUP: &str = "这个 Bot 不在群里";
/// Required text is empty.
pub const NEED_TEXT: &str = "请填写内容";
/// Required name is empty.
pub const NEED_NAME: &str = "请填写名称";
/// Skill title is empty.
pub const NEED_TITLE: &str = "请填写标题";
/// Skill instruction is empty.
pub const NEED_INSTRUCTION: &str = "请填写说明";
/// Required URL is empty (model, search, MCP settings).
pub const NEED_URL: &str = "请填写地址";
/// Model name is empty on the model settings page.
pub const NEED_MODEL_NAME: &str = "请填写模型名";
/// MCP stdio command is empty.
pub const NEED_COMMAND: &str = "请填写启动命令";
/// MCP secret name is empty.
pub const NEED_SECRET_NAME: &str = "请填写密钥名称";
/// A new MCP secret has no value.
pub const NEED_SECRET_VALUE: &str = "新增密钥时请填写值";
/// Allow rule text is empty.
pub const NEED_RULE: &str = "请填写规则";
/// Approval summary is empty.
pub const NEED_ACTION: &str = "请填写要做什么";
/// SearXNG chosen without a URL.
pub const NEED_SEARXNG_URL: &str = "请填写 SearXNG 地址";
/// Request body does not match the expected shape (422/400).
pub const BAD_REQUEST_FORMAT: &str = "请求格式不正确";
/// Settings request body is not JSON.
pub const BODY_NOT_JSON: &str = "请求体不是 JSON";
/// Settings request body has the wrong fields.
pub const BODY_INVALID: &str = "请求体无效";
/// PATCH with no fields.
pub const NOTHING_TO_CHANGE: &str = "没有要改的内容";
/// Any SQLite failure (500).
pub const DATABASE_ERROR: &str = "数据库错误";
/// Approval or confirm card was already answered (409).
pub const ALREADY_ANSWERED: &str = "已经答过了";
/// `since` query is not an RFC 3339 time.
pub const SINCE_INVALID: &str = "since 不是合法时间";
/// `readAt` is not an RFC 3339 time.
pub const READ_AT_INVALID: &str = "readAt 不是合法时间";
/// URL must start with http:// or https://.
pub const URL_NEEDS_HTTP: &str = "地址要以 http:// 或 https:// 开头";
/// A key or secret value is empty.
pub const KEY_EMPTY: &str = "密钥不能为空";
/// Model or search key contains whitespace.
pub const KEY_HAS_WHITESPACE: &str = "密钥里不能有空白";
/// MCP server args are invalid.
pub const ARGS_INVALID: &str = "参数无效";
/// Memory or skill scope is invalid.
pub const BAD_SCOPE: &str = "scope 只能是 user 或 bot:<botId>";
/// Memory or skill status filter is invalid.
pub const BAD_STATUS: &str = "status 只能是 pending、kept 或 rejected";
/// Memory kind is invalid.
pub const BAD_MEMORY_KIND: &str = "kind 只能是 profile、habit 或 project";
/// Message kind is invalid.
pub const BAD_MESSAGE_KIND: &str = "kind 只能是 text、image 或 file";
/// Skill slug is invalid.
pub const BAD_SLUG: &str = "slug 只能是小写字母、数字和连字符";
/// Schedule cron expression is invalid.
pub const BAD_CRON: &str = "cron 格式不对";
/// MCP transport is invalid.
pub const BAD_TRANSPORT: &str = "transport 只能是 stdio 或 http";
/// Search provider is invalid.
pub const BAD_SEARCH_PROVIDER: &str = "provider 只能是 brave 或 searxng";
/// Missing or wrong bearer token (401).
pub const TOKEN_REJECTED: &str = "缺少或错误的令牌";
/// Bro name is too long.
pub const NAME_TOO_LONG: &str = "名字太长";
/// Bro avatar `shape` or `color` is out of range.
pub const INVALID_AVATAR: &str = "头像的形状或颜色超出范围";
/// MCP server name is over 80 characters.
pub const NAME_MAX_80: &str = "名称最多 80 个字";
/// MCP server name has a line break.
pub const NAME_ONE_LINE: &str = "名称只能一行";
/// MCP server name already used.
pub const NAME_TAKEN: &str = "这个名字已经有了";
/// Skill title is over 80 characters.
pub const TITLE_TOO_LONG: &str = "标题太长，最多 80 个字";
/// Skill instruction is over 8000 characters.
pub const INSTRUCTION_TOO_LONG: &str = "说明太长，最多 8000 个字";
/// Memory text is over 2000 characters.
pub const MEMORY_TOO_LONG: &str = "内容太长，最多 2000 个字";
/// Approval summary is not one line of at most 200 characters.
pub const SUMMARY_ONE_LINE: &str = "说明只能一行，最多 200 字";
/// Allow rule is not one line of at most 200 characters.
pub const RULE_ONE_LINE: &str = "规则只能一行，最多 200 字";
/// MCP stdio command is invalid.
pub const COMMAND_INVALID: &str = "启动命令无效";
/// MCP secret name is over 64 characters.
pub const SECRET_NAME_MAX: &str = "密钥名称最多 64 个字";
/// MCP secret name has whitespace.
pub const SECRET_NAME_WHITESPACE: &str = "密钥名称里不能有空白";
/// Skill slug already used.
pub const SLUG_TAKEN: &str = "这个 slug 已经有了";
/// Bro id does not exist.
pub const BRO_NOT_FOUND: &str = "bro 不存在";
/// Memory scope names a bro that does not exist.
pub const UNKNOWN_BRO: &str = "这个 bro 不存在";
/// Task id does not exist.
pub const TASK_NOT_FOUND: &str = "任务不存在";
/// Approval card id does not exist.
pub const APPROVAL_NOT_FOUND: &str = "审批卡不存在";
/// Approval card disappeared while it was being created (500).
pub const APPROVAL_VANISHED: &str = "审批卡不见了";
/// Confirm card id does not exist.
pub const CONFIRM_CARD_NOT_FOUND: &str = "确认卡不存在";
/// Choice card id does not exist.
pub const CHOICE_CARD_NOT_FOUND: &str = "选项卡不存在";
/// Memory id does not exist.
pub const MEMORY_NOT_FOUND: &str = "记忆不存在";
/// Skill id does not exist.
pub const SKILL_NOT_FOUND: &str = "技能不存在";
/// Allow rule id does not exist.
pub const RULE_NOT_FOUND: &str = "规则不存在";
/// Schedule id does not exist.
pub const ROUTINE_NOT_FOUND: &str = "例行任务不存在";
/// MCP server id does not exist.
pub const MCP_SERVER_NOT_FOUND: &str = "MCP 服务器不存在";
/// The MCP server is not enabled in this group.
pub const MCP_NOT_ENABLED_IN_GROUP: &str = "这个群还没启用这台服务器";
/// Approving a command on a non-stdio MCP server.
pub const MCP_STDIO_ONLY_APPROVE: &str = "只有 stdio 服务器需要批准启动命令";
/// 403: the caller is not this bro's owner (rename, add to a group, open
/// its DM, or answer its card). Only the owner may do this.
/// A DM already has its one bro.
pub const DM_ONE_BRO: &str = "私聊里只能有一个 bro";
/// 403: only the group's creator or an admin may rename or delete it.
pub const NOT_GROUP_CREATOR: &str = "只有建群的人或管理员能改这个群";
/// 403: the caller has no part in this group.
pub const NOT_GROUP_MEMBER: &str = "你不在这个群里";
/// 403: a DM cannot be renamed or deleted (its name follows the bro).
pub const GROUP_IS_DM: &str = "私聊不能改名或删除";
/// Approval action is not supported.
pub const ACTION_NOT_SUPPORTED: &str = "不支持这个动作";
/// Only the host may raise this approval action.
pub const ACTION_HOST_ONLY: &str = "这个动作只能由宿主机发出";
/// Approving an MCP card failed to write the resume marker (500).
pub const APPROVE_CONTINUE_MARKER_FAILED: &str = "批准失败，无法写入继续标记";
/// Approving a takeover card failed to write the marker (500).
pub const APPROVE_TAKEOVER_MARKER_FAILED: &str = "批准失败，无法写入接管标记";
/// Resume found no blocked intent.
pub const RESUME_NO_INTENT: &str = "恢复失败，找不到卡住记录";
/// Resume failed to write the takeover marker.
pub const RESUME_MARKER_FAILED: &str = "恢复失败，无法写入接管标记";
/// Resume while the approval card is still open.
pub const RESUME_NEEDS_APPROVAL: &str = "请先回答审批卡";
/// The task cannot be resumed this time (409).
pub const CANT_RESUME: &str = "这次不能恢复";
/// Choice card needs 2 to 4 options.
pub const CHOICE_OPTION_COUNT: &str = "选项需要 2 到 4 个";
/// At most one recommended option.
pub const CHOICE_ONE_RECOMMENDED: &str = "推荐项最多一个";
/// An option is empty.
pub const CHOICE_OPTION_EMPTY: &str = "选项不能为空";
/// Duplicate options or picks.
pub const CHOICE_DUPLICATE: &str = "选项重复";
/// Multi-pick card answered with nothing.
pub const CHOICE_PICK_AT_LEAST_ONE: &str = "至少选一个";
/// Single-pick card answered with more than one.
pub const CHOICE_PICK_ONLY_ONE: &str = "只能选一个";
/// Pick is not one of the options.
pub const CHOICE_NO_SUCH_OPTION: &str = "没有这个选项";
/// Choice card already answered (409).
pub const CHOICE_ALREADY_PICKED: &str = "已经选过了";
/// No model set up: task result text and `needsSetup` hint.
pub const MODEL_NOT_CONFIGURED: &str = "还没设置模型";
/// `ROBROZ_MODEL_KEY` (or its file) has whitespace.
pub const MODEL_KEY_HAS_WHITESPACE: &str = "模型密钥里不能有空白";
/// Both model key env vars are set.
pub const MODEL_ENV_KEY_CONFLICT: &str = "ROBROZ_MODEL_KEY 和 ROBROZ_MODEL_KEY_FILE 只能设一个";
/// The model key file is empty.
pub const MODEL_ENV_KEY_FILE_EMPTY: &str = "ROBROZ_MODEL_KEY_FILE 是空的";
/// Prefix; followed by the OS error.
pub const MODEL_ENV_KEY_FILE_UNREADABLE_PREFIX: &str = "ROBROZ_MODEL_KEY_FILE 读不出来：";
/// Web search not set up: test result and task text.
pub const SEARCH_NOT_CONFIGURED: &str = "还没设置联网搜索";
/// Prefix on model and search test errors; followed by the redacted error.
pub const CANT_CONNECT_PREFIX: &str = "连不上：";
/// Prefix on MCP tool-list errors; followed by the error.
pub const CONNECTION_LOST_PREFIX: &str = "连接中断：";
/// Prefix on a search provider URL error; followed by the parse error.
pub const URL_INVALID_PREFIX: &str = "地址无效：";
/// Prefix; followed by the HTTP status code.
pub const SEARCH_SERVICE_STATUS_PREFIX: &str = "搜索服务返回 ";
/// Search provider body is not JSON.
pub const SEARCH_NOT_JSON: &str = "搜索返回的不是 JSON";
/// Brave body has no `web.results`.
pub const SEARCH_NO_WEB_RESULTS: &str = "搜索返回里没有 web.results";
/// SearXNG body has no `results`.
pub const SEARCH_NO_RESULTS: &str = "搜索返回里没有 results";
/// Prefix; followed by the provider name.
pub const SEARCH_PROVIDER_UNSUPPORTED_PREFIX: &str = "不支持的搜索服务：";
/// Task progress: just started.
pub const PROGRESS_STARTED: &str = "开始干活";
/// Task progress: waiting for the model.
pub const PROGRESS_THINKING: &str = "在想怎么回";
/// Task progress: running in the sandbox.
pub const PROGRESS_IN_SANDBOX: &str = "在沙箱里跑";
/// Progress prefix; followed by a tool's display name.
pub const PROGRESS_USING_PREFIX: &str = "在用 ";
/// Watchdog hint prefix; followed by minutes and WATCHDOG_STEP_TIMEOUT_SUFFIX.
pub const WATCHDOG_STEP_TIMEOUT_PREFIX: &str = "模型和工具还没回，已等 ";
/// Watchdog step-timeout hint suffix.
pub const WATCHDOG_STEP_TIMEOUT_SUFFIX: &str = " 分钟";
/// Watchdog stall hint suffix; preceded by minutes.
pub const WATCHDOG_STALL_SUFFIX: &str = " 分钟没有新进度";
/// Delivery `summary` when the task wrote its result.
pub const DELIVERY_WRITTEN: &str = "已写入结果";
/// Delivery `summary` when a blocked MCP call was denied and never ran.
pub const DELIVERY_NOT_RUN: &str = "未执行";
/// Approval summary prefix for an outbound write; followed by the task's first line.
pub const GATE_PREFIX: &str = "要对外写入：";
/// Used for the task line when the prompt has no plain line.
pub const GATE_FALLBACK_LINE: &str = "这个任务";
/// Task `failure` prefix after a denial; followed by the task line or tool.
pub const DENIED_PREFIX: &str = "你拒绝了：";
/// Tool name in a denial when the card has none.
pub const DENIED_MCP_FALLBACK: &str = "MCP 工具";
/// MCP approval summary prefix; followed by `server.tool`.
pub const MCP_CALL_SUMMARY_PREFIX: &str = "调用 ";
/// Between `server.tool` and the compact JSON args in an MCP approval summary.
pub const MCP_CALL_ARGS_SEPARATOR: &str = "：";
/// Task `failure` prefix; followed by the model error.
pub const MODEL_NO_REPLY_PREFIX: &str = "模型没回：";
/// Task `failure` text while a tool call waits; followed by the tool's display name.
pub const WAITING_APPROVAL_PREFIX: &str = "等待批准：";
/// Task `failure`: the tool loop hit its round limit.
pub const TOOL_ROUND_LIMIT: &str = "工具调用超过 8 轮";
/// Task `failure` when the runner reports over quota with no detail.
pub const OUTBOUND_OVER_QUOTA: &str = "出网超额";
/// Task `failure` prefix; followed by the error.
pub const SAVE_APPROVAL_STATE_FAILED_PREFIX: &str = "无法保存审批状态：";
/// Task `failure` prefix; followed by the error.
pub const RESUME_APPROVED_CALL_FAILED_PREFIX: &str = "无法继续已批准的调用：";
/// Nested error after SAVE_APPROVAL_STATE_FAILED_PREFIX.
pub const PENDING_WRITE_FAILED_PREFIX: &str = "无法写入审批状态：";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_NOT_FOUND_PREFIX: &str = "找不到待批准的调用：";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_CORRUPT_PREFIX: &str = "待批准状态损坏：";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_NOT_APPROVED: &str = "调用还没有被批准";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_MISSING_MESSAGES: &str = "待批准状态缺少 messages";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_MISSING_REMAINING: &str = "待批准状态缺少 remaining";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_CALL_MISSING_ID: &str = "tool call 缺少 id";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_CALL_MISSING_NAME: &str = "tool call 缺少 name";
/// Nested error after RESUME_APPROVED_CALL_FAILED_PREFIX.
pub const PENDING_MESSAGE_MISSING_ROLE: &str = "message 缺少 role";
/// Nested error prefix; followed by the role.
pub const PENDING_UNKNOWN_ROLE_PREFIX: &str = "未知的 message role：";
/// Approval `reason`: auto-review has no model.
pub const REVIEW_NO_MODEL_REASON: &str = "还没设置模型，没法自动审核";
/// Approval `reason`: auto-review failed twice.
pub const REVIEW_FAILED_REASON: &str = "自动审核没有成功，需要你批准";
/// Approval `reason`: no allow rule matched (also the fallback for an empty block reason).
pub const REVIEW_NO_MATCH_REASON: &str = "没有放行规则允许这个操作";
/// Fallback `reason` when auto-review allows with no reason.
pub const REVIEW_ALLOW_FALLBACK: &str = "符合放行规则";

/// Watchdog hint for a step past its timeout: 「模型和工具还没回，已等 N 分钟」.
pub fn step_timeout_hint(minutes: u64) -> String {
    format!("{WATCHDOG_STEP_TIMEOUT_PREFIX}{minutes}{WATCHDOG_STEP_TIMEOUT_SUFFIX}")
}

/// Watchdog hint for a live container with no new progress: 「N 分钟没有新进度」.
pub fn stall_hint(minutes: u64) -> String {
    format!("{minutes}{WATCHDOG_STALL_SUFFIX}")
}

/// Task progress while a tool runs: 「在用 {name}」.
pub fn using_tool(name: &str) -> String {
    format!("{PROGRESS_USING_PREFIX}{name}")
}

/// Task progress while a task split by the Router runs step `n` of `m`:
/// 「{n}/{m} 步」. It is the whole `progress` text, never combined with other
/// text, and only set while the task is `running` with its container up.
/// Clients can spot it with [`parse_router_step_progress`].
pub fn router_step_progress(n: usize, m: usize) -> String {
    format!("{n}/{m} 步")
}

/// `Some((n, m))` when `progress` is exactly [`router_step_progress`]`(n, m)`.
pub fn parse_router_step_progress(progress: &str) -> Option<(usize, usize)> {
    let (n, m) = progress.strip_suffix(" 步")?.split_once('/')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(n) || !digits(m) {
        return None;
    }
    let (n, m) = (n.parse().ok()?, m.parse().ok()?);
    (n >= 1 && n <= m).then_some((n, m))
}

/// Every host-written client string, as `(constant name, text)`. Includes
/// the older constants at the crate root that the host also writes.
/// Lines built at run time ([`step_timeout_hint`], [`stall_hint`],
/// [`using_tool`], [`router_step_progress`]) are functions, not listed here.
/// Tool result when a tool call returns an error, before the error text.
/// The host hands it to the model as the tool's result
/// (`host/src/tools.rs`); it is listed here because a reply can carry it
/// onto a card.
pub const TOOL_CALL_FAILED_PREFIX: &str = "调用失败：";
/// Tool result when an approved call still hits the approval gate
/// (`host/src/tools.rs`).
pub const TOOL_STILL_NEEDS_APPROVAL: &str = "调用失败：这个工具还需要批准";

pub const HOST_UI_STRINGS: &[(&str, &str)] = &[
    ("GROUP_NOT_FOUND", GROUP_NOT_FOUND),
    ("BOT_NOT_IN_GROUP", BOT_NOT_IN_GROUP),
    ("NEED_TEXT", NEED_TEXT),
    ("NEED_NAME", NEED_NAME),
    ("NEED_TITLE", NEED_TITLE),
    ("NEED_INSTRUCTION", NEED_INSTRUCTION),
    ("NEED_URL", NEED_URL),
    ("NEED_MODEL_NAME", NEED_MODEL_NAME),
    ("NEED_COMMAND", NEED_COMMAND),
    ("NEED_SECRET_NAME", NEED_SECRET_NAME),
    ("NEED_SECRET_VALUE", NEED_SECRET_VALUE),
    ("NEED_RULE", NEED_RULE),
    ("NEED_ACTION", NEED_ACTION),
    ("NEED_SEARXNG_URL", NEED_SEARXNG_URL),
    ("BAD_REQUEST_FORMAT", BAD_REQUEST_FORMAT),
    ("BODY_NOT_JSON", BODY_NOT_JSON),
    ("BODY_INVALID", BODY_INVALID),
    ("NOTHING_TO_CHANGE", NOTHING_TO_CHANGE),
    ("DATABASE_ERROR", DATABASE_ERROR),
    ("ALREADY_ANSWERED", ALREADY_ANSWERED),
    ("SINCE_INVALID", SINCE_INVALID),
    ("READ_AT_INVALID", READ_AT_INVALID),
    ("URL_NEEDS_HTTP", URL_NEEDS_HTTP),
    ("KEY_EMPTY", KEY_EMPTY),
    ("KEY_HAS_WHITESPACE", KEY_HAS_WHITESPACE),
    ("ARGS_INVALID", ARGS_INVALID),
    ("BAD_SCOPE", BAD_SCOPE),
    ("BAD_STATUS", BAD_STATUS),
    ("BAD_MEMORY_KIND", BAD_MEMORY_KIND),
    ("BAD_MESSAGE_KIND", BAD_MESSAGE_KIND),
    ("BAD_SLUG", BAD_SLUG),
    ("BAD_CRON", BAD_CRON),
    ("BAD_TRANSPORT", BAD_TRANSPORT),
    ("BAD_SEARCH_PROVIDER", BAD_SEARCH_PROVIDER),
    ("TOKEN_REJECTED", TOKEN_REJECTED),
    ("NAME_TOO_LONG", NAME_TOO_LONG),
    ("INVALID_AVATAR", INVALID_AVATAR),
    ("NAME_MAX_80", NAME_MAX_80),
    ("NAME_ONE_LINE", NAME_ONE_LINE),
    ("NAME_TAKEN", NAME_TAKEN),
    ("TITLE_TOO_LONG", TITLE_TOO_LONG),
    ("INSTRUCTION_TOO_LONG", INSTRUCTION_TOO_LONG),
    ("MEMORY_TOO_LONG", MEMORY_TOO_LONG),
    ("SUMMARY_ONE_LINE", SUMMARY_ONE_LINE),
    ("RULE_ONE_LINE", RULE_ONE_LINE),
    ("COMMAND_INVALID", COMMAND_INVALID),
    ("SECRET_NAME_MAX", SECRET_NAME_MAX),
    ("SECRET_NAME_WHITESPACE", SECRET_NAME_WHITESPACE),
    ("SLUG_TAKEN", SLUG_TAKEN),
    ("BRO_NOT_FOUND", BRO_NOT_FOUND),
    ("UNKNOWN_BRO", UNKNOWN_BRO),
    ("TASK_NOT_FOUND", TASK_NOT_FOUND),
    ("APPROVAL_NOT_FOUND", APPROVAL_NOT_FOUND),
    ("APPROVAL_VANISHED", APPROVAL_VANISHED),
    ("CONFIRM_CARD_NOT_FOUND", CONFIRM_CARD_NOT_FOUND),
    ("CHOICE_CARD_NOT_FOUND", CHOICE_CARD_NOT_FOUND),
    ("MEMORY_NOT_FOUND", MEMORY_NOT_FOUND),
    ("SKILL_NOT_FOUND", SKILL_NOT_FOUND),
    ("RULE_NOT_FOUND", RULE_NOT_FOUND),
    ("ROUTINE_NOT_FOUND", ROUTINE_NOT_FOUND),
    ("MCP_SERVER_NOT_FOUND", MCP_SERVER_NOT_FOUND),
    ("MCP_NOT_ENABLED_IN_GROUP", MCP_NOT_ENABLED_IN_GROUP),
    ("MCP_STDIO_ONLY_APPROVE", MCP_STDIO_ONLY_APPROVE),
    ("DM_ONE_BRO", DM_ONE_BRO),
    ("NOT_GROUP_CREATOR", NOT_GROUP_CREATOR),
    ("NOT_GROUP_MEMBER", NOT_GROUP_MEMBER),
    ("GROUP_IS_DM", GROUP_IS_DM),
    ("ACTION_NOT_SUPPORTED", ACTION_NOT_SUPPORTED),
    ("ACTION_HOST_ONLY", ACTION_HOST_ONLY),
    (
        "APPROVE_CONTINUE_MARKER_FAILED",
        APPROVE_CONTINUE_MARKER_FAILED,
    ),
    (
        "APPROVE_TAKEOVER_MARKER_FAILED",
        APPROVE_TAKEOVER_MARKER_FAILED,
    ),
    ("RESUME_NO_INTENT", RESUME_NO_INTENT),
    ("RESUME_MARKER_FAILED", RESUME_MARKER_FAILED),
    ("RESUME_NEEDS_APPROVAL", RESUME_NEEDS_APPROVAL),
    ("CANT_RESUME", CANT_RESUME),
    ("CHOICE_OPTION_COUNT", CHOICE_OPTION_COUNT),
    ("CHOICE_ONE_RECOMMENDED", CHOICE_ONE_RECOMMENDED),
    ("CHOICE_OPTION_EMPTY", CHOICE_OPTION_EMPTY),
    ("CHOICE_DUPLICATE", CHOICE_DUPLICATE),
    ("CHOICE_PICK_AT_LEAST_ONE", CHOICE_PICK_AT_LEAST_ONE),
    ("CHOICE_PICK_ONLY_ONE", CHOICE_PICK_ONLY_ONE),
    ("CHOICE_NO_SUCH_OPTION", CHOICE_NO_SUCH_OPTION),
    ("CHOICE_ALREADY_PICKED", CHOICE_ALREADY_PICKED),
    ("MODEL_NOT_CONFIGURED", MODEL_NOT_CONFIGURED),
    ("MODEL_KEY_HAS_WHITESPACE", MODEL_KEY_HAS_WHITESPACE),
    ("MODEL_ENV_KEY_CONFLICT", MODEL_ENV_KEY_CONFLICT),
    ("MODEL_ENV_KEY_FILE_EMPTY", MODEL_ENV_KEY_FILE_EMPTY),
    (
        "MODEL_ENV_KEY_FILE_UNREADABLE_PREFIX",
        MODEL_ENV_KEY_FILE_UNREADABLE_PREFIX,
    ),
    ("SEARCH_NOT_CONFIGURED", SEARCH_NOT_CONFIGURED),
    ("CANT_CONNECT_PREFIX", CANT_CONNECT_PREFIX),
    ("CONNECTION_LOST_PREFIX", CONNECTION_LOST_PREFIX),
    ("URL_INVALID_PREFIX", URL_INVALID_PREFIX),
    ("SEARCH_SERVICE_STATUS_PREFIX", SEARCH_SERVICE_STATUS_PREFIX),
    ("SEARCH_NOT_JSON", SEARCH_NOT_JSON),
    ("SEARCH_NO_WEB_RESULTS", SEARCH_NO_WEB_RESULTS),
    ("SEARCH_NO_RESULTS", SEARCH_NO_RESULTS),
    (
        "SEARCH_PROVIDER_UNSUPPORTED_PREFIX",
        SEARCH_PROVIDER_UNSUPPORTED_PREFIX,
    ),
    ("PROGRESS_STARTED", PROGRESS_STARTED),
    ("PROGRESS_THINKING", PROGRESS_THINKING),
    ("PROGRESS_IN_SANDBOX", PROGRESS_IN_SANDBOX),
    ("PROGRESS_USING_PREFIX", PROGRESS_USING_PREFIX),
    ("TOOL_CALL_FAILED_PREFIX", TOOL_CALL_FAILED_PREFIX),
    ("TOOL_STILL_NEEDS_APPROVAL", TOOL_STILL_NEEDS_APPROVAL),
    ("WATCHDOG_STEP_TIMEOUT_PREFIX", WATCHDOG_STEP_TIMEOUT_PREFIX),
    ("WATCHDOG_STEP_TIMEOUT_SUFFIX", WATCHDOG_STEP_TIMEOUT_SUFFIX),
    ("WATCHDOG_STALL_SUFFIX", WATCHDOG_STALL_SUFFIX),
    ("DELIVERY_WRITTEN", DELIVERY_WRITTEN),
    ("DELIVERY_NOT_RUN", DELIVERY_NOT_RUN),
    ("GATE_PREFIX", GATE_PREFIX),
    ("GATE_FALLBACK_LINE", GATE_FALLBACK_LINE),
    ("DENIED_PREFIX", DENIED_PREFIX),
    ("DENIED_MCP_FALLBACK", DENIED_MCP_FALLBACK),
    ("MCP_CALL_SUMMARY_PREFIX", MCP_CALL_SUMMARY_PREFIX),
    ("MCP_CALL_ARGS_SEPARATOR", MCP_CALL_ARGS_SEPARATOR),
    ("MODEL_NO_REPLY_PREFIX", MODEL_NO_REPLY_PREFIX),
    ("WAITING_APPROVAL_PREFIX", WAITING_APPROVAL_PREFIX),
    ("TOOL_ROUND_LIMIT", TOOL_ROUND_LIMIT),
    ("OUTBOUND_OVER_QUOTA", OUTBOUND_OVER_QUOTA),
    (
        "SAVE_APPROVAL_STATE_FAILED_PREFIX",
        SAVE_APPROVAL_STATE_FAILED_PREFIX,
    ),
    (
        "RESUME_APPROVED_CALL_FAILED_PREFIX",
        RESUME_APPROVED_CALL_FAILED_PREFIX,
    ),
    ("PENDING_WRITE_FAILED_PREFIX", PENDING_WRITE_FAILED_PREFIX),
    ("PENDING_NOT_FOUND_PREFIX", PENDING_NOT_FOUND_PREFIX),
    ("PENDING_CORRUPT_PREFIX", PENDING_CORRUPT_PREFIX),
    ("PENDING_NOT_APPROVED", PENDING_NOT_APPROVED),
    ("PENDING_MISSING_MESSAGES", PENDING_MISSING_MESSAGES),
    ("PENDING_MISSING_REMAINING", PENDING_MISSING_REMAINING),
    ("PENDING_CALL_MISSING_ID", PENDING_CALL_MISSING_ID),
    ("PENDING_CALL_MISSING_NAME", PENDING_CALL_MISSING_NAME),
    ("PENDING_MESSAGE_MISSING_ROLE", PENDING_MESSAGE_MISSING_ROLE),
    ("PENDING_UNKNOWN_ROLE_PREFIX", PENDING_UNKNOWN_ROLE_PREFIX),
    ("REVIEW_NO_MODEL_REASON", REVIEW_NO_MODEL_REASON),
    ("REVIEW_FAILED_REASON", REVIEW_FAILED_REASON),
    ("REVIEW_NO_MATCH_REASON", REVIEW_NO_MATCH_REASON),
    ("REVIEW_ALLOW_FALLBACK", REVIEW_ALLOW_FALLBACK),
    ("APPROVAL_EXPIRED_REASON", crate::APPROVAL_EXPIRED_REASON),
    (
        "SETTINGS_APPROVAL_BOT_NAME",
        crate::SETTINGS_APPROVAL_BOT_NAME,
    ),
    (
        "APPROVAL_SENT_TO_OWNER_DM_NOTICE",
        crate::APPROVAL_SENT_TO_OWNER_DM_NOTICE,
    ),
    ("MODEL_TEST_BAD_KEY", crate::MODEL_TEST_BAD_KEY),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `pub const` in this file is in [`HOST_UI_STRINGS`], once, under
    /// its own name, and no two constants share a text.
    #[test]
    fn host_ui_strings_lists_every_constant() {
        let src = include_str!("host_text.rs");
        let declared: Vec<&str> = src
            .lines()
            .filter_map(|l| l.strip_prefix("pub const "))
            .filter_map(|l| l.split(':').next())
            .filter(|n| *n != "HOST_UI_STRINGS")
            .collect();
        let listed: Vec<&str> = HOST_UI_STRINGS.iter().map(|(n, _)| *n).collect();
        for name in &declared {
            assert_eq!(
                listed.iter().filter(|n| *n == name).count(),
                1,
                "{name} must be listed exactly once in HOST_UI_STRINGS"
            );
        }
        let crate_root = [
            "APPROVAL_EXPIRED_REASON",
            "SETTINGS_APPROVAL_BOT_NAME",
            "APPROVAL_SENT_TO_OWNER_DM_NOTICE",
            "MODEL_TEST_BAD_KEY",
        ];
        assert_eq!(listed.len(), declared.len() + crate_root.len());
        let mut texts: Vec<&str> = HOST_UI_STRINGS.iter().map(|(_, t)| *t).collect();
        texts.sort_unstable();
        let before = texts.len();
        texts.dedup();
        assert_eq!(texts.len(), before, "two constants share a text");
        assert!(HOST_UI_STRINGS.iter().all(|(_, t)| !t.is_empty()));
    }

    #[test]
    fn built_lines_keep_the_wire_text() {
        assert_eq!(step_timeout_hint(10), "模型和工具还没回，已等 10 分钟");
        assert_eq!(stall_hint(15), "15 分钟没有新进度");
        assert_eq!(using_tool("web_search"), "在用 web_search");
        assert_eq!(router_step_progress(2, 3), "2/3 步");
    }

    #[test]
    fn router_step_progress_round_trips() {
        for (n, m) in [(1, 2), (2, 3), (3, 3)] {
            assert_eq!(
                parse_router_step_progress(&router_step_progress(n, m)),
                Some((n, m))
            );
        }
        for text in [
            "2/3步",
            "x/3 步",
            "0/3 步",
            "4/3 步",
            "/3 步",
            "在用 2/3 步",
            "2/3 步 ",
        ] {
            assert_eq!(parse_router_step_progress(text), None, "{text}");
        }
    }
}
