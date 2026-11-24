//! Masked secret input. The host never sends the value back: after a save
//! the field only shows 「已填 · 末4位」, with a single 「清除」 action.
//! Clicking it marks the key to be deleted on the next save, and the row
//! shows grey 「保存后删除」 plus 「撤销」.
//!
//! Shared by the 联网搜索 settings page and MCP secret rows.

use crate::i18n::Key;
/// Shown next to a stored key, with the last four characters when known.
pub fn filled(last4: Option<&str>) -> String {
    match last4 {
        Some(l) if !l.is_empty() => format!("{} · {l}", Key::SecretFilled),
        _ => Key::SecretFilled.into(),
    }
}

pub const CLEAR: Key = Key::SecretClear;
pub const PENDING_CLEAR: Key = Key::SecretPendingClear;
pub const UNDO: Key = Key::SecretUndo;

/// Local state for one secret. `draft` is only the text just typed; a
/// stored key is never copied into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaskedKey {
    pub has_key: bool,
    pub last4: Option<String>,
    pub draft: String,
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
        self.has_key && !self.clearing
    }

    pub fn clear(&mut self) {
        self.clearing = true;
        self.draft.clear();
    }

    pub fn cancel(&mut self) {
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
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        assert_eq!(filled(Some("WXYZ")), "已填 · WXYZ");
        assert_eq!(filled(Some("")), "已填");
        assert_eq!(filled(None), "已填");
    }

    #[test]
    fn clear_is_pending_until_cancelled_or_sent() {
        let _zh = crate::i18n::scoped(crate::i18n::Locale::Zh);
        let mut k = MaskedKey::from_stored(true, Some("WXYZ".into()));
        assert!(k.shows_stored());
        assert_eq!(k.to_send(), None);
        k.clear();
        assert!(!k.shows_stored());
        assert_eq!(k.to_send(), Some(None));
        k.cancel();
        assert!(k.shows_stored());
        assert_eq!(k.to_send(), None);
        assert_eq!(CLEAR, "清除");
        assert_eq!(PENDING_CLEAR, "保存后删除");
        assert_eq!(UNDO, "撤销");
    }

    #[test]
    fn typed_key_is_sent_when_not_clearing() {
        let mut k = MaskedKey::default();
        k.draft = "  new-key  ".into();
        assert_eq!(k.to_send(), Some(Some("new-key".into())));
        k.clear();
        assert_eq!(k.to_send(), Some(None));
    }
}
