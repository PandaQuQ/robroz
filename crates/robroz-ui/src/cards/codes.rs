//! Map v0.2 `statusCode` / `failureCode` / `badge` to the client's own copy.
//!
//! The host still sends the legacy `label` / `failure` / `briefing` text.
//! When a code is present the client never shows that text; it picks a
//! sentence from i18n. `failureDetail` is host-written and may be in the
//! other language: if it does not fit the UI locale it is kept off the
//! main line, in a row the user can open.

use robroz_contract::{CardStatusCode, DeliveryBadge, FailureCode};

use crate::i18n::{locale, Key, Locale};

/// Whether a host `failureDetail` can sit on the main line in `locale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailFit {
    /// Same language as the UI, or language-neutral (a status code, tool
    /// name, HTTP method, or a figure like `3/2`).
    Inline,
    /// CJK in an English UI, or English prose in a Chinese UI.
    Collapse,
}

/// A CJK ideograph, kana, hangul, or CJK punctuation / fullwidth mark.
pub fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{3400}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{3040}'..='\u{30FF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{20000}'..='\u{2CEAF}'
            | '\u{3000}'..='\u{303F}'
            | '\u{FF00}'..='\u{FFEF}'
    )
}

/// A token that does not belong to one UI language: digits, an HTTP
/// method, a tool name, a path, a status code, or `used/limit`.
fn is_neutral(detail: &str) -> bool {
    if detail.chars().any(is_cjk) {
        return false;
    }
    if !detail.chars().any(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    !detail.chars().any(char::is_whitespace)
}

/// How `detail` should be shown in `locale`.
pub fn detail_fit(locale: Locale, detail: &str) -> DetailFit {
    let detail = detail.trim();
    if detail.is_empty() || is_neutral(detail) {
        return DetailFit::Inline;
    }
    let cjk = detail.chars().any(is_cjk);
    match locale {
        Locale::En if cjk => DetailFit::Collapse,
        Locale::Zh if !cjk => DetailFit::Collapse,
        _ => DetailFit::Inline,
    }
}

/// The status word for a `statusCode`.
pub fn status_text(code: CardStatusCode) -> String {
    match code {
        CardStatusCode::Queued => Key::CardQueued.into(),
        CardStatusCode::Running => Key::CardRunning.into(),
        CardStatusCode::Blocked => Key::CardBlocked.into(),
        CardStatusCode::Done => Key::CardDone.into(),
        CardStatusCode::NoResult => Key::HostNoResult.into(),
    }
}

/// The corner badge for a delivery `badge`.
pub fn badge_text(badge: DeliveryBadge) -> String {
    match badge {
        DeliveryBadge::Briefing => Key::CardBriefing.into(),
    }
}

/// Standalone sentence for a `failureCode`, with no prefix and no detail.
pub fn failure_sentence(code: FailureCode) -> String {
    failure_key(code).into()
}

pub fn failure_key(code: FailureCode) -> Key {
    match code {
        FailureCode::NoResult => Key::HostNoResult,
        FailureCode::Denied => Key::FailDenied,
        FailureCode::ApprovalExpired => Key::FailApprovalExpired,
        FailureCode::ModelNoReply => Key::FailModelNoReply,
        FailureCode::ToolRoundLimit => Key::FailToolRoundLimit,
        FailureCode::OutboundOverQuota => Key::FailOutboundOverQuota,
        FailureCode::OutboundNotGet => Key::FailOutboundNotGet,
        FailureCode::SaveApprovalStateFailed => Key::FailSaveApprovalState,
        FailureCode::ResumeApprovedCallFailed => Key::FailResumeApprovedCall,
        FailureCode::WaitingApproval => Key::FailWaitingApproval,
        FailureCode::ToolCallFailed => Key::FailToolCall,
        FailureCode::ToolNeedsApproval => Key::FailToolNeedsApproval,
        FailureCode::Other => Key::FailOther,
    }
}

/// The line a failed card shows, plus an optional detail to hide behind
/// a disclosure when it is in the other language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureLine {
    pub text: String,
    pub collapsed: Option<String>,
}

impl FailureLine {
    /// Build from a v0.2 code and optional raw `failureDetail`.
    ///
    /// `no_result` never takes the failed prefix. For every other code a
    /// usable detail is shown as exactly one prefix plus that detail
    /// (`Couldn't finish: …` / 「没做成：…」). A detail in the other
    /// language is kept off the main line; the sentence for the code is
    /// shown instead.
    pub fn from_code(code: FailureCode, detail: Option<&str>) -> Self {
        Self::from_code_in(locale(), code, detail)
    }

    pub fn from_code_in(locale: Locale, code: FailureCode, detail: Option<&str>) -> Self {
        let sentence = failure_key(code).to_string_in(locale);
        let Some(detail) = detail.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self {
                text: sentence,
                collapsed: None,
            };
        };
        if code == FailureCode::NoResult || detail_fit(locale, detail) == DetailFit::Collapse {
            return Self {
                text: sentence,
                collapsed: (code != FailureCode::NoResult).then(|| detail.to_string()),
            };
        }
        Self {
            text: Key::HostFailedBecause.fill_in(locale, &[&detail]),
            collapsed: None,
        }
    }
}

trait ToStringIn {
    fn to_string_in(self, locale: Locale) -> String;
}

impl ToStringIn for Key {
    fn to_string_in(self, locale: Locale) -> String {
        crate::i18n::text_in(locale, self).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{scoped, text_in, Locale};

    fn line(locale: Locale, code: FailureCode, detail: Option<&str>) -> FailureLine {
        FailureLine::from_code_in(locale, code, detail)
    }

    #[test]
    fn every_failure_code_has_a_non_empty_sentence_in_both_locales() {
        for code in FailureCode::ALL {
            for locale in [Locale::En, Locale::Zh] {
                let s = text_in(locale, failure_key(code));
                assert!(!s.trim().is_empty(), "{code:?} {locale:?}");
                // The sentence itself must not carry the failed prefix: the
                // client adds that at most once, and only around a detail.
                assert!(
                    !s.starts_with("Couldn't finish:") && !s.starts_with("没做成："),
                    "sentence for {code:?} starts with the failed prefix: {s}"
                );
            }
        }
        assert_eq!(FailureCode::ALL.len(), 13);
    }

    #[test]
    fn no_result_never_takes_the_prefix_even_with_a_detail() {
        let l = line(Locale::Zh, FailureCode::NoResult, Some("模型服务返回 401"));
        assert_eq!(l.text, "没写出结果");
        assert_eq!(l.collapsed, None);
        let l = line(Locale::En, FailureCode::NoResult, None);
        assert_eq!(l.text, "Finished with no result");
    }

    #[test]
    fn a_matching_detail_is_the_failed_prefix_once() {
        let l = line(
            Locale::Zh,
            FailureCode::ModelNoReply,
            Some("模型服务返回 401"),
        );
        assert_eq!(l.text, "没做成：模型服务返回 401");
        assert_eq!(l.collapsed, None);
        // Never the old double prefix.
        assert!(!l.text.contains("没做成：调用失败"));
        assert!(!l.text.contains("调用失败："));
    }

    #[test]
    fn a_cjk_detail_is_collapsed_in_english() {
        let l = line(
            Locale::En,
            FailureCode::ModelNoReply,
            Some("模型服务返回 401"),
        );
        assert_eq!(l.text, "The model did not reply.");
        assert_eq!(l.collapsed.as_deref(), Some("模型服务返回 401"));
        assert!(!l.text.contains("模型"));
    }

    #[test]
    fn english_prose_is_collapsed_in_chinese() {
        let l = line(
            Locale::Zh,
            FailureCode::ModelNoReply,
            Some("model returned 401"),
        );
        assert_eq!(l.text, "模型没有回复。");
        assert_eq!(l.collapsed.as_deref(), Some("model returned 401"));
    }

    #[test]
    fn a_neutral_detail_is_inlined_in_both_locales() {
        for (locale, want) in [
            (Locale::En, "Couldn't finish: 401"),
            (Locale::Zh, "没做成：401"),
        ] {
            let l = line(locale, FailureCode::ModelNoReply, Some("401"));
            assert_eq!(l.text, want, "{locale:?}");
            assert_eq!(l.collapsed, None);
        }
        let quota = line(Locale::En, FailureCode::OutboundOverQuota, Some("3/2"));
        assert_eq!(quota.text, "Couldn't finish: 3/2");
        let method = line(Locale::Zh, FailureCode::OutboundNotGet, Some("POST"));
        assert_eq!(method.text, "没做成：POST");
        let tool = line(
            Locale::En,
            FailureCode::WaitingApproval,
            Some("server.tool"),
        );
        assert_eq!(tool.text, "Couldn't finish: server.tool");
    }

    #[test]
    fn an_absent_detail_is_the_code_sentence() {
        let l = line(Locale::En, FailureCode::ToolRoundLimit, None);
        assert_eq!(l.text, "The tool was called too many times.");
        assert_eq!(l.collapsed, None);
        let l = line(Locale::Zh, FailureCode::ApprovalExpired, None);
        assert_eq!(l.text, "审批已过期。");
    }

    #[test]
    fn detail_fit_treats_tokens_as_neutral_and_prose_by_script() {
        assert_eq!(detail_fit(Locale::En, "POST"), DetailFit::Inline);
        assert_eq!(detail_fit(Locale::Zh, "3/2"), DetailFit::Inline);
        assert_eq!(detail_fit(Locale::En, "server.tool"), DetailFit::Inline);
        assert_eq!(
            detail_fit(Locale::En, "模型服务返回 401"),
            DetailFit::Collapse
        );
        assert_eq!(
            detail_fit(Locale::Zh, "model returned 401"),
            DetailFit::Collapse
        );
        assert_eq!(
            detail_fit(Locale::En, "model returned 401"),
            DetailFit::Inline
        );
        assert_eq!(
            detail_fit(Locale::Zh, "模型服务返回 401"),
            DetailFit::Inline
        );
        assert_eq!(detail_fit(Locale::En, ""), DetailFit::Inline);
    }

    #[test]
    fn status_and_badge_use_the_existing_card_words() {
        let _g = scoped(Locale::En);
        assert_eq!(status_text(CardStatusCode::Queued), "Queued");
        assert_eq!(status_text(CardStatusCode::Running), "Running");
        assert_eq!(status_text(CardStatusCode::Blocked), "Stuck");
        assert_eq!(status_text(CardStatusCode::Done), "Done");
        assert_eq!(
            status_text(CardStatusCode::NoResult),
            "Finished with no result"
        );
        assert_eq!(badge_text(DeliveryBadge::Briefing), "Briefing");
        let _g = scoped(Locale::Zh);
        assert_eq!(status_text(CardStatusCode::Running), "进行");
        assert_eq!(badge_text(DeliveryBadge::Briefing), "简报");
    }
}
