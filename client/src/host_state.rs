//! Whether the host has answered yet, and what the window shows until it
//! has. The empty-account state (「还没有 bro」 in the chat list, 「选一个聊天」
//! in the main pane) only makes sense once the host has listed the groups;
//! before that the sidebar stays empty.

use robroz_ui::i18n::Key as Text;

/// Where the first group listing stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    /// The first listing is still on its way.
    Waiting,
    /// The first listing failed: the host can't be reached.
    Unreachable,
    /// The host listed the groups at least once.
    Answered,
}

impl HostState {
    /// `loaded`: the host listed the groups at least once. `load_failed`:
    /// the latest attempt at that first listing failed.
    pub fn of(loaded: bool, load_failed: bool) -> Self {
        match (loaded, load_failed) {
            (true, _) => Self::Answered,
            (false, true) => Self::Unreachable,
            (false, false) => Self::Waiting,
        }
    }

    /// The chat list and page buttons show only once the host answered.
    pub fn shows_sidebar(self) -> bool {
        self == Self::Answered
    }

    /// The main pane's message before the host answered: the headline, with
    /// the host address on a lighter line below it. `None` once answered,
    /// and while still waiting (the pane stays blank).
    pub fn notice(self) -> Option<Text> {
        (self == Self::Unreachable).then_some(Text::ClientHostUnreachable)
    }

    /// Retry the first listing on the poll interval while unreachable.
    pub fn retries(self) -> bool {
        self == Self::Unreachable
    }
}

#[cfg(test)]
mod tests {
    use super::HostState;
    use robroz_ui::i18n::{scoped, text_in, Key as Text, Locale};

    #[test]
    fn empty_account_waits_for_the_host_to_answer() {
        let waiting = HostState::of(false, false);
        assert_eq!(waiting, HostState::Waiting);
        assert!(!waiting.shows_sidebar());
        assert_eq!(waiting.notice(), None);

        let down = HostState::of(false, true);
        assert_eq!(down, HostState::Unreachable);
        assert!(!down.shows_sidebar(), "no 「还没有 bro」 while unreachable");
        assert_eq!(down.notice(), Some(Text::ClientHostUnreachable));
        assert!(down.retries());

        // Once the host answered, a later failure keeps the account on
        // screen; the footer carries the error.
        for failed in [false, true] {
            let up = HostState::of(true, failed);
            assert_eq!(up, HostState::Answered);
            assert!(up.shows_sidebar());
            assert_eq!(up.notice(), None);
            assert!(!up.retries());
        }
    }

    #[test]
    fn unreachable_copy_in_both_locales() {
        assert_eq!(
            text_in(Locale::En, Text::ClientHostUnreachable),
            "Can't reach the host"
        );
        assert_eq!(
            text_in(Locale::Zh, Text::ClientHostUnreachable),
            "连不上宿主"
        );
        let _en = scoped(Locale::En);
        assert_eq!(
            HostState::Unreachable.notice().map(|k| k.as_str()),
            Some("Can't reach the host")
        );
    }
}
