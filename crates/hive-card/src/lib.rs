use hive_contract::TaskState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Gray,
    Ink,
    Green,
    Amber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub tone: Tone,
    pub action: Option<&'static str>,
}

/// `container_up` is ignored except for a live `running` card.
/// A dead run stays `Running` and uses the same gray as queued.
pub fn appearance(state: TaskState, container_up: bool) -> Appearance {
    match state {
        TaskState::Queued => Appearance {
            tone: Tone::Gray,
            action: None,
        },
        TaskState::Running if container_up => Appearance {
            tone: Tone::Ink,
            action: None,
        },
        TaskState::Running => Appearance {
            tone: Tone::Gray,
            action: None,
        },
        TaskState::Done => Appearance {
            tone: Tone::Green,
            action: None,
        },
        TaskState::Blocked => Appearance {
            tone: Tone::Amber,
            action: Some("去私聊接管"),
        },
    }
}

pub fn briefing_badge() -> &'static str {
    "简报"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_for_each_state() {
        assert_eq!(
            appearance(TaskState::Queued, false),
            Appearance { tone: Tone::Gray, action: None }
        );
        assert_eq!(
            appearance(TaskState::Queued, true),
            Appearance { tone: Tone::Gray, action: None },
            "queued ignores container_up"
        );
        assert_eq!(
            appearance(TaskState::Running, true),
            Appearance { tone: Tone::Ink, action: None }
        );
        assert_eq!(
            appearance(TaskState::Running, false),
            Appearance { tone: Tone::Gray, action: None }
        );
        assert_eq!(
            appearance(TaskState::Done, false),
            Appearance { tone: Tone::Green, action: None }
        );
        assert_eq!(
            appearance(TaskState::Blocked, false),
            Appearance { tone: Tone::Amber, action: Some("去私聊接管") }
        );
    }

    #[test]
    fn briefing_badge_text() {
        assert_eq!(briefing_badge(), "简报");
    }
}
