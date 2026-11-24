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

/// Short reason shown next to the host address on the disconnected pane.
/// Maps common transport errors from ureq / `std::io`; anything else is
/// omitted so the pane never shows `io:` or `os error`.
pub fn connect_reason(err: &str) -> Option<Text> {
    let s = err.to_ascii_lowercase();
    if s.contains("connection refused") {
        Some(Text::ClientConnectionRefused)
    } else if s.contains("timed out") || s.contains("timed-out") || s.contains("timeout") {
        Some(Text::ClientTimedOut)
    } else if is_host_not_found(&s) {
        Some(Text::ClientHostNotFound)
    } else {
        None
    }
}

fn is_host_not_found(s: &str) -> bool {
    s.contains("host not found")
        || s.contains("failed to lookup")
        || s.contains("name or service not known")
        || s.contains("nodename nor servname")
        || s.contains("temporary failure in name resolution")
        || s.contains("no such host")
        || s.contains("dns error")
        || s.contains("could not resolve")
        || s.contains("failed to resolve")
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

    #[test]
    fn connect_reason_maps_common_io_errors() {
        use super::connect_reason;

        let refused =
            "Can't reach the host http://127.0.0.1:1: io: Connection refused (os error 111)";
        assert_eq!(connect_reason(refused), Some(Text::ClientConnectionRefused));
        assert_eq!(
            connect_reason("io: Connection refused (os error 10061)"),
            Some(Text::ClientConnectionRefused)
        );
        assert_eq!(
            text_in(Locale::En, Text::ClientConnectionRefused),
            "Connection refused"
        );
        assert_eq!(
            text_in(Locale::Zh, Text::ClientConnectionRefused),
            "连接被拒绝"
        );

        assert_eq!(
            connect_reason("io: Connection timed out (os error 110)"),
            Some(Text::ClientTimedOut)
        );
        assert_eq!(
            connect_reason("timeout: Global"),
            Some(Text::ClientTimedOut)
        );
        assert_eq!(text_in(Locale::En, Text::ClientTimedOut), "Timed out");
        assert_eq!(text_in(Locale::Zh, Text::ClientTimedOut), "超时");

        assert_eq!(
            connect_reason("host not found"),
            Some(Text::ClientHostNotFound)
        );
        assert_eq!(
            connect_reason("io: failed to lookup address information: Name or service not known"),
            Some(Text::ClientHostNotFound)
        );
        assert_eq!(
            connect_reason("nodename nor servname provided, or not known"),
            Some(Text::ClientHostNotFound)
        );
        assert_eq!(
            text_in(Locale::En, Text::ClientHostNotFound),
            "Host not found"
        );
        assert_eq!(text_in(Locale::Zh, Text::ClientHostNotFound), "找不到主机");

        // Unknown transport errors are omitted, never echoed raw.
        for s in [
            "io: UnexpectedEof (os error 0)",
            "Can't reach the host http://127.0.0.1:1: io: Broken pipe (os error 32)",
            "http status: 500",
        ] {
            assert_eq!(connect_reason(s), None);
        }
        for key in [
            Text::ClientConnectionRefused,
            Text::ClientTimedOut,
            Text::ClientHostNotFound,
        ] {
            let en = text_in(Locale::En, key);
            let zh = text_in(Locale::Zh, key);
            assert!(!en.contains("io:"), "{en}");
            assert!(!en.to_ascii_lowercase().contains("os error"), "{en}");
            assert!(!zh.contains("io:"), "{zh}");
            assert!(!zh.contains("os error"), "{zh}");
        }
    }
}
