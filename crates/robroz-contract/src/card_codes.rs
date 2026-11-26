//! Codes plus language-neutral parameters for the last host-written card
//! copy (#180): the status card's `progress` line (including Watchdog hints)
//! and the auto-review `reason` on an approval card.
//!
//! The legacy text fields (`progress`, `reason`) are unchanged and still sent.
//! The code fields are additive and optional: an older host omits them and a
//! client that does not know them ignores them. A client renders its own
//! copy from the code and parameters and does not show the legacy text next
//! to them, except where a code says the text is the bro's or the model's own
//! words (`custom`, `review_model`).

use serde::{Deserialize, Serialize};

/// `progressCode` on a status card: what the bro is doing now.
///
/// | code | `progressParams` | legacy `progress` |
/// |---|---|---|
/// | `started` | none | 「开始干活」 |
/// | `thinking` | none | 「在想怎么回」 |
/// | `in_sandbox` | none | 「在沙箱里跑」 |
/// | `using_tool` | `tool` | 「在用 {tool}」 |
/// | `router_step` | `step`, `steps` | 「{step}/{steps} 步」 |
/// | `step_timeout` | `minutes` | 「模型和工具还没回，已等 {minutes} 分钟」 (Watchdog) |
/// | `stalled` | `minutes` | 「{minutes} 分钟没有新进度」 (Watchdog) |
/// | `custom` | `text` | the bro's own progress line, as written |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressCode {
    Started,
    Thinking,
    InSandbox,
    UsingTool,
    RouterStep,
    StepTimeout,
    Stalled,
    Custom,
}

impl ProgressCode {
    pub const ALL: [ProgressCode; 8] = [
        Self::Started,
        Self::Thinking,
        Self::InSandbox,
        Self::UsingTool,
        Self::RouterStep,
        Self::StepTimeout,
        Self::Stalled,
        Self::Custom,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Thinking => "thinking",
            Self::InSandbox => "in_sandbox",
            Self::UsingTool => "using_tool",
            Self::RouterStep => "router_step",
            Self::StepTimeout => "step_timeout",
            Self::Stalled => "stalled",
            Self::Custom => "custom",
        }
    }
}

/// `progressParams`: the language-neutral parts of a progress line. Every
/// field is optional and omitted when unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressParams {
    /// Tool name (`server.tool` for an MCP tool), for `using_tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 1-based step, for `router_step`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    /// Number of steps, for `router_step`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    /// Whole minutes (at least 1), for `step_timeout` and `stalled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u64>,
    /// The bro's own line, for `custom`. Not host copy: show it as is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl ProgressParams {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A progress line as the host stores it next to the legacy text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressData {
    pub code: ProgressCode,
    #[serde(default, skip_serializing_if = "ProgressParams::is_empty")]
    pub params: ProgressParams,
}

impl ProgressData {
    pub fn new(code: ProgressCode) -> Self {
        Self {
            code,
            params: ProgressParams::default(),
        }
    }

    /// `progressParams` for the event: `None` when there are none.
    pub fn wire_params(&self) -> Option<ProgressParams> {
        (!self.params.is_empty()).then(|| self.params.clone())
    }
}

/// `reasonCode` on an approval card: why auto-review settled or showed it,
/// or why the card ended.
///
/// | code | legacy `reason` |
/// |---|---|
/// | `review_model` | the review model's own one-line reason, in the language of the rules (show `reason` as is) |
/// | `review_allowed` | 「符合放行规则」: allowed, the model gave no reason |
/// | `review_no_match` | 「没有放行规则允许这个操作」: no rule allows it (or the model gave no block reason) |
/// | `review_no_model` | 「还没设置模型，没法自动审核」 |
/// | `review_failed` | 「自动审核没有成功，需要你批准」: review failed twice |
/// | `review_allow_failed` | review allowed it but the host could not continue the task; the owner decides |
/// | `expired` | 「审批已过期」 ([`crate::APPROVAL_EXPIRED_REASON`]) |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReasonCode {
    ReviewModel,
    ReviewAllowed,
    ReviewNoMatch,
    ReviewNoModel,
    ReviewFailed,
    ReviewAllowFailed,
    Expired,
}

impl ApprovalReasonCode {
    pub const ALL: [ApprovalReasonCode; 7] = [
        Self::ReviewModel,
        Self::ReviewAllowed,
        Self::ReviewNoMatch,
        Self::ReviewNoModel,
        Self::ReviewFailed,
        Self::ReviewAllowFailed,
        Self::Expired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReviewModel => "review_model",
            Self::ReviewAllowed => "review_allowed",
            Self::ReviewNoMatch => "review_no_match",
            Self::ReviewNoModel => "review_no_model",
            Self::ReviewFailed => "review_failed",
            Self::ReviewAllowFailed => "review_allow_failed",
            Self::Expired => "expired",
        }
    }

    /// Parse the stored column value.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == text)
    }

    /// The code for a card stored before the column existed, from its exact
    /// `reason` text. Only the host's fixed texts match; any other reason on
    /// a reviewed card is the model's own words. Used once, by the backfill.
    pub fn from_legacy(reason: &str, reviewed: bool) -> Option<Self> {
        use crate::host_text as ht;
        let fixed = [
            (ht::REVIEW_ALLOW_FALLBACK, Self::ReviewAllowed),
            (ht::REVIEW_NO_MATCH_REASON, Self::ReviewNoMatch),
            (ht::REVIEW_NO_MODEL_REASON, Self::ReviewNoModel),
            (ht::REVIEW_FAILED_REASON, Self::ReviewFailed),
            (ht::APPROVE_CONTINUE_MARKER_FAILED, Self::ReviewAllowFailed),
            (ht::APPROVE_TAKEOVER_MARKER_FAILED, Self::ReviewAllowFailed),
            (crate::APPROVAL_EXPIRED_REASON, Self::Expired),
        ];
        fixed
            .iter()
            .find(|(text, _)| *text == reason)
            .map(|(_, code)| *code)
            .or(reviewed.then_some(Self::ReviewModel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_codes_round_trip_as_snake_case() {
        for code in ProgressCode::ALL {
            let value = serde_json::to_value(code).unwrap();
            assert_eq!(value, json!(code.as_str()));
            assert_eq!(serde_json::from_value::<ProgressCode>(value).unwrap(), code);
        }
    }

    #[test]
    fn progress_data_omits_empty_params() {
        let bare = ProgressData::new(ProgressCode::Thinking);
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"code": "thinking"})
        );
        assert_eq!(bare.wire_params(), None);
        let step = ProgressData {
            code: ProgressCode::RouterStep,
            params: ProgressParams {
                step: Some(2),
                steps: Some(3),
                ..Default::default()
            },
        };
        let value = serde_json::to_value(&step).unwrap();
        assert_eq!(
            value,
            json!({"code": "router_step", "params": {"step": 2, "steps": 3}})
        );
        assert_eq!(serde_json::from_value::<ProgressData>(value).unwrap(), step);
    }

    #[test]
    fn approval_reason_codes_round_trip_and_parse() {
        for code in ApprovalReasonCode::ALL {
            let value = serde_json::to_value(code).unwrap();
            assert_eq!(value, json!(code.as_str()));
            assert_eq!(ApprovalReasonCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(ApprovalReasonCode::parse("nope"), None);
    }

    #[test]
    fn legacy_reasons_map_by_exact_text() {
        use crate::host_text as ht;
        use ApprovalReasonCode::*;
        assert_eq!(
            ApprovalReasonCode::from_legacy(ht::REVIEW_NO_MODEL_REASON, true),
            Some(ReviewNoModel)
        );
        assert_eq!(
            ApprovalReasonCode::from_legacy(crate::APPROVAL_EXPIRED_REASON, false),
            Some(Expired)
        );
        assert_eq!(
            ApprovalReasonCode::from_legacy("The rule allows weekly reports.", true),
            Some(ReviewModel)
        );
        assert_eq!(ApprovalReasonCode::from_legacy("anything", false), None);
    }
}
