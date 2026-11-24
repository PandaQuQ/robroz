//! 模型 settings page (`GET`/`PUT /api/settings/model`, `POST …/test`).
//!
//! The host has no provider list. The dropdown only fills a known OpenAI-
//! compatible base URL; the user still types the model name and key. The
//! key box is the shared [`super::secret::MaskedKey`]: after a save it
//! shows 「已填 · 末4位」, and 「清除」 marks the key for deletion until
//! the next save (「保存后删除」 / 「撤销」). Every save is followed by one
//! test call, shown as 「能用了」 or 「已保存，但连不上：原因」.

use super::secret::{self, MaskedKey};
use crate::i18n::Key;

pub const TITLE: Key = Key::ModelTitle;
pub const SAVE: Key = Key::ModelSave;
pub const FROM_ENV: Key = Key::ModelFromEnv;
pub const NOT_SET: Key = Key::ModelNotSet;
pub const TESTING: Key = Key::ModelTesting;
pub const WORKS: Key = Key::ModelWorks;
pub const LABEL_PRESET: Key = Key::ModelLabelPreset;
/// Empty value of the provider dropdown.
pub const PRESET_PLACEHOLDER: Key = Key::ModelPresetPlaceholder;
pub const LABEL_URL: Key = Key::ModelLabelUrl;
pub const LABEL_MODEL: Key = Key::ModelLabelModel;
pub const LABEL_KEY: Key = Key::ModelLabelKey;
/// Address placeholder is empty so it does not repeat the label.
pub const URL_PLACEHOLDER: &str = "";
/// Key placeholder is empty so it does not repeat the label.
pub const KEY_PLACEHOLDER: &str = "";
pub use secret::{CLEAR, PENDING_CLEAR, UNDO};
pub const NEED_URL: Key = Key::ModelNeedUrl;
pub const BAD_URL: Key = Key::ModelBadUrl;
pub const NEED_MODEL: Key = Key::ModelNeedModel;
/// Delivery entry when a task needed a model (`needsSetup: "model"`).
pub const GO_SETUP: Key = Key::ModelGoSetup;
/// Shown under the delivery path when `needsSetup` is `"model"` and there is no failure.
pub const NEEDS_SETUP_HINT: Key = Key::ModelNeedsSetupHint;
/// Hint under the OpenRouter preset.
pub const OPENROUTER_HINT: Key = Key::ModelOpenrouterHint;

/// Host `/test` returns this as `error` on 401/403. Show it after
/// 「已保存，但连不上：」, never a raw 401.
pub use robroz_contract::MODEL_TEST_BAD_KEY;

pub fn key_filled(last4: Option<&str>) -> String {
    secret::filled(last4)
}

/// Drop a leading 「连不上：」 (the host's, or this locale's) so a later
/// prefix is not doubled.
pub fn strip_connect_prefix(reason: &str) -> &str {
    let ours = Key::CannotConnect.as_str().trim_end_matches("{0}");
    reason
        .strip_prefix(
            Key::CannotConnect
                .fill_in(crate::i18n::Locale::Zh, &[&""])
                .as_str(),
        )
        .or_else(|| reason.strip_prefix(ours))
        .unwrap_or(reason)
}

pub fn cannot_connect(reason: &str) -> String {
    Key::CannotConnect.fill(&[&crate::i18n::host_text(strip_connect_prefix(reason))])
}

fn is_raw_auth_error(reason: &str) -> bool {
    let reason = reason.trim();
    let reason = reason
        .strip_prefix("HTTP ")
        .or_else(|| reason.strip_prefix("http "))
        .unwrap_or(reason);
    reason == "401"
        || reason == "403"
        || reason.eq_ignore_ascii_case("401 Unauthorized")
        || reason.eq_ignore_ascii_case("403 Forbidden")
}

pub fn saved_but_unreachable(reason: &str) -> String {
    let reason = strip_connect_prefix(reason);
    let reason = if reason == MODEL_TEST_BAD_KEY || is_raw_auth_error(reason) {
        MODEL_TEST_BAD_KEY
    } else {
        reason
    };
    Key::SavedButUnreachable.fill(&[&crate::i18n::host_text(reason)])
}

pub fn failed(reason: &str) -> String {
    Key::NotSaved.fill(&[&crate::i18n::host_text(reason)])
}

/// Frontend-only shortcuts. The host does not know these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// First, and the recommended free OpenAI-compatible option.
    Gemini,
    Groq,
    OpenRouter,
    Ollama,
    Custom,
}

impl Preset {
    pub const ALL: &'static [Preset] = &[
        Preset::Gemini,
        Preset::Groq,
        Preset::OpenRouter,
        Preset::Ollama,
        Preset::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Gemini => Key::ModelPresetGemini.as_str(),
            Preset::Groq => "Groq",
            Preset::OpenRouter => "OpenRouter",
            Preset::Ollama => "Ollama",
            Preset::Custom => Key::ModelPresetCustom.as_str(),
        }
    }

    /// Canonical base URL, no trailing slash (the host strips it on save).
    pub fn base_url(self) -> Option<&'static str> {
        match self {
            Preset::Gemini => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            Preset::Groq => Some("https://api.groq.com/openai/v1"),
            Preset::OpenRouter => Some("https://openrouter.ai/api/v1"),
            Preset::Ollama => Some("http://localhost:11434/v1"),
            Preset::Custom => None,
        }
    }

    /// Example model id used as the placeholder. Empty for custom / no pick.
    pub fn model_example(self) -> &'static str {
        match self {
            Preset::Gemini => "gemini-3.8-flash",
            Preset::Groq => "llama-3.3-70b-versatile",
            Preset::OpenRouter => "meta-llama/llama-3.3-70b-instruct:free",
            Preset::Ollama => "llama3.2",
            Preset::Custom => "",
        }
    }

    /// Whether the key field is hidden. Currently the field stays visible
    /// for every preset, including Ollama. Flip the body to
    /// `self == Preset::Ollama` if PM later wants it hidden.
    pub fn hides_key(self) -> bool {
        let _ = self;
        false
    }

    pub fn from_url(url: &str) -> Self {
        let url = normalize_url(url);
        for p in [
            Preset::Gemini,
            Preset::Groq,
            Preset::OpenRouter,
            Preset::Ollama,
        ] {
            if p.base_url() == Some(url) {
                return p;
            }
        }
        if url.is_empty() {
            // Empty box: keep Gemini as the starting suggestion only after a pick.
            Preset::Custom
        } else {
            Preset::Custom
        }
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Trim whitespace and a trailing `/`, like the host does on PUT.
pub fn normalize_url(url: &str) -> &str {
    url.trim().trim_end_matches('/')
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

/// What the host returned (`GET`/`PUT`); the key is never included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Current {
    pub base_url: Option<String>,
    pub model: Option<String>,
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

/// What 保存 sends. Each field: `None` leave as-is, `Some(None)` clear,
/// `Some(Some(v))` set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Put {
    pub base_url: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub key: Option<Option<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub current: Option<Current>,
    pub preset: Option<Preset>,
    pub url: String,
    pub model: String,
    pub key: MaskedKey,
    pub busy: bool,
    pub note: Option<String>,
    pub test: Option<Test>,
}

impl Page {
    /// Fresh data from the host: the form shows what is in effect.
    pub fn load(&mut self, cur: Current) {
        self.url = cur.base_url.clone().unwrap_or_default();
        self.model = cur.model.clone().unwrap_or_default();
        self.key = MaskedKey::from_stored(cur.has_key, cur.key_last4.clone());
        self.preset = if self.url.trim().is_empty() {
            None
        } else {
            Some(Preset::from_url(&self.url))
        };
        self.current = Some(cur);
    }

    /// 「来自环境变量」 only when the host says `env`.
    pub fn from_env(&self) -> bool {
        self.current.as_ref().and_then(|c| c.source) == Some(Source::Env)
    }

    pub fn shows_stored_key(&self) -> bool {
        self.key.shows_stored()
    }

    /// Key field is shown unless the selected preset hides it.
    pub fn shows_key(&self) -> bool {
        !self.preset.is_some_and(Preset::hides_key)
    }

    pub fn model_placeholder(&self) -> &'static str {
        self.preset.map_or("", Preset::model_example)
    }

    pub fn not_set(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|c| c.base_url.is_none() && c.model.is_none() && !c.has_key)
            && self.preset.is_none()
            && self.url.trim().is_empty()
            && self.model.trim().is_empty()
            && !self.key.has_key
            && self.key.draft.trim().is_empty()
    }

    pub fn pick(&mut self, p: Preset) {
        self.preset = Some(p);
        if let Some(url) = p.base_url() {
            self.url = url.to_string();
        }
        self.note = None;
    }

    pub fn set_model(&mut self, text: String) {
        self.model = text;
        self.note = None;
    }

    pub fn set_url(&mut self, text: String) {
        self.url = text;
        if !self.url.trim().is_empty() {
            self.preset = Some(Preset::from_url(&self.url));
        }
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
        let url = normalize_url(&self.url).to_string();
        if url.is_empty() {
            self.note = Some(NEED_URL.into());
            return None;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.note = Some(BAD_URL.into());
            return None;
        }
        let model = self.model.trim().to_string();
        if model.is_empty() {
            self.note = Some(NEED_MODEL.into());
            return None;
        }
        let key = match self.key.to_send() {
            Some(Some(k)) => Some(Some(k)),
            Some(None) => Some(None),
            None => None,
        };
        self.note = None;
        Some(Put {
            base_url: Some(Some(url)),
            model: Some(Some(model)),
            key,
        })
    }

    /// PUT answered: show what the host now has, then the caller runs one test.
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
        !self.busy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_gemini() -> Current {
        Current {
            base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai".into()),
            model: Some("gemini-3.8-flash".into()),
            has_key: true,
            key_last4: Some("ABCD".into()),
            source: Some(Source::Env),
        }
    }

    #[test]
    fn gemini_is_first_and_recommended() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        assert_eq!(Preset::ALL[0], Preset::Gemini);
        assert_eq!(Preset::Gemini.label(), "Gemini（推荐）");
        assert_eq!(
            Preset::Gemini.base_url(),
            Some("https://generativelanguage.googleapis.com/v1beta/openai")
        );
        assert_eq!(
            Preset::from_url("https://generativelanguage.googleapis.com/v1beta/openai/"),
            Preset::Gemini
        );
        assert_eq!(
            Preset::from_url("https://api.groq.com/openai/v1"),
            Preset::Groq
        );
        assert_eq!(
            Preset::from_url("https://openrouter.ai/api/v1/"),
            Preset::OpenRouter
        );
        assert_eq!(
            Preset::from_url("http://localhost:11434/v1"),
            Preset::Ollama
        );
        assert_eq!(
            Preset::from_url("https://api.openai.com/v1"),
            Preset::Custom
        );
        assert!(URL_PLACEHOLDER.is_empty());
        assert!(KEY_PLACEHOLDER.is_empty());
        assert_ne!(LABEL_KEY, KEY_PLACEHOLDER);
        assert_eq!(LABEL_KEY, "密钥（选填）");
        assert_eq!(Preset::Gemini.model_example(), "gemini-3.8-flash");
        assert_eq!(Preset::Groq.model_example(), "llama-3.3-70b-versatile");
        assert_eq!(
            Preset::OpenRouter.model_example(),
            "meta-llama/llama-3.3-70b-instruct:free"
        );
        assert_eq!(Preset::Ollama.model_example(), "llama3.2");
        assert_eq!(Preset::Custom.model_example(), "");
        assert_eq!(OPENROUTER_HINT, "免费模型的名字以 :free 结尾");
        for p in Preset::ALL {
            assert!(!p.hides_key(), "key field stays visible, including Ollama");
        }
        assert_eq!(PENDING_CLEAR, "保存后删除");
        assert_eq!(UNDO, "撤销");
        assert_eq!(TITLE, "模型");
    }

    #[test]
    fn picking_a_preset_fills_the_url_only() {
        let mut p = Page::default();
        p.model = "already".into();
        p.pick(Preset::Groq);
        assert_eq!(p.url, "https://api.groq.com/openai/v1");
        assert_eq!(p.model, "already");
        p.pick(Preset::Custom);
        assert_eq!(
            p.url, "https://api.groq.com/openai/v1",
            "custom keeps the url"
        );
        p.set_url("https://example.com/v1/".into());
        assert_eq!(p.preset, Some(Preset::Custom));
        p.set_url("https://api.groq.com/openai/v1/".into());
        assert_eq!(p.preset, Some(Preset::Groq));
    }

    #[test]
    fn env_label_only_when_source_is_env() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut p = Page::default();
        p.load(env_gemini());
        assert!(p.from_env());
        assert!(p.shows_stored_key());
        assert_eq!(p.preset, Some(Preset::Gemini));
        assert_eq!(key_filled(Some("ABCD")), "已填 · ABCD");

        let mut absent = env_gemini();
        absent.source = None;
        p.load(absent);
        assert!(!p.from_env(), "absent source: no label, no guess");
        assert_eq!(Source::parse(Some("weird")), None);
        assert_eq!(Source::parse(None), None);
    }

    #[test]
    fn save_then_test_and_pending_key_clear() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut p = Page::default();
        p.pick(Preset::Ollama);
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(NEED_MODEL.as_str()));
        p.set_model("llama3.2".into());
        assert_eq!(p.note.take(), None);
        let without_key = p.to_send().unwrap();
        assert_eq!(
            without_key.base_url,
            Some(Some("http://localhost:11434/v1".into()))
        );
        assert_eq!(without_key.model, Some(Some("llama3.2".into())));
        assert_eq!(without_key.key, None, "key is optional");
        assert!(p.shows_key(), "Ollama still shows the key field");
        p.key.draft = " sk-test ".into();
        let put = p.to_send().unwrap();
        assert_eq!(put.base_url, Some(Some("http://localhost:11434/v1".into())));
        assert_eq!(put.model, Some(Some("llama3.2".into())));
        assert_eq!(put.key, Some(Some("sk-test".into())));

        p.saved(Current {
            base_url: Some("http://localhost:11434/v1".into()),
            model: Some("llama3.2".into()),
            has_key: true,
            key_last4: Some("test".into()),
            source: Some(Source::Settings),
        });
        assert!(!p.from_env());
        assert_eq!(p.test_line().as_deref(), Some(TESTING.as_str()));
        p.tested(true, None);
        assert_eq!(p.test_line().as_deref(), Some(WORKS.as_str()));
        p.tested(false, Some("超时".into()));
        assert_eq!(p.test_line().as_deref(), Some("已保存，但连不上：超时"));

        assert_eq!(p.to_send().unwrap().key, None, "stored key: leave as-is");
        p.clear_key();
        assert!(!p.shows_stored_key());
        assert_eq!(p.to_send().unwrap().key, Some(None));
        p.cancel_key();
        assert!(p.shows_stored_key());
        assert_eq!(p.to_send().unwrap().key, None);
    }

    #[test]
    fn bad_or_empty_url_is_rejected() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut p = Page::default();
        p.model = "m".into();
        p.key.draft = "k".into();
        p.url = "not-a-url".into();
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(BAD_URL.as_str()));
        p.url.clear();
        assert!(p.to_send().is_none());
        assert_eq!(p.note.as_deref(), Some(NEED_URL.as_str()));
        assert_eq!(
            saved_but_unreachable("连不上：io: refused"),
            "已保存，但连不上：io: refused"
        );
        assert_eq!(failed("请求体无效"), "没保存：请求体无效");
        assert_eq!(
            saved_but_unreachable(MODEL_TEST_BAD_KEY),
            "已保存，但连不上：密钥不对或没填"
        );
        assert_eq!(
            saved_but_unreachable("401"),
            "已保存，但连不上：密钥不对或没填"
        );
        assert_eq!(
            saved_but_unreachable("401 Unauthorized"),
            "已保存，但连不上：密钥不对或没填"
        );
    }

    #[test]
    fn empty_key_saves_and_ollama_keeps_the_key_field() {
        let mut p = Page::default();
        p.pick(Preset::Gemini);
        p.set_model("gemini-3.8-flash".into());
        let put = p.to_send().expect("url + model is enough");
        assert_eq!(put.key, None);
        assert!(p.shows_key());
        p.pick(Preset::Ollama);
        assert!(p.shows_key());
        assert!(!Preset::Ollama.hides_key());
        assert_eq!(p.model_placeholder(), "llama3.2");
    }
}
