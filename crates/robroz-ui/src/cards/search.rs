//! 联网搜索 settings page (robroz#76 `/api/settings/search`).
//!
//! Pick Brave (key) or SearXNG (address). The key box is masked and never
//! filled from the host: after a save it only shows 「已填 · 末4位」 with
//! 「清除」. Clicking it marks the key for deletion (「保存后删除」 / 「撤销」)
//! until the next save. Every save is followed by one test call, shown in
//! place as 「能用了」 or 「连不上：原因」.
//!
//! `source` is optional on the host: 「来自环境变量」 shows only when it is
//! `env`. When it is absent nothing is shown and nothing is guessed.

use super::secret::{self, MaskedKey};
use crate::i18n::Key;

pub const TITLE: Key = Key::SearchTitle;
pub const BRAVE: &str = "Brave";
pub const SEARXNG: &str = "SearXNG";
pub const KEY_PLACEHOLDER: Key = Key::SearchKeyPlaceholder;
pub const URL_PLACEHOLDER: Key = Key::SearchUrlPlaceholder;
pub use secret::{CLEAR, PENDING_CLEAR, UNDO};
pub const SAVE: Key = Key::SearchSave;
pub const CANCEL: Key = Key::SearchCancel;
pub const FROM_ENV: Key = Key::SearchFromEnv;
pub const NOT_SET: Key = Key::SearchNotSet;
pub const TESTING: Key = Key::SearchTesting;
pub const WORKS: Key = Key::SearchWorks;
pub const NEED_KEY: Key = Key::SearchNeedKey;
pub const NEED_URL: Key = Key::SearchNeedUrl;
/// Delivery entry when a task needed search (`needsSetup: "search"`).
pub const GO_SETUP: Key = Key::SearchGoSetup;
/// Shown under the delivery path when `needsSetup` is `"search"` and there is no failure.
pub const NEEDS_SETUP_HINT: Key = Key::SearchNeedsSetupHint;

pub fn key_filled(last4: Option<&str>) -> String {
    secret::filled(last4)
}

/// Drop a leading 「连不上：」 (the host's, or this locale's) so a later
/// prefix is not doubled.
pub fn strip_connect_prefix(reason: &str) -> &str {
    super::model::strip_connect_prefix(reason)
}

pub fn cannot_connect(reason: &str) -> String {
    super::model::cannot_connect(reason)
}

/// PUT succeeded, then the test failed.
pub fn saved_but_unreachable(reason: &str) -> String {
    Key::SavedButUnreachable.fill(&[&crate::i18n::host_text(strip_connect_prefix(reason))])
}

/// The save itself failed. Keep a host 「连不上：」 rather than doubling it.
pub fn failed(reason: &str) -> String {
    Key::NotSaved.fill(&[&crate::i18n::host_text(reason)])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Brave,
    Searxng,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "brave" => Some(Provider::Brave),
            "searxng" => Some(Provider::Searxng),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Brave => "brave",
            Provider::Searxng => "searxng",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::Brave => BRAVE,
            Provider::Searxng => SEARXNG,
        }
    }
}

/// Where the effective config comes from, if the host says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Settings,
}

impl Source {
    /// `None` for absent or unknown values: never guess.
    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s {
            Some("env") => Some(Source::Env),
            Some("settings") => Some(Source::Settings),
            _ => None,
        }
    }
}

/// What the host returned (`GET`/`PUT`), key never included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Current {
    pub provider: Option<Provider>,
    pub url: Option<String>,
    pub has_key: bool,
    pub key_last4: Option<String>,
    pub source: Option<Source>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Test {
    Running,
    Ok,
    Failed(String),
}

/// What 保存 sends. `key`: `None` leave as-is, `Some(None)` clear,
/// `Some(Some(k))` set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put {
    pub provider: Provider,
    pub url: Option<String>,
    pub key: Option<Option<String>>,
    /// Wipe provider, url and key so the page falls back to env or empty.
    pub clear: bool,
}

impl Put {
    pub fn clear() -> Self {
        Self {
            provider: Provider::Brave,
            url: None,
            key: Some(None),
            clear: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub current: Option<Current>,
    /// Form state.
    pub provider: Option<Provider>,
    pub url: String,
    /// Masked key shared with the MCP secret row. 清除 still wipes the
    /// whole search settings (provider, url and key) on save, not only the key.
    pub key: MaskedKey,
    pub busy: bool,
    pub note: Option<String>,
    pub test: Option<Test>,
}

impl Page {
    /// Fresh data from the host: the form shows what is in effect.
    pub fn load(&mut self, cur: Current) {
        self.provider = cur.provider;
        self.url = cur.url.clone().unwrap_or_default();
        self.key = MaskedKey::from_stored(cur.has_key, cur.key_last4.clone());
        self.current = Some(cur);
    }

    /// 「来自环境变量」 only when the host says `env`.
    pub fn from_env(&self) -> bool {
        self.current.as_ref().and_then(|c| c.source) == Some(Source::Env)
    }

    /// A stored key is shown as 「已填 · 末4位」 (unless marked for deletion).
    pub fn shows_stored_key(&self) -> bool {
        self.key.shows_stored() && self.provider == Some(Provider::Brave)
    }

    pub fn pick(&mut self, p: Provider) {
        self.provider = Some(p);
        self.note = None;
    }

    pub fn clear_key(&mut self) {
        self.key.clear();
    }

    pub fn cancel_key(&mut self) {
        self.key.cancel();
    }

    /// Validate and build the PUT, or set an inline note.
    pub fn to_send(&mut self) -> Option<Put> {
        let provider = self.provider?;
        if self.key.clearing {
            self.note = None;
            return Some(Put::clear());
        }
        let put = match provider {
            Provider::Brave => {
                let key = match self.key.to_send() {
                    Some(Some(k)) => Some(Some(k)),
                    None if self.key.has_key => None,
                    Some(None) => Some(None),
                    None => {
                        self.note = Some(NEED_KEY.into());
                        return None;
                    }
                };
                Put {
                    provider,
                    url: None,
                    key,
                    clear: false,
                }
            }
            Provider::Searxng => {
                let url = self.url.trim().to_string();
                if url.is_empty() {
                    self.note = Some(NEED_URL.into());
                    return None;
                }
                Put {
                    provider,
                    url: Some(url),
                    key: None,
                    clear: false,
                }
            }
        };
        self.note = None;
        Some(put)
    }

    /// PUT answered: show what the host now has (page settings win),
    /// then the caller runs one test.
    pub fn saved(&mut self, cur: Current) {
        self.busy = false;
        self.load(cur);
        self.test = Some(Test::Running);
    }

    pub fn save_failed(&mut self, reason: &str) {
        self.busy = false;
        self.note = Some(failed(reason));
    }

    pub fn tested(&mut self, ok: bool, error: Option<String>) {
        self.test = Some(if ok {
            Test::Ok
        } else {
            Test::Failed(error.unwrap_or_default())
        });
    }

    pub fn test_line(&self) -> Option<String> {
        self.test.as_ref().map(|t| match t {
            Test::Running => TESTING.to_string(),
            Test::Ok => WORKS.to_string(),
            Test::Failed(r) => saved_but_unreachable(r),
        })
    }

    pub fn can_save(&self) -> bool {
        !self.busy && self.provider.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_brave() -> Current {
        Current {
            provider: Some(Provider::Brave),
            url: None,
            has_key: true,
            key_last4: Some("XXXX".into()),
            source: Some(Source::Env),
        }
    }

    #[test]
    fn env_label_only_when_source_is_env() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut p = Page::default();
        p.load(env_brave());
        assert!(p.from_env());
        assert!(p.shows_stored_key());
        assert_eq!(key_filled(Some("XXXX")), "已填 · XXXX");

        let mut absent = env_brave();
        absent.source = None;
        p.load(absent);
        assert!(!p.from_env(), "absent source: no label, no guess");

        assert_eq!(Source::parse(Some("weird")), None);
        assert_eq!(Source::parse(None), None);
    }

    #[test]
    fn after_save_shows_page_config() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut p = Page::default();
        p.load(env_brave());
        p.pick(Provider::Searxng);
        p.url = "http://127.0.0.1:8888".into();
        let put = p.to_send().unwrap();
        assert_eq!(put.url.as_deref(), Some("http://127.0.0.1:8888"));
        p.saved(Current {
            provider: Some(Provider::Searxng),
            url: Some("http://127.0.0.1:8888".into()),
            has_key: false,
            key_last4: None,
            source: Some(Source::Settings),
        });
        assert!(!p.from_env());
        assert_eq!(p.provider, Some(Provider::Searxng));
        assert_eq!(p.test_line().as_deref(), Some(TESTING.as_str()));
        p.tested(true, None);
        assert_eq!(p.test_line().as_deref(), Some(WORKS.as_str()));
        p.tested(false, Some("超时".into()));
        assert_eq!(p.test_line().as_deref(), Some("已保存，但连不上：超时"));
    }

    #[test]
    fn connect_copy_says_whether_it_was_saved() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        assert_eq!(
            strip_connect_prefix("连不上：io: Connection refused"),
            "io: Connection refused"
        );
        assert_eq!(strip_connect_prefix("超时"), "超时");
        assert_eq!(
            saved_but_unreachable("连不上：io: Connection refused (os error 111)"),
            "已保存，但连不上：io: Connection refused (os error 111)"
        );
        assert_eq!(saved_but_unreachable("超时"), "已保存，但连不上：超时");
        assert_eq!(
            failed("连不上：io: Connection refused (os error 111)"),
            "没保存：连不上：io: Connection refused (os error 111)"
        );
        assert_eq!(
            failed("连不上宿主机 http://127.0.0.1:1: io"),
            "没保存：连不上宿主机 http://127.0.0.1:1: io"
        );
        assert_eq!(failed("请求体无效"), "没保存：请求体无效");
        assert_eq!(cannot_connect("连不上：io: refused"), "连不上：io: refused");
        assert_eq!(cannot_connect("超时"), "连不上：超时");
    }

    #[test]
    fn key_rules() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut p = Page::default();
        p.pick(Provider::Brave);
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(NEED_KEY.as_str()));
        p.key.draft = "abcd1234".into();
        assert_eq!(p.to_send().unwrap().key, Some(Some("abcd1234".into())));

        p.load(env_brave());
        assert_eq!(p.to_send().unwrap().key, None, "stored key: leave as-is");
        p.clear_key();
        assert!(!p.shows_stored_key());
        assert_eq!(PENDING_CLEAR, "保存后删除");
        assert_eq!(UNDO, "撤销");
        assert!(p.to_send().unwrap().clear);
        p.cancel_key();
        assert!(p.shows_stored_key());
        assert_eq!(p.to_send().unwrap().key, None);
    }

    #[test]
    fn searxng_needs_url() {
        let mut p = Page::default();
        p.pick(Provider::Searxng);
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(NEED_URL.as_str()));
    }
}
