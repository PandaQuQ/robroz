//! Stable machine codes for the task card states and results the host used
//! to send only as Chinese text. A client renders its own copy from the code
//! through its i18n and never shows the legacy text next to it.
//!
//! The legacy text fields (`label`, `failure`, `briefing`) are unchanged and
//! still sent. Every code field is additive and optional: a host from before
//! this change omits them and a client that does not know them ignores them.
//!
//! All codes serialize as snake_case strings.

use serde::{Deserialize, Serialize};

use crate::host_text as ht;
use crate::legacy::NO_RESULT;
use crate::{TaskState, APPROVAL_EXPIRED_REASON};

/// `statusCode` of a status card (the code form of `label`).
///
/// `no_result` is a `running` task whose container is gone (a dead run in old
/// data). New dead runs end as `done` with [`FailureCode::NoResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardStatusCode {
    Queued,
    Running,
    Blocked,
    Done,
    NoResult,
}

impl CardStatusCode {
    /// The code for a task row, with the same rule as the legacy `label`.
    pub fn of(state: TaskState, container_up: bool) -> Self {
        match state {
            TaskState::Queued => Self::Queued,
            TaskState::Running if container_up => Self::Running,
            TaskState::Running => Self::NoResult,
            TaskState::Blocked => Self::Blocked,
            TaskState::Done => Self::Done,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::NoResult => "no_result",
        }
    }
}

/// `badge` of a delivery card (the code form of the 「简报」 corner badge).
/// Present only when `briefing` is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryBadge {
    Briefing,
}

/// `failureCode` of a `done` card that did not get done. `failureDetail`
/// carries the raw reason without any prefix, so the client adds exactly one
/// prefix of its own (「没做成：…」 / "Couldn't finish: …").
///
/// `failureDetail` is [`FailureData::detail`]: only language-neutral raw
/// data (a status code, a tool name, `used/limit`, a method, raw upstream
/// error text or a reason token), never host-written copy. See `docs/api.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    NoResult,
    Denied,
    ApprovalExpired,
    ModelNoReply,
    ToolRoundLimit,
    OutboundOverQuota,
    OutboundNotGet,
    SaveApprovalStateFailed,
    ResumeApprovedCallFailed,
    WaitingApproval,
    ToolCallFailed,
    ToolNeedsApproval,
    Other,
}

impl FailureCode {
    /// Every code, for tests and clients that must map all of them.
    pub const ALL: [FailureCode; 13] = [
        Self::NoResult,
        Self::Denied,
        Self::ApprovalExpired,
        Self::ModelNoReply,
        Self::ToolRoundLimit,
        Self::OutboundOverQuota,
        Self::OutboundNotGet,
        Self::SaveApprovalStateFailed,
        Self::ResumeApprovedCallFailed,
        Self::WaitingApproval,
        Self::ToolCallFailed,
        Self::ToolNeedsApproval,
        Self::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoResult => "no_result",
            Self::Denied => "denied",
            Self::ApprovalExpired => "approval_expired",
            Self::ModelNoReply => "model_no_reply",
            Self::ToolRoundLimit => "tool_round_limit",
            Self::OutboundOverQuota => "outbound_over_quota",
            Self::OutboundNotGet => "outbound_not_get",
            Self::SaveApprovalStateFailed => "save_approval_state_failed",
            Self::ResumeApprovedCallFailed => "resume_approved_call_failed",
            Self::WaitingApproval => "waiting_approval",
            Self::ToolCallFailed => "tool_call_failed",
            Self::ToolNeedsApproval => "tool_needs_approval",
            Self::Other => "other",
        }
    }
}

const OVER_QUOTA_USED: &str = "已用 ";
const OVER_QUOTA_TIMES_LIMIT: &str = " 次，上限 ";
const NOT_GET_PREFIX: &str = "出网只允许 GET：用了 ";

/// Split the `failure` text the host stores into a code and the raw detail
/// without its prefix. Every text the host writes maps to a code; anything
/// else (a runner error, older data) is [`FailureCode::Other`] with the whole
/// text as the detail. The legacy `failure` text itself is not changed.
pub fn split_failure(failure: &str) -> (FailureCode, Option<String>) {
    let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let exact = [
        (NO_RESULT, FailureCode::NoResult),
        (APPROVAL_EXPIRED_REASON, FailureCode::ApprovalExpired),
        (ht::TOOL_ROUND_LIMIT, FailureCode::ToolRoundLimit),
        (
            ht::TOOL_STILL_NEEDS_APPROVAL,
            FailureCode::ToolNeedsApproval,
        ),
        (ht::OUTBOUND_OVER_QUOTA, FailureCode::OutboundOverQuota),
    ];
    for (text, code) in exact {
        if failure == text {
            return (code, None);
        }
    }
    // `TOOL_STILL_NEEDS_APPROVAL` starts with the call-failed prefix, so it
    // is matched above, before the prefixes below.
    let prefixed = [
        (ht::DENIED_PREFIX, FailureCode::Denied),
        (ht::MODEL_NO_REPLY_PREFIX, FailureCode::ModelNoReply),
        (
            ht::SAVE_APPROVAL_STATE_FAILED_PREFIX,
            FailureCode::SaveApprovalStateFailed,
        ),
        (
            ht::RESUME_APPROVED_CALL_FAILED_PREFIX,
            FailureCode::ResumeApprovedCallFailed,
        ),
        (ht::WAITING_APPROVAL_PREFIX, FailureCode::WaitingApproval),
        (ht::TOOL_CALL_FAILED_PREFIX, FailureCode::ToolCallFailed),
    ];
    for (prefix, code) in prefixed {
        if let Some(rest) = failure.strip_prefix(prefix) {
            return (code, some(rest));
        }
    }
    if let Some(method) = failure.strip_prefix(NOT_GET_PREFIX) {
        return (FailureCode::OutboundNotGet, some(method));
    }
    if let Some(rest) = failure
        .strip_prefix(ht::OUTBOUND_OVER_QUOTA)
        .and_then(|r| r.strip_prefix('：'))
    {
        let counts = rest
            .strip_prefix(OVER_QUOTA_USED)
            .and_then(|r| r.split_once(OVER_QUOTA_TIMES_LIMIT))
            .filter(|(used, limit)| {
                [used, limit]
                    .iter()
                    .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .map(|(used, limit)| format!("{used}/{limit}"));
        return (
            FailureCode::OutboundOverQuota,
            counts.or_else(|| some(rest)),
        );
    }
    (FailureCode::Other, some(failure))
}

/// The language-neutral parameters of a failure, stored next to its
/// [`FailureCode`] so nothing has to parse the legacy text again. Every field
/// is optional; which ones are set depends on the code (see
/// [`FailureData::detail`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureParams {
    /// HTTP status code the upstream service answered with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Outbound GETs the task made, for `outbound_over_quota`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
    /// The outbound GET limit, for `outbound_over_quota`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    /// HTTP method that was used, for `outbound_not_get`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Tool name (`server.tool` for an MCP tool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The person's own task line, for `denied`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// Raw error text from upstream or the OS, never host-written copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A stable snake_case token for a host-side reason with no raw error,
    /// for example `response_not_json` or `pending_corrupt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `other` only: the whole text that was stored, as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl FailureParams {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A failure as the host stores it: the code plus its parameters. Serialized
/// as JSON into the `failure_data` column next to the legacy `failure` text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureData {
    pub code: FailureCode,
    #[serde(default, skip_serializing_if = "FailureParams::is_empty")]
    pub params: FailureParams,
}

impl FailureData {
    pub fn new(code: FailureCode) -> Self {
        Self {
            code,
            params: FailureParams::default(),
        }
    }

    pub fn with(code: FailureCode, params: FailureParams) -> Self {
        Self { code, params }
    }

    /// The `failureDetail` sent to clients: only language-neutral raw data.
    /// The first set field wins, in this order: `status` (`"401"`),
    /// `used`/`limit` (`"3/2"`), `method`, `tool`, `line`, `error`, `reason`,
    /// `text`.
    pub fn detail(&self) -> Option<String> {
        let p = &self.params;
        let counts = match (p.used, p.limit) {
            (Some(used), Some(limit)) => Some(format!("{used}/{limit}")),
            _ => None,
        };
        p.status
            .map(|s| s.to_string())
            .or(counts)
            .or_else(|| p.method.clone())
            .or_else(|| p.tool.clone())
            .or_else(|| p.line.clone())
            .or_else(|| p.error.clone())
            .or_else(|| p.reason.clone())
            .or_else(|| p.text.clone())
            .filter(|d| !d.is_empty())
    }

    /// Read a stored legacy `failure` text once, for rows written before the
    /// typed column existed. New failures are built typed by the host and
    /// never go through this.
    pub fn from_legacy(failure: &str) -> Self {
        let (code, rest) = split_failure(failure);
        let mut params = FailureParams::default();
        match code {
            FailureCode::ModelNoReply => {
                if let Some(rest) = rest.as_deref() {
                    legacy_model_error(rest, &mut params);
                }
            }
            FailureCode::OutboundOverQuota => match rest.as_deref().and_then(|r| r.split_once('/'))
            {
                Some((used, limit))
                    if used.parse::<u64>().is_ok() && limit.parse::<u64>().is_ok() =>
                {
                    params.used = used.parse().ok();
                    params.limit = limit.parse().ok();
                }
                _ => params.text = rest,
            },
            FailureCode::OutboundNotGet => params.method = rest,
            FailureCode::WaitingApproval | FailureCode::ToolCallFailed => params.error = rest,
            FailureCode::Denied => match rest {
                Some(r) if looks_like_tool(&r) => params.tool = Some(r),
                r => params.line = r,
            },
            FailureCode::SaveApprovalStateFailed | FailureCode::ResumeApprovedCallFailed => {
                if let Some(rest) = rest.as_deref() {
                    legacy_pending_error(rest, &mut params);
                }
            }
            FailureCode::Other => params.text = rest,
            FailureCode::NoResult
            | FailureCode::ApprovalExpired
            | FailureCode::ToolRoundLimit
            | FailureCode::ToolNeedsApproval => {}
        }
        // `waiting_approval` carried the tool's display name.
        if code == FailureCode::WaitingApproval {
            params.tool = params.error.take();
        }
        Self { code, params }
    }
}

/// `server.tool` or a plain tool identifier: ASCII, no spaces, has a dot.
fn looks_like_tool(s: &str) -> bool {
    s.contains('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The robroz-model error texts an older host stored after 「模型没回：」.
const LEGACY_MODEL_ERRORS: [(&str, &str); 9] = [
    ("此模型不支持工具调用", "tools_unsupported"),
    ("模型在没有工具时返回了工具调用", "unexpected_tool_calls"),
    ("模型返回的不是 JSON", "response_not_json"),
    (
        "模型返回里没有 choices[0].message",
        "response_missing_message",
    ),
    (
        "模型返回里没有 choices[0].message.content",
        "response_missing_content",
    ),
    ("模型返回了空内容", "response_empty_content"),
    ("模型返回的 tool_calls 不是数组", "tool_calls_not_array"),
    ("工具调用里没有 name", "tool_call_missing_name"),
    ("模型没有更多返回", "no_more_turns"),
];

fn legacy_model_error(rest: &str, params: &mut FailureParams) {
    if let Some(status) = rest
        .strip_prefix("模型服务返回 ")
        .and_then(|s| s.trim().parse::<u16>().ok())
    {
        params.status = Some(status);
        return;
    }
    for (text, token) in LEGACY_MODEL_ERRORS {
        if rest == text {
            params.reason = Some(token.into());
            return;
        }
    }
    for (prefix, token) in [
        ("连不上模型服务：", "connect_failed"),
        ("读模型返回失败：", "read_failed"),
        ("不支持的 tool_call 类型：", "unsupported_tool_call_type"),
    ] {
        if let Some(raw) = rest.strip_prefix(prefix) {
            params.reason = Some(token.into());
            params.error = (!raw.is_empty()).then(|| raw.to_string());
            return;
        }
    }
    params.error = Some(rest.to_string());
}

/// Reason tokens for the nested approval-state errors
/// ([`ht::PENDING_WRITE_FAILED_PREFIX`] and friends).
pub const PENDING_REASONS: [(&str, &str); 10] = [
    (ht::PENDING_WRITE_FAILED_PREFIX, "pending_write_failed"),
    (ht::PENDING_NOT_FOUND_PREFIX, "pending_not_found"),
    (ht::PENDING_CORRUPT_PREFIX, "pending_corrupt"),
    (ht::PENDING_NOT_APPROVED, "pending_not_approved"),
    (ht::PENDING_MISSING_MESSAGES, "pending_missing_messages"),
    (ht::PENDING_MISSING_REMAINING, "pending_missing_remaining"),
    (ht::PENDING_CALL_MISSING_ID, "pending_call_missing_id"),
    (ht::PENDING_CALL_MISSING_NAME, "pending_call_missing_name"),
    (
        ht::PENDING_MESSAGE_MISSING_ROLE,
        "pending_message_missing_role",
    ),
    (ht::PENDING_UNKNOWN_ROLE_PREFIX, "pending_unknown_role"),
];

fn legacy_pending_error(rest: &str, params: &mut FailureParams) {
    for (text, token) in PENDING_REASONS {
        if let Some(raw) = rest.strip_prefix(text) {
            params.reason = Some(token.into());
            params.error = (!raw.is_empty()).then(|| raw.to_string());
            return;
        }
    }
    params.error = Some(rest.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn card_status_code_for_each_state() {
        use CardStatusCode::*;
        assert_eq!(CardStatusCode::of(TaskState::Queued, false), Queued);
        assert_eq!(CardStatusCode::of(TaskState::Queued, true), Queued);
        assert_eq!(CardStatusCode::of(TaskState::Running, true), Running);
        assert_eq!(CardStatusCode::of(TaskState::Running, false), NoResult);
        assert_eq!(CardStatusCode::of(TaskState::Blocked, false), Blocked);
        assert_eq!(CardStatusCode::of(TaskState::Done, false), Done);
    }

    #[test]
    fn card_status_code_round_trips_as_snake_case() {
        for (code, text) in [
            (CardStatusCode::Queued, "queued"),
            (CardStatusCode::Running, "running"),
            (CardStatusCode::Blocked, "blocked"),
            (CardStatusCode::Done, "done"),
            (CardStatusCode::NoResult, "no_result"),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), json!(text));
            assert_eq!(
                serde_json::from_value::<CardStatusCode>(json!(text)).unwrap(),
                code
            );
            assert_eq!(code.as_str(), text);
        }
    }

    #[test]
    fn delivery_badge_round_trips() {
        assert_eq!(
            serde_json::to_value(DeliveryBadge::Briefing).unwrap(),
            json!("briefing")
        );
        assert_eq!(
            serde_json::from_value::<DeliveryBadge>(json!("briefing")).unwrap(),
            DeliveryBadge::Briefing
        );
    }

    #[test]
    fn failure_code_round_trips_as_snake_case() {
        for code in FailureCode::ALL {
            let value = serde_json::to_value(code).unwrap();
            assert_eq!(value, json!(code.as_str()));
            assert_eq!(serde_json::from_value::<FailureCode>(value).unwrap(), code);
        }
        let names: std::collections::HashSet<_> =
            FailureCode::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names.len(), FailureCode::ALL.len(), "codes are unique");
    }

    #[test]
    fn split_failure_strips_the_prefix_for_every_host_text() {
        let cases: [(String, FailureCode, Option<&str>); 17] = [
            (NO_RESULT.into(), FailureCode::NoResult, None),
            (
                APPROVAL_EXPIRED_REASON.into(),
                FailureCode::ApprovalExpired,
                None,
            ),
            (
                ht::TOOL_ROUND_LIMIT.into(),
                FailureCode::ToolRoundLimit,
                None,
            ),
            (
                ht::TOOL_STILL_NEEDS_APPROVAL.into(),
                FailureCode::ToolNeedsApproval,
                None,
            ),
            (
                ht::OUTBOUND_OVER_QUOTA.into(),
                FailureCode::OutboundOverQuota,
                None,
            ),
            (
                "出网超额：已用 3 次，上限 2".into(),
                FailureCode::OutboundOverQuota,
                Some("3/2"),
            ),
            (
                "出网超额：别的原因".into(),
                FailureCode::OutboundOverQuota,
                Some("别的原因"),
            ),
            (
                "出网只允许 GET：用了 POST".into(),
                FailureCode::OutboundNotGet,
                Some("POST"),
            ),
            (
                "你拒绝了：发送周报".into(),
                FailureCode::Denied,
                Some("发送周报"),
            ),
            (
                "你拒绝了：notes.write_note".into(),
                FailureCode::Denied,
                Some("notes.write_note"),
            ),
            (
                "模型没回：模型服务返回 500".into(),
                FailureCode::ModelNoReply,
                Some("模型服务返回 500"),
            ),
            (
                "无法保存审批状态：disk full".into(),
                FailureCode::SaveApprovalStateFailed,
                Some("disk full"),
            ),
            (
                "无法继续已批准的调用：待批准状态损坏：x".into(),
                FailureCode::ResumeApprovedCallFailed,
                Some("待批准状态损坏：x"),
            ),
            (
                "等待批准：发邮件".into(),
                FailureCode::WaitingApproval,
                Some("发邮件"),
            ),
            (
                "调用失败：磁盘满了".into(),
                FailureCode::ToolCallFailed,
                Some("磁盘满了"),
            ),
            (
                "runner exploded".into(),
                FailureCode::Other,
                Some("runner exploded"),
            ),
            ("你拒绝了：".into(), FailureCode::Denied, None),
        ];
        for (text, code, detail) in cases {
            assert_eq!(
                split_failure(&text),
                (code, detail.map(String::from)),
                "{text}"
            );
        }
    }

    #[test]
    fn the_detail_never_keeps_a_host_prefix() {
        for prefix in [
            ht::DENIED_PREFIX,
            ht::MODEL_NO_REPLY_PREFIX,
            ht::TOOL_CALL_FAILED_PREFIX,
            ht::SAVE_APPROVAL_STATE_FAILED_PREFIX,
            ht::RESUME_APPROVED_CALL_FAILED_PREFIX,
            ht::WAITING_APPROVAL_PREFIX,
        ] {
            let (code, detail) = split_failure(&format!("{prefix}原因"));
            assert_ne!(code, FailureCode::Other, "{prefix}");
            assert_eq!(detail.as_deref(), Some("原因"), "{prefix}");
        }
    }
    #[test]
    fn failure_data_round_trips_and_omits_empty_params() {
        let bare = FailureData::new(FailureCode::NoResult);
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"code": "no_result"})
        );
        let quota = FailureData::with(
            FailureCode::OutboundOverQuota,
            FailureParams {
                used: Some(3),
                limit: Some(2),
                ..Default::default()
            },
        );
        let value = serde_json::to_value(&quota).unwrap();
        assert_eq!(
            value,
            json!({"code": "outbound_over_quota", "params": {"used": 3, "limit": 2}})
        );
        assert_eq!(serde_json::from_value::<FailureData>(value).unwrap(), quota);
        assert_eq!(quota.detail().as_deref(), Some("3/2"));
    }

    #[test]
    fn from_legacy_keeps_only_language_neutral_detail() {
        let cases: [(&str, FailureCode, Option<&str>); 14] = [
            (
                "模型没回：模型服务返回 401",
                FailureCode::ModelNoReply,
                Some("401"),
            ),
            (
                "模型没回：连不上模型服务：timed out",
                FailureCode::ModelNoReply,
                Some("timed out"),
            ),
            (
                "模型没回：模型返回的不是 JSON",
                FailureCode::ModelNoReply,
                Some("response_not_json"),
            ),
            (
                "模型没回：此模型不支持工具调用",
                FailureCode::ModelNoReply,
                Some("tools_unsupported"),
            ),
            (
                "出网超额：已用 3 次，上限 2",
                FailureCode::OutboundOverQuota,
                Some("3/2"),
            ),
            (
                ht::OUTBOUND_OVER_QUOTA,
                FailureCode::OutboundOverQuota,
                None,
            ),
            (
                "出网只允许 GET：用了 POST",
                FailureCode::OutboundNotGet,
                Some("POST"),
            ),
            (
                "你拒绝了：notes.write_note",
                FailureCode::Denied,
                Some("notes.write_note"),
            ),
            ("你拒绝了：发送周报", FailureCode::Denied, Some("发送周报")),
            (
                "无法保存审批状态：无法写入审批状态：disk full",
                FailureCode::SaveApprovalStateFailed,
                Some("disk full"),
            ),
            (
                "无法继续已批准的调用：调用还没有被批准",
                FailureCode::ResumeApprovedCallFailed,
                Some("pending_not_approved"),
            ),
            (NO_RESULT, FailureCode::NoResult, None),
            (APPROVAL_EXPIRED_REASON, FailureCode::ApprovalExpired, None),
            (
                "runner exploded",
                FailureCode::Other,
                Some("runner exploded"),
            ),
        ];
        for (text, code, detail) in cases {
            let data = FailureData::from_legacy(text);
            assert_eq!(data.code, code, "{text}");
            assert_eq!(data.detail().as_deref(), detail, "{text}");
        }
        let denied = FailureData::from_legacy("你拒绝了：notes.write_note");
        assert_eq!(denied.params.tool.as_deref(), Some("notes.write_note"));
        let waiting = FailureData::from_legacy("等待批准：send_email");
        assert_eq!(waiting.params.tool.as_deref(), Some("send_email"));
    }
}
