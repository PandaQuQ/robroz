//! Client view models for the phase-1 cards (UI SPEC §4–§8).
//!
//! `choice` and the routines list are built from the contract types added in
//! robroz#15 (`Event::Choice`, `ScheduleEntry`). `approval` and `draft` have
//! no contract event yet, so their shapes follow SPEC §8 and stay client-side
//! until the backend adds them. None of this touches the frozen status card.

pub mod interact;
pub mod routines;
pub mod status;

use robroz_contract::{ApprovalAction, ApprovalDecision, ChoiceOption, Event};

/// New card kinds, side by side with the frozen status and delivery cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Card {
    Choice(ChoiceCard),
    Approval(ApprovalCard),
    Draft(DraftCard),
}

impl Card {
    /// Key for in-place updates.
    pub fn id(&self) -> &str {
        match self {
            Card::Choice(c) => &c.id,
            Card::Approval(c) => &c.id,
            Card::Draft(c) => &c.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceCard {
    pub id: String,
    pub bot: String,
    pub prompt: String,
    /// 2..=4, in the order the bot gave them. `primary` is ignored on multi.
    pub options: Vec<ChoiceOption>,
    pub multi: bool,
    pub state: ChoiceState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceState {
    Open,
    Answered {
        selected: Vec<String>,
        /// ISO 8601 from the contract (`answeredAt`); shown as local HH:MM.
        at: Option<String>,
        /// True when the answer is known to have come from somewhere else
        /// (DM, another member). The contract has no such field yet, so
        /// cards built from events always say false.
        elsewhere: bool,
    },
    /// No contract state for this yet; kept so the view is ready for it.
    Expired,
}

impl ChoiceCard {
    /// Build from the `choice` event. Returns `None` for any other event.
    pub fn from_event(event: &Event) -> Option<Self> {
        let Event::Choice {
            id,
            bot_name,
            prompt,
            options,
            multi,
            choice,
            answered_at,
            ..
        } = event
        else {
            return None;
        };
        let state = match choice {
            Some(selected) => ChoiceState::Answered {
                selected: selected.clone(),
                at: answered_at.clone(),
                elsewhere: false,
            },
            None => ChoiceState::Open,
        };
        Some(Self {
            id: id.clone(),
            bot: bot_name.clone(),
            prompt: prompt.clone(),
            options: options.clone(),
            multi: *multi,
            state,
        })
    }

    pub fn label_of(&self, option_id: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.id == option_id)
            .map(|o| o.label.as_str())
    }

    /// The option that may get the orange button: the first `primary`, and
    /// only on single choice. Guarantees at most one per card.
    pub fn primary_option(&self) -> Option<&str> {
        if self.multi {
            return None;
        }
        self.options
            .iter()
            .find(|o| o.primary)
            .map(|o| o.id.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Delete,
    Send,
    Spend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCard {
    pub id: String,
    pub bot: String,
    pub kind: ActionKind,
    pub summary: String,
    /// Relative path, display only (monospace, not a link).
    pub path: Option<String>,
    pub state: ApprovalState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalState {
    Pending,
    Approved { by: String, at: i64 },
    Denied { by: String, at: i64 },
    NeedsOwner,
}

impl ApprovalCard {
    /// Build from the `approval` event. Returns `None` for any other event.
    /// The host has no "answered by" field yet, so an answer reads as 「我」.
    pub fn from_event(event: &Event) -> Option<Self> {
        let Event::Approval {
            id,
            bot_name,
            action,
            summary,
            path,
            decision,
            answered_at,
            ..
        } = event
        else {
            return None;
        };
        let at = answered_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map_or(0, |t| t.timestamp());
        let by = String::from("我");
        let state = match decision {
            None => ApprovalState::Pending,
            Some(ApprovalDecision::Approved) => ApprovalState::Approved { by, at },
            Some(ApprovalDecision::Denied) => ApprovalState::Denied { by, at },
        };
        let kind = match action {
            ApprovalAction::Delete => ActionKind::Delete,
            ApprovalAction::Send => ActionKind::Send,
            ApprovalAction::Spend => ActionKind::Spend,
        };
        Some(Self {
            id: id.clone(),
            bot: bot_name.clone(),
            kind,
            summary: summary.clone(),
            path: path.clone(),
            state,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftChannel {
    Email,
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftCard {
    pub id: String,
    pub bot: String,
    pub channel: DraftChannel,
    /// Address, display only (monospace, not a link).
    pub to: String,
    pub subject: Option<String>,
    pub body: String,
    pub state: DraftState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftState {
    Pending,
    Sent { by: String, at: i64 },
    Discarded { by: String, at: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn opt(id: &str, label: &str, primary: bool) -> ChoiceOption {
        ChoiceOption {
            id: id.into(),
            label: label.into(),
            primary,
        }
    }

    fn event(choice: Option<Vec<&str>>, multi: bool) -> Event {
        Event::Choice {
            id: "c1".into(),
            bot_id: "bot-doc".into(),
            bot_name: "文档哥".into(),
            prompt: "安装说明先写哪一种？".into(),
            options: vec![
                opt("docker", "Docker 一键", true),
                opt("cargo", "cargo install", false),
            ],
            multi,
            choice: choice.map(|v| v.into_iter().map(String::from).collect()),
            created_at: "2026-11-20T01:50:00.000Z".into(),
            answered_at: None,
        }
    }

    #[test]
    fn open_and_answered_from_contract() {
        let open = ChoiceCard::from_event(&event(None, false)).unwrap();
        assert_eq!(open.state, ChoiceState::Open);
        assert_eq!(open.bot, "文档哥");
        let done = ChoiceCard::from_event(&event(Some(vec!["cargo"]), false)).unwrap();
        assert!(matches!(
            done.state,
            ChoiceState::Answered { ref selected, elsewhere: false, .. } if selected == &["cargo"]
        ));
        assert_eq!(done.label_of("cargo"), Some("cargo install"));
    }

    #[test]
    fn primary_only_on_single_and_at_most_one() {
        let single = ChoiceCard::from_event(&event(None, false)).unwrap();
        assert_eq!(single.primary_option(), Some("docker"));
        let multi = ChoiceCard::from_event(&event(None, true)).unwrap();
        assert_eq!(multi.primary_option(), None);
        let mut two = single.clone();
        two.options[1].primary = true;
        assert_eq!(two.primary_option(), Some("docker"), "first primary wins");
    }

    #[test]
    fn other_events_are_not_choices() {
        let msg = Event::Message {
            id: "m".into(),
            kind: robroz_contract::MessageKind::Text,
            body: "hi".into(),
            created_at: "2026-11-20T01:50:00.000Z".into(),
        };
        assert!(ChoiceCard::from_event(&msg).is_none());
    }
}
