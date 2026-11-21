//! Masked secret input. The host never sends the value back: after a save
//! the field only shows 「已填 · 末4位」, with 「换一个」 and 「清除」.
//!
//! Written here because the 联网搜索 settings page (which wants the same
//! control) is not on main yet. The two can be unified once that lands.

/// Shown next to a stored key, with the last four characters when known.
pub fn filled(last4: Option<&str>) -> String {
    match last4 {
        Some(l) if !l.is_empty() => format!("已填 · {l}"),
        _ => "已填".into(),
    }
}

pub const REPLACE: &str = "换一个";
pub const CLEAR: &str = "清除";

/// Local state for one secret. `draft` is only the text just typed; a
/// stored key is never copied into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaskedKey {
    pub has_key: bool,
    pub last4: Option<String>,
    pub draft: String,
    pub replacing: bool,
    pub clearing: bool,
}

impl MaskedKey {
    pub fn from_stored(has_key: bool, last4: Option<String>) -> Self {
        Self {
            has_key,
            last4,
            ..Self::default()
        }
    }

    /// Show the stored label instead of a text box.
    pub fn shows_stored(&self) -> bool {
        self.has_key && !self.replacing && !self.clearing
    }

    pub fn replace(&mut self) {
        self.replacing = true;
        self.clearing = false;
        self.draft.clear();
    }

    pub fn clear(&mut self) {
        self.clearing = true;
        self.replacing = false;
        self.draft.clear();
    }

    pub fn cancel(&mut self) {
        self.replacing = false;
        self.clearing = false;
        self.draft.clear();
    }

    /// `None` leave as-is, `Some(None)` clear, `Some(Some(k))` set.
    pub fn to_send(&self) -> Option<Option<String>> {
        if self.clearing {
            Some(None)
        } else {
            let typed = self.draft.trim();
            if typed.is_empty() {
                None
            } else {
                Some(Some(typed.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filled_label_uses_last4_when_present() {
        assert_eq!(filled(Some("WXYZ")), "已填 · WXYZ");
        assert_eq!(filled(Some("")), "已填");
        assert_eq!(filled(None), "已填");
    }

    #[test]
    fn stored_key_is_not_sent_until_replaced_or_cleared() {
        let mut k = MaskedKey::from_stored(true, Some("WXYZ".into()));
        assert!(k.shows_stored());
        assert_eq!(k.to_send(), None);
        k.replace();
        assert!(!k.shows_stored());
        assert_eq!(k.to_send(), None);
        k.draft = "  new-key  ".into();
        assert_eq!(k.to_send(), Some(Some("new-key".into())));
        k.clear();
        assert_eq!(k.to_send(), Some(None));
        k.cancel();
        assert!(k.shows_stored());
        assert_eq!(k.to_send(), None);
    }
}
