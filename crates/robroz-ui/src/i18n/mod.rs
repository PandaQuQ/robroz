//! Client strings in two locales: English (the default) and Simplified
//! Chinese.
//!
//! Every user-facing string in the client is a [`Key`]. [`en`] and [`zh`]
//! each hold one `match` over every key, so the compiler refuses a key that
//! is missing from either locale. English copy lives only in `en.rs`; the
//! comment above each entry is the Chinese source it was written from.
//!
//! Text with values in it uses numbered placeholders (`{0}`, `{1}`), filled
//! by [`Key::fill`]. Both locales of a key use the same placeholders.
//!
//! Strings the host sends (contract constants such as
//! [`robroz_contract::APPROVAL_SENT_TO_OWNER_DM_NOTICE`]) are mapped by
//! [`host_text`]: a known host string comes back in the active locale, and
//! anything else passes through unchanged.

use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

mod en;
mod host;
mod keys;
mod zh;

pub use host::{host_text, host_text_in};
pub use keys::Key;

/// A UI language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Locale {
    /// English, the default.
    #[default]
    En,
    /// Simplified Chinese.
    Zh,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::En, Locale::Zh];

    /// Short code used in settings and `ROBROZ_LANG`: `en` or `zh`.
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Zh => "zh",
        }
    }

    /// Parse a code (`en`, `zh`, `zh-CN`, `en_GB.UTF-8`, ...). Unknown
    /// values give `None`.
    pub fn parse(value: &str) -> Option<Locale> {
        let v = value.trim().to_ascii_lowercase();
        if v == "en" || v.starts_with("en-") || v.starts_with("en_") || v.starts_with("en.") {
            Some(Locale::En)
        } else if v == "zh" || v.starts_with("zh-") || v.starts_with("zh_") || v.starts_with("zh.")
        {
            Some(Locale::Zh)
        } else {
            None
        }
    }

    /// The language's own name, for the Settings row: 「English」 /
    /// 「中文」, the same in both locales (`LocaleName*` in `en.rs`/`zh.rs`).
    pub fn native_name(self) -> &'static str {
        match self {
            Locale::En => Key::LocaleNameEn.as_str(),
            Locale::Zh => Key::LocaleNameZh.as_str(),
        }
    }

    /// The other locale.
    pub fn other(self) -> Locale {
        match self {
            Locale::En => Locale::Zh,
            Locale::Zh => Locale::En,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Locale::En => 0,
            Locale::Zh => 1,
        }
    }

    fn from_u8(v: u8) -> Locale {
        if v == 1 {
            Locale::Zh
        } else {
            Locale::En
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    /// Per-thread override, for tests that check one locale's copy.
    static SCOPED: Cell<Option<Locale>> = const { Cell::new(None) };
}

/// The active locale: this thread's [`scoped`] override, else the one the
/// app set with [`set_locale`], else English.
pub fn locale() -> Locale {
    SCOPED
        .with(Cell::get)
        .unwrap_or_else(|| Locale::from_u8(ACTIVE.load(Ordering::Relaxed)))
}

/// Switch the whole app to `locale`. The next redraw uses it.
pub fn set_locale(locale: Locale) {
    ACTIVE.store(locale.to_u8(), Ordering::Relaxed);
}

/// Guard from [`scoped`]; puts the previous override back on drop.
#[must_use = "the locale only applies while the guard is alive"]
pub struct Scoped(Option<Locale>);

impl Drop for Scoped {
    fn drop(&mut self) {
        SCOPED.with(|s| s.set(self.0));
    }
}

/// Use `locale` on this thread only until the guard drops. Tests use it so
/// parallel tests never see each other's locale.
pub fn scoped(locale: Locale) -> Scoped {
    Scoped(SCOPED.with(|s| s.replace(Some(locale))))
}

/// `key` in `locale`, whatever the active locale is.
pub fn text_in(locale: Locale, key: Key) -> &'static str {
    match locale {
        Locale::En => en::text(key),
        Locale::Zh => zh::text(key),
    }
}

/// `key` in the active locale.
pub fn t(key: Key) -> &'static str {
    text_in(locale(), key)
}

/// Replace `{0}`, `{1}`, ... in `template` with `args`.
fn fill_template(template: &str, args: &[&dyn fmt::Display]) -> String {
    // Parse the template, not the result: an argument containing `{1}` is
    // user data and must never be interpreted as another placeholder.
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('}') else {
            break;
        };
        if let Some(arg) = rest[1..end].parse::<usize>().ok().and_then(|i| args.get(i)) {
            out.push_str(&arg.to_string());
        } else {
            out.push_str(&rest[..=end]);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

impl Key {
    /// This key in the active locale.
    pub fn as_str(self) -> &'static str {
        t(self)
    }

    /// This key in the active locale with `{0}`, `{1}`, ... filled in.
    pub fn fill(self, args: &[&dyn fmt::Display]) -> String {
        fill_template(t(self), args)
    }

    /// Like [`Key::fill`], in a given locale.
    pub fn fill_in(self, locale: Locale, args: &[&dyn fmt::Display]) -> String {
        fill_template(text_in(locale, self), args)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::ops::Deref for Key {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Key {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<Key> for String {
    fn from(key: Key) -> String {
        key.as_str().to_string()
    }
}

impl From<Key> for Cow<'static, str> {
    fn from(key: Key) -> Self {
        Cow::Borrowed(key.as_str())
    }
}

impl<'a> iced::widget::text::IntoFragment<'a> for Key {
    fn into_fragment(self) -> iced::widget::text::Fragment<'a> {
        Cow::Borrowed(self.as_str())
    }
}

impl PartialEq<Key> for str {
    fn eq(&self, other: &Key) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Key> for &str {
    fn eq(&self, other: &Key) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<Key> for String {
    fn eq(&self, other: &Key) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<&str> for Key {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for Key {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<String> for Key {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

/// A local calendar date without the year: 「11月30日」 / "Nov 30".
pub fn month_day(month: u32, day: u32) -> String {
    const MONTHS: [Key; 12] = [
        Key::DateMonth1,
        Key::DateMonth2,
        Key::DateMonth3,
        Key::DateMonth4,
        Key::DateMonth5,
        Key::DateMonth6,
        Key::DateMonth7,
        Key::DateMonth8,
        Key::DateMonth9,
        Key::DateMonth10,
        Key::DateMonth11,
        Key::DateMonth12,
    ];
    let m = month.clamp(1, 12) as usize - 1;
    Key::DateMonthDay.fill(&[&MONTHS[m], &day])
}

#[cfg(test)]
mod tests;
