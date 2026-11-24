//! Allow-rules settings page (docs/design/auto-review.md).
//!
//! Adding or deleting a rule only proposes a confirm card in the owner's DM.
//! The list shows confirmed rules only.

use chrono::{DateTime, TimeZone};

/// Sidebar and page title, directly under 「MCP」.
pub const TITLE: &str = "放行规则";
pub const ADD: &str = "添加";
pub const DELETE: &str = "删除";
/// Empty state, wording from 最优雅UI姐.
pub const EMPTY: &str = "还没有放行规则。有风险的操作都会先问你。";
/// Shown after POST/DELETE returns 202, grey, directly under the input.
pub const PENDING_NOTE: &str = "已发到私聊，确认后生效";
/// Placeholder, wording from 最优雅UI姐.
pub const PLACEHOLDER: &str = "用一句话写，比如：允许合并 CI 全绿的 PR";
pub const NEED_TEXT: &str = "请填写规则";
pub const LOADING: &str = "读取中";
pub const LOAD_FAILED: &str = "读不到";

/// Grey line on an add confirm card, wording from 最优雅UI姐.
pub const ADD_HINT: &str = "确认后加入放行规则";
/// Grey line on a delete confirm card, wording from 最优雅UI姐.
pub const DELETE_HINT: &str = "确认后删除这条规则";
/// Primary button on an add card, wording from 最优雅UI姐.
pub const ADD_ACTION: &str = "添加";
/// Primary (red) button on a delete card, wording from the design.
pub const DELETE_ACTION: &str = "删除规则";
/// Deny button on rule confirm cards, wording from 最优雅UI姐.
pub const DROP: &str = "不用";

/// Prefix on a blocked-action DM card's reason line, wording from 最优雅UI姐.
pub const BLOCKED_REASON_PREFIX: &str = "被拦原因：";

/// Grey reason line for a blocked-action card. `reason` is the host value.
pub fn blocked_reason_line(reason: &str) -> String {
    format!("{BLOCKED_REASON_PREFIX}{reason}")
}

/// Group-thread line under 「等你批准」, wording from the design.
pub const SENT_TO_OWNER: &str = "已发到主人私聊";
/// Top-right status on an expired card, wording from the design.
pub const EXPIRED: &str = "已过期";
/// Footnote text on an expired card, wording from the design.
pub const EXPIRED_FOOT: &str = "没人批准，已过期";

/// TODO: switch to `robroz_contract::APPROVAL_EXPIRED_REASON` once the
/// backend PR lands. Do not hand-write this string at call sites.
pub const APPROVAL_EXPIRED_REASON: &str = "审批已过期";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub text: String,
    pub created_at: String,
}

impl Rule {
    /// 「添加于 {日期}」, local `M月D日` from `createdAt`. Empty if unparsable.
    pub fn added_on<Tz: TimeZone>(&self, tz: &Tz) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        added_on(&self.created_at, tz)
    }
}

pub fn added_on<Tz: TimeZone>(created_at: &str, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    DateTime::parse_from_rfc3339(created_at)
        .map(|t| {
            let local = t.with_timezone(tz);
            format!("添加于 {}", local.format("%-m月%-d日"))
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct Page {
    /// `None` until the first successful load.
    pub rules: Option<Vec<Rule>>,
    pub draft: String,
    pub busy: bool,
    pub note: Option<String>,
}

impl Page {
    pub fn load(&mut self, rules: Vec<Rule>) {
        self.rules = Some(rules);
        self.busy = false;
        // Keep a pending note if we just proposed; a fresh open clears it.
    }

    pub fn opened(&mut self, rules: Vec<Rule>) {
        self.note = None;
        self.load(rules);
    }

    pub fn load_failed(&mut self, reason: &str) {
        self.busy = false;
        self.note = Some(if reason.is_empty() {
            LOAD_FAILED.to_string()
        } else {
            format!("{LOAD_FAILED}：{reason}")
        });
    }

    pub fn set_draft(&mut self, text: String) {
        self.draft = text;
        if self.note.as_deref() == Some(NEED_TEXT) {
            self.note = None;
        }
    }

    /// Trimmed body for POST, or an inline note if empty.
    pub fn to_add(&mut self) -> Option<String> {
        let text = self.draft.trim().to_string();
        if text.is_empty() {
            self.note = Some(NEED_TEXT.into());
            return None;
        }
        // One line only, as the design requires.
        if text.lines().count() > 1 {
            self.note = Some(NEED_TEXT.into());
            return None;
        }
        self.note = None;
        Some(text)
    }

    /// POST/DELETE returned 202: keep the list, show the pending note.
    pub fn proposed(&mut self) {
        self.busy = false;
        self.draft.clear();
        self.note = Some(PENDING_NOTE.into());
    }

    pub fn propose_failed(&mut self, reason: &str) {
        self.busy = false;
        self.note = Some(reason.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.rules.as_ref().is_some_and(|r| r.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn rule(id: &str, text: &str, at: &str) -> Rule {
        Rule {
            id: id.into(),
            text: text.into(),
            created_at: at.into(),
        }
    }

    #[test]
    fn empty_list_is_the_empty_state() {
        let mut page = Page::default();
        assert!(!page.is_empty());
        page.opened(Vec::new());
        assert!(page.is_empty());
        assert_eq!(page.note, None);
    }

    #[test]
    fn list_keeps_text_and_added_on() {
        let mut page = Page::default();
        page.opened(vec![rule(
            "r1",
            "允许合并已经过 CI 的 PR",
            "2026-11-20T08:00:00Z",
        )]);
        let r = &page.rules.as_ref().unwrap()[0];
        assert_eq!(r.text, "允许合并已经过 CI 的 PR");
        assert_eq!(r.added_on(&Utc), "添加于 11月20日");
    }

    #[test]
    fn add_requires_a_non_empty_line() {
        let mut page = Page::default();
        page.opened(Vec::new());
        assert_eq!(page.to_add(), None);
        assert_eq!(page.note.as_deref(), Some(NEED_TEXT));
        page.set_draft("   ".into());
        assert_eq!(page.to_add(), None);
        page.set_draft("允许发周报\n第二行".into());
        assert_eq!(page.to_add(), None);
        page.set_draft("  允许发周报  ".into());
        assert_eq!(page.to_add().as_deref(), Some("允许发周报"));
        assert_eq!(page.note, None);
    }

    #[test]
    fn proposed_add_clears_the_draft_and_does_not_list_the_rule() {
        let mut page = Page::default();
        page.opened(Vec::new());
        page.draft = "允许发周报".into();
        assert_eq!(page.to_add().as_deref(), Some("允许发周报"));
        page.busy = true;
        page.proposed();
        assert!(page.draft.is_empty());
        assert_eq!(page.note.as_deref(), Some(PENDING_NOTE));
        assert!(page.is_empty(), "unconfirmed rules stay off the list");
        assert!(!page.busy);
    }

    #[test]
    fn proposed_delete_leaves_the_rule_listed() {
        let mut page = Page::default();
        page.opened(vec![rule("r1", "允许发周报", "2026-11-20T08:00:00Z")]);
        page.busy = true;
        page.proposed();
        assert_eq!(page.rules.as_ref().unwrap().len(), 1);
        assert_eq!(page.note.as_deref(), Some(PENDING_NOTE));
    }

    #[test]
    fn blocked_reason_line_uses_the_settled_prefix() {
        assert_eq!(
            blocked_reason_line("这条操作会改生产数据"),
            "被拦原因：这条操作会改生产数据"
        );
    }

    #[test]
    fn confirm_hints_are_the_settled_copy() {
        assert_eq!(ADD_HINT, "确认后加入放行规则");
        assert_eq!(DELETE_HINT, "确认后删除这条规则");
        assert_eq!(PLACEHOLDER, "用一句话写，比如：允许合并 CI 全绿的 PR");
    }
}
