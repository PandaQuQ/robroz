//! 联网搜索 settings page (robroz#76 `/api/settings/search`).
//!
//! Pick Brave (key) or SearXNG (address). The key box is masked and never
//! filled from the host: after a save it only shows 「已填 · 末4位」 with
//! 「换一个」 and 「清除」. Every save is followed by one test call, shown in
//! place as 「能用了」 or 「连不上：原因」.
//!
//! `source` is optional on the host: 「来自环境变量」 shows only when it is
//! `env`. When it is absent nothing is shown and nothing is guessed.

pub const TITLE: &str = "联网搜索";
pub const BRAVE: &str = "Brave";
pub const SEARXNG: &str = "SearXNG";
pub const KEY_PLACEHOLDER: &str = "API 密钥";
pub const URL_PLACEHOLDER: &str = "SearXNG 地址，如 http://127.0.0.1:8888";
pub const REPLACE: &str = "换一个";
pub const CLEAR: &str = "清除";
pub const SAVE: &str = "保存";
pub const CANCEL: &str = "取消";
pub const FROM_ENV: &str = "来自环境变量";
pub const NOT_SET: &str = "还没设置联网搜索";
pub const TESTING: &str = "试连中…";
pub const WORKS: &str = "能用了";
pub const NEED_KEY: &str = "请填写 key";
pub const NEED_URL: &str = "请填写地址";
/// Delivery entry when a task needed search (`needsSetup: "search"`).
pub const GO_SETUP: &str = "去设置";

pub fn key_filled(last4: Option<&str>) -> String {
    match last4 {
        Some(l) => format!("已填 · {l}"),
        None => "已填".into(),
    }
}

/// Drop a leading 「连不上：」 so a later prefix is not doubled.
pub fn strip_connect_prefix(reason: &str) -> &str {
    reason.strip_prefix("连不上：").unwrap_or(reason)
}

pub fn cannot_connect(reason: &str) -> String {
    format!("连不上：{}", strip_connect_prefix(reason))
}

/// PUT succeeded, then the test failed.
pub fn saved_but_unreachable(reason: &str) -> String {
    format!("已保存，但连不上：{}", strip_connect_prefix(reason))
}

/// The save itself failed. Keep a host 「连不上：」 rather than doubling it.
pub fn failed(reason: &str) -> String {
    format!("没保存：{reason}")
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
    /// Typed key (masked). Only used while `replacing` or no key is stored.
    pub key: String,
    pub replacing: bool,
    /// 清除 pressed: next send wipes provider, url and key.
    pub clearing: bool,
    pub busy: bool,
    pub note: Option<String>,
    pub test: Option<Test>,
}

impl Page {
    /// Fresh data from the host: the form shows what is in effect.
    pub fn load(&mut self, cur: Current) {
        self.provider = cur.provider;
        self.url = cur.url.clone().unwrap_or_default();
        self.key.clear();
        self.replacing = false;
        self.clearing = false;
        self.current = Some(cur);
    }

    /// 「来自环境变量」 only when the host says `env`.
    pub fn from_env(&self) -> bool {
        self.current.as_ref().and_then(|c| c.source) == Some(Source::Env)
    }

    /// A stored key is shown as 「已填 · 末4位」 (unless being replaced or cleared).
    pub fn shows_stored_key(&self) -> bool {
        !self.replacing
            && !self.clearing
            && self.current.as_ref().is_some_and(|c| c.has_key)
            && self.provider == Some(Provider::Brave)
    }

    pub fn pick(&mut self, p: Provider) {
        self.provider = Some(p);
        self.note = None;
    }

    pub fn replace_key(&mut self) {
        self.replacing = true;
        self.clearing = false;
        self.key.clear();
    }

    pub fn clear_key(&mut self) {
        self.clearing = true;
        self.replacing = false;
        self.key.clear();
    }

    pub fn cancel_key(&mut self) {
        self.replacing = false;
        self.clearing = false;
        self.key.clear();
    }

    /// Validate and build the PUT, or set an inline note.
    pub fn to_send(&mut self) -> Option<Put> {
        let Some(provider) = self.provider else {
            return None;
        };
        if self.clearing {
            self.note = None;
            return Some(Put::clear());
        }
        let key = self.key.trim().to_string();
        let put = match provider {
            Provider::Brave => {
                let stored = self.current.as_ref().is_some_and(|c| c.has_key);
                let key = if !key.is_empty() {
                    Some(Some(key))
                } else if stored {
                    None
                } else {
                    self.note = Some(NEED_KEY.into());
                    return None;
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
        assert_eq!(p.test_line().as_deref(), Some(TESTING));
        p.tested(true, None);
        assert_eq!(p.test_line().as_deref(), Some(WORKS));
        p.tested(false, Some("超时".into()));
        assert_eq!(p.test_line().as_deref(), Some("已保存，但连不上：超时"));
    }

    #[test]
    fn connect_copy_says_whether_it_was_saved() {
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
        let mut p = Page::default();
        p.pick(Provider::Brave);
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(NEED_KEY));
        p.key = "abcd1234".into();
        assert_eq!(p.to_send().unwrap().key, Some(Some("abcd1234".into())));

        p.load(env_brave());
        assert_eq!(p.to_send().unwrap().key, None, "stored key: leave as-is");
        p.clear_key();
        assert!(!p.shows_stored_key());
        assert!(p.to_send().unwrap().clear);
        p.cancel_key();
        p.replace_key();
        p.key = "newkey99".into();
        assert_eq!(p.to_send().unwrap().key, Some(Some("newkey99".into())));
    }

    #[test]
    fn searxng_needs_url() {
        let mut p = Page::default();
        p.pick(Provider::Searxng);
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(NEED_URL));
    }
}
