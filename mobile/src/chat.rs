//! One conversation as chat rows: text bubbles and status cards.
//!
//! Own messages are bubbles on the right, a bro's messages bubbles on the
//! left under the bro's name, and a task's status card sits on the left
//! under the bro. A card is keyed by task: later status events update the
//! card where it first appeared. Other card types (delivery, choice,
//! approval, ...) are not drawn in the spike.

use robroz_contract::Event;

use crate::card::StatusCard;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatItem {
    Text { body: String, own: bool },
    Status(StatusCard),
}

/// `events` is oldest first, each with whether a message is the user's own
/// (see `Conn::events`). A status event replaces the earlier card of the
/// same task in place.
pub fn build(events: &[(Event, bool)]) -> Vec<ChatItem> {
    let mut items: Vec<ChatItem> = Vec::new();
    let mut card_at: Vec<(&str, usize)> = Vec::new();
    for (event, own) in events {
        match event {
            Event::Message { body, .. } => items.push(ChatItem::Text {
                body: body.clone(),
                own: *own,
            }),
            Event::Status { task_id, .. } => {
                let Some(card) = StatusCard::from_event(event) else {
                    continue;
                };
                match card_at.iter().find(|(t, _)| *t == task_id.as_str()) {
                    Some(&(_, at)) => items[at] = ChatItem::Status(card),
                    None => {
                        card_at.push((task_id.as_str(), items.len()));
                        items.push(ChatItem::Status(card));
                    }
                }
            }
            _ => {}
        }
    }
    items
}

/// The name shown over a bro's bubbles: the bro on the newest status card,
/// else the conversation's own name.
pub fn bro_name<'a>(items: &'a [ChatItem], fallback: &'a str) -> &'a str {
    items
        .iter()
        .rev()
        .find_map(|i| match i {
            ChatItem::Status(c) => Some(c.bot_name.as_str()),
            _ => None,
        })
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::{MessageKind, TaskState};

    fn msg(id: &str, body: &str) -> Event {
        Event::Message {
            id: id.into(),
            kind: MessageKind::Text,
            body: body.into(),
            created_at: "2026-11-20T10:00:00.000Z".into(),
        }
    }

    fn status(task: &str, state: TaskState) -> Event {
        Event::Status {
            id: format!("s-{task}"),
            task_id: task.into(),
            bot_id: "b".into(),
            bot_name: "Ada".into(),
            state,
            label: String::new(),
            created_at: "2026-11-20T10:00:00.000Z".into(),
            updated_at: "2026-11-20T10:00:00.000Z".into(),
            container_up: true,
            failure: None,
            waiting_for: None,
            progress: None,
        }
    }

    #[test]
    fn card_updates_in_place_and_sides_are_kept() {
        let events = vec![
            (msg("m1", "write the report"), true),
            (status("t1", TaskState::Running), false),
            (msg("m2", "on it"), false),
            (status("t1", TaskState::Done), false),
        ];
        let items = build(&events);
        assert_eq!(items.len(), 3, "one card per task");
        assert!(matches!(&items[0], ChatItem::Text { own: true, .. }));
        assert!(matches!(&items[2], ChatItem::Text { own: false, .. }));
        let ChatItem::Status(card) = &items[1] else {
            panic!("card stays where it first appeared")
        };
        assert_eq!(
            card.status_line,
            robroz_card::status_line(TaskState::Done, true)
        );
        assert_eq!(bro_name(&items, "chat"), "Ada");
        assert_eq!(bro_name(&items[..1], "chat"), "chat");
    }
}
