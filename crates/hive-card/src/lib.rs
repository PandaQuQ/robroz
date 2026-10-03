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

pub fn appearance(state: TaskState) -> Appearance {
    match state {
        TaskState::Queued => Appearance {
            tone: Tone::Gray,
            action: None,
        },
        TaskState::Running => Appearance {
            tone: Tone::Ink,
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
            appearance(TaskState::Queued),
            Appearance { tone: Tone::Gray, action: None }
        );
        assert_eq!(
            appearance(TaskState::Running),
            Appearance { tone: Tone::Ink, action: None }
        );
        assert_eq!(
            appearance(TaskState::Done),
            Appearance { tone: Tone::Green, action: None }
        );
        assert_eq!(
            appearance(TaskState::Blocked),
            Appearance { tone: Tone::Amber, action: Some("去私聊接管") }
        );
    }

    #[test]
    fn briefing_badge_text() {
        assert_eq!(briefing_badge(), "简报");
    }
}
