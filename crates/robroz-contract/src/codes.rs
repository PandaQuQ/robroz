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
/// What `failureDetail` holds for each code:
///
/// | code | detail |
/// |---|---|
/// | `no_result` | none (the client shows it without a prefix) |
/// | `approval_expired` | none |
/// | `tool_round_limit` | none |
/// | `tool_needs_approval` | none |
/// | `denied` | the denied task line or `server.tool` |
/// | `model_no_reply` | the model error |
/// | `outbound_over_quota` | `"{used}/{limit}"`, or none |
/// | `outbound_not_get` | the HTTP method that was used |
/// | `save_approval_state_failed` | the error |
/// | `resume_approved_call_failed` | the error |
/// | `waiting_approval` | the tool's display name |
/// | `tool_call_failed` | the tool error |
/// | `other` | the whole text the host wrote |
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
}
