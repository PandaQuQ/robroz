//! Map Chinese text the host sends to the active locale.
//!
//! The host keeps writing its own strings (the contract is frozen). Where a
//! string is a public constant, it is compared against that constant, never
//! a copy of its text. Host-crate strings the client cannot import (the
//! client does not depend on `robroz-host`) are matched by their exact text;
//! the client's integration tests pin the public ones. Anything not listed
//! here passes through unchanged.

use std::borrow::Cow;

use robroz_card::NO_RESULT;
use robroz_contract::{
    TaskState, APPROVAL_EXPIRED_REASON, APPROVAL_SENT_TO_OWNER_DM_NOTICE, MODEL_TEST_BAD_KEY,
    SETTINGS_APPROVAL_BOT_NAME,
};

use super::{locale, Key, Locale};

/// Whole strings, compared with `==`.
fn exact(s: &str) -> Option<Key> {
    // Contract and robroz-card constants.
    if s == APPROVAL_SENT_TO_OWNER_DM_NOTICE {
        return Some(Key::HostSentToOwnerDm);
    }
    if s == APPROVAL_EXPIRED_REASON {
        return Some(Key::HostApprovalExpired);
    }
    if s == SETTINGS_APPROVAL_BOT_NAME {
        return Some(Key::HostSettingsBotName);
    }
    if s == MODEL_TEST_BAD_KEY {
        return Some(Key::HostModelBadKey);
    }
    if s == NO_RESULT {
        return Some(Key::HostNoResult);
    }
    // robroz-card's own status copy (task status card and its action).
    for (state, key) in [
        (TaskState::Queued, Key::CardQueued),
        (TaskState::Done, Key::CardDone),
        (TaskState::Blocked, Key::CardBlocked),
    ] {
        if s == robroz_card::status_line(state, false) {
            return Some(key);
        }
    }
    if s == robroz_card::status_line(TaskState::Running, true) {
        return Some(Key::CardRunning);
    }
    if s == robroz_card::briefing_badge() {
        return Some(Key::CardBriefing);
    }
    if Some(s) == robroz_card::appearance(TaskState::Blocked, false).action {
        return Some(Key::CardTakeOver);
    }
    // Host-crate strings (not importable from the client).
    let key = match s {
        // host/src/progress.rs
        "开始干活" => Key::HostProgressStarted,
        "在想怎么回" => Key::HostProgressThinking,
        "在沙箱里跑" => Key::HostProgressInSandbox,
        // host/src/review
        "自动审核没有成功，需要你批准" => Key::HostReviewFailed,
        "没有放行规则允许这个操作" => Key::HostReviewNoMatch,
        "还没设置模型，没法自动审核" => Key::HostReviewNoModel,
        // host/src/tools.rs
        "工具调用超过 8 轮" => Key::HostToolRoundLimit,
        "调用失败：这个工具还需要批准" => Key::HostToolNeedsApproval,
        // host/src/model_settings.rs, host/src/search.rs
        "还没设置模型" => Key::HostModelNotConfigured,
        "还没设置联网搜索" => Key::HostSearchNotConfigured,
        // host/src/lib.rs, host/src/approvals.rs
        "不支持这个动作" => Key::HostUnsupportedAction,
        "请先回答审批卡" => Key::HostResumeNeedsApproval,
        "恢复失败，找不到卡住记录" => Key::HostResumeNoIntent,
        "恢复失败，无法写入接管标记" => Key::HostResumeMarkerFailed,
        // host/src/mcp.rs
        "这个名字已经有了" => Key::HostNameTaken,
        "MCP 服务器不存在" => Key::HostMcpNotFound,
        "群不存在" => Key::HostGroupNotFound,
        "只有 stdio 服务器需要批准启动命令" => Key::HostStdioOnlyApprove,
        // host/src/memories.rs, host/src/skills.rs
        "这个 bro 不存在" => Key::HostUnknownBro,
        "记忆不存在" => Key::HostMemoryNotFound,
        "技能不存在" => Key::HostSkillNotFound,
        "这个 slug 已经有了" => Key::HostSlugTaken,
        "slug 只能是小写字母、数字和连字符" => Key::HostBadSlug,
        _ => return None,
    };
    Some(key)
}

/// `prefix` + host value; the value is mapped again and goes into `{0}`.
const PREFIXES: [(&str, Key); 5] = [
    // host/src/pump.rs: the tool a bro is using right now.
    ("在用 ", Key::HostProgressUsing),
    // host/src/approvals.rs: a denied task's failure.
    ("你拒绝了：", Key::HostDeniedFailure),
    // host/src/approvals.rs: the summary of an external-write approval.
    ("要对外写入：", Key::HostExternalWrite),
    // host/src/tools.rs
    ("调用失败：", Key::HostCallFailed),
    // robroz-card `failure_line`: 「没做成：原因」.
    ("没做成：", Key::HostFailedBecause),
];

/// `s` in the active locale when it is a known host string; otherwise `s`
/// unchanged. Chinese is the host's own language, so it always passes
/// through as is.
pub fn host_text(s: &str) -> Cow<'_, str> {
    host_text_in(locale(), s)
}

/// [`host_text`] for a given locale.
pub fn host_text_in(locale: Locale, s: &str) -> Cow<'_, str> {
    if locale == Locale::Zh {
        return Cow::Borrowed(s);
    }
    if let Some(key) = exact(s) {
        return Cow::Borrowed(super::text_in(locale, key));
    }
    for (prefix, key) in PREFIXES {
        if let Some(rest) = s.strip_prefix(prefix) {
            if rest.is_empty() {
                continue;
            }
            let inner = host_text_in(locale, rest);
            return Cow::Owned(key.fill_in(locale, &[&inner]));
        }
    }
    Cow::Borrowed(s)
}
