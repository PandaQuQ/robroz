//! Maps contract events to the card models the view draws.
//!
//! Every colour, status line, action and badge comes from `robroz-card`.
//! This module only decides where a card sits in the stream.

use robroz_card::{Palette, Tone};
use robroz_contract::{Event, MessageKind};
use robroz_ui::cards::{ApprovalCard, Card, ChoiceCard};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusCard {
    pub task_id: String,
    pub bot_name: String,
    pub status_line: &'static str,
    pub tone: Tone,
    pub palette: Palette,
    pub action: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryCard {
    pub summary: String,
    /// Volume-relative path, shown as-is. Display only.
    pub path: String,
    pub badge: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Text {
        body: String,
    },
    Attachment {
        kind: MessageKind,
        name: String,
    },
    Status(StatusCard),
    Delivery(DeliveryCard),
    /// New card kinds (`choice`, `approval`, `draft`), drawn by `robroz-ui`.
    Card(Card),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Event(String),
    Task(String),
}

/// Rows for one group, in arrival order. Applying an event is idempotent, so
/// a source may hand over the full snapshot from
/// `GET /api/groups/:id/events` or only what is new.
#[derive(Debug, Default, Clone)]
pub struct Feed {
    rows: Vec<(Key, Row)>,
}

impl Feed {
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().map(|(_, row)| row)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn apply(&mut self, event: &Event) {
        let (key, row) = to_row(event);
        self.upsert(key, row);
    }

    /// Insert or replace a new-kind card by its id. Used for card kinds the
    /// contract has no event for yet (approval, draft) and for applying the
    /// host's reply to an answer before the next poll.
    pub fn apply_card(&mut self, card: Card) {
        let key = Key::Event(card.id().to_string());
        self.upsert(key, Row::Card(card));
    }

    pub fn card(&self, id: &str) -> Option<&Card> {
        self.rows.iter().find_map(|(k, row)| match (k, row) {
            (Key::Event(k), Row::Card(c)) if k == id => Some(c),
            _ => None,
        })
    }

    fn upsert(&mut self, key: Key, row: Row) {
        match self.rows.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = row,
            None => self.rows.push((key, row)),
        }
    }
}

/// When this card last changed, as the host defines it for `?since=`:
/// `updatedAt` for status cards, `answeredAt` for answered choices and approvals,
/// otherwise `createdAt`.
pub fn change_time(event: &Event) -> &str {
    match event {
        Event::Status { updated_at, .. } => updated_at,
        Event::Choice {
            answered_at: Some(at),
            ..
        }
        | Event::Approval {
            answered_at: Some(at),
            ..
        } => at,
        Event::Choice { created_at, .. }
        | Event::Approval { created_at, .. }
        | Event::Message { created_at, .. }
        | Event::Delivery { created_at, .. } => created_at,
    }
}

/// One-line preview of an event for the conversation list.
pub fn preview(event: &Event) -> String {
    match event {
        Event::Message { kind, body, .. } => match kind {
            MessageKind::Text => body.clone(),
            MessageKind::Image | MessageKind::File => {
                format!("[{}] {body}", attachment_label(*kind))
            }
        },
        Event::Status {
            bot_name,
            state,
            container_up,
            ..
        } => format!(
            "{bot_name}：{}",
            robroz_card::status_line(*state, *container_up)
        ),
        Event::Delivery { summary, .. } => summary.clone(),
        Event::Choice {
            bot_name, prompt, ..
        } => format!("{bot_name}：{prompt}"),
        Event::Approval {
            bot_name, summary, ..
        } => format!("{bot_name}：{summary}"),
    }
}

fn to_row(event: &Event) -> (Key, Row) {
    match event {
        Event::Message { id, kind, body, .. } => {
            let row = match kind {
                MessageKind::Text => Row::Text { body: body.clone() },
                MessageKind::Image | MessageKind::File => Row::Attachment {
                    kind: *kind,
                    name: body.clone(),
                },
            };
            (Key::Event(id.clone()), row)
        }
        Event::Status {
            task_id,
            bot_name,
            state,
            container_up,
            ..
        } => {
            let look = robroz_card::appearance(*state, *container_up);
            let card = StatusCard {
                task_id: task_id.clone(),
                bot_name: bot_name.clone(),
                status_line: robroz_card::status_line(*state, *container_up),
                tone: look.tone,
                palette: robroz_card::palette(look.tone),
                action: look.action,
            };
            (Key::Task(task_id.clone()), Row::Status(card))
        }
        // A briefing is the same delivery event with `briefing: true`; the
        // badge is driven only by that contract field.
        Event::Delivery {
            id,
            summary,
            path,
            briefing,
            ..
        } => {
            let card = DeliveryCard {
                summary: summary.clone(),
                path: path.clone(),
                badge: briefing.then(robroz_card::briefing_badge),
            };
            (Key::Event(id.clone()), Row::Delivery(card))
        }
        // Keyed by id, so the answered snapshot replaces the open card in place.
        Event::Choice { id, .. } => {
            let card = ChoiceCard::from_event(event).expect("choice event");
            (Key::Event(id.clone()), Row::Card(Card::Choice(card)))
        }
        // Same as choice: the answered snapshot replaces the pending card.
        Event::Approval { id, .. } => {
            let card = ApprovalCard::from_event(event).expect("approval event");
            (Key::Event(id.clone()), Row::Card(Card::Approval(card)))
        }
    }
}

/// Plain label for an attachment row. Not a task state, so it lives here.
pub fn attachment_label(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Image => "图片",
        MessageKind::File => "文件",
        MessageKind::Text => "文本",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::TaskState;

    fn status(task: &str, bot: &str, state: TaskState, container_up: bool) -> Event {
        Event::Status {
            id: task.into(),
            task_id: task.into(),
            bot_id: format!("bot-{bot}"),
            bot_name: bot.into(),
            state,
            label: state.label().into(),
            created_at: "2026-10-07T19:00:00.000Z".into(),
            updated_at: "2026-10-07T19:00:00.000Z".into(),
            container_up,
        }
    }

    fn text(id: &str, body: &str) -> Event {
        Event::Message {
            id: id.into(),
            kind: MessageKind::Text,
            body: body.into(),
            created_at: "2026-10-07T19:00:00.000Z".into(),
        }
    }

    fn delivery(id: &str, path: &str, briefing: bool) -> Event {
        Event::Delivery {
            id: id.into(),
            summary: "已写入结果".into(),
            path: path.into(),
            briefing,
            created_at: "2026-10-07T19:00:00.000Z".into(),
        }
    }

    fn status_cards(feed: &Feed) -> Vec<&StatusCard> {
        feed.rows()
            .filter_map(|r| match r {
                Row::Status(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    fn expected(
        state: TaskState,
        container_up: bool,
    ) -> (&'static str, Tone, Palette, Option<&'static str>) {
        let look = robroz_card::appearance(state, container_up);
        (
            robroz_card::status_line(state, container_up),
            look.tone,
            robroz_card::palette(look.tone),
            look.action,
        )
    }

    fn actual(card: &StatusCard) -> (&'static str, Tone, Palette, Option<&'static str>) {
        (card.status_line, card.tone, card.palette, card.action)
    }

    #[test]
    fn status_updates_in_place_by_task_id() {
        let mut feed = Feed::default();
        feed.apply(&text("m1", "开始"));
        feed.apply(&status("t1", "Ada", TaskState::Queued, false));
        feed.apply(&text("m2", "中间插一句"));
        assert_eq!(
            actual(status_cards(&feed)[0]),
            expected(TaskState::Queued, false)
        );

        feed.apply(&status("t1", "Ada", TaskState::Running, true));
        assert_eq!(
            actual(status_cards(&feed)[0]),
            expected(TaskState::Running, true)
        );

        feed.apply(&status("t1", "Ada", TaskState::Done, false));
        feed.apply(&delivery("d1", "tasks/t1/result.txt", false));

        assert_eq!(feed.len(), 4, "one status card, not three");
        let cards = status_cards(&feed);
        assert_eq!(cards.len(), 1);
        assert_eq!(actual(cards[0]), expected(TaskState::Done, false));
        assert_eq!(cards[0].bot_name, "Ada");
        // Still in its original slot, between the two messages.
        assert!(matches!(feed.rows().nth(1), Some(Row::Status(_))));
        assert!(matches!(feed.rows().nth(3), Some(Row::Delivery(_))));
    }

    #[test]
    fn dead_running_renders_no_result_gray_without_action() {
        let mut feed = Feed::default();
        feed.apply(&status("t2", "Cy", TaskState::Running, true));
        feed.apply(&status("t2", "Cy", TaskState::Running, false));
        let cards = status_cards(&feed);
        assert_eq!(cards.len(), 1);
        assert_eq!(actual(cards[0]), expected(TaskState::Running, false));
        assert_eq!(cards[0].status_line, "没写出结果");
        assert_eq!(cards[0].palette, expected(TaskState::Queued, false).2);
        assert_eq!(cards[0].action, None);
    }

    #[test]
    fn blocked_has_only_take_over_action() {
        let mut feed = Feed::default();
        feed.apply(&status("t3", "Bea", TaskState::Blocked, false));
        let card = status_cards(&feed)[0];
        assert_eq!(actual(card), expected(TaskState::Blocked, false));
        assert_eq!(card.action, Some("去私聊接管"));
    }

    #[test]
    fn briefing_delivery_carries_badge_and_raw_path() {
        let mut feed = Feed::default();
        feed.apply(&delivery("d1", "tasks/t1/result.txt", false));
        feed.apply(&delivery("d2", "tasks/t-brief/result.txt", true));
        let cards: Vec<_> = feed
            .rows()
            .filter_map(|r| match r {
                Row::Delivery(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(cards[0].badge, None);
        assert_eq!(cards[0].path, "tasks/t1/result.txt");
        assert_eq!(cards[1].badge, Some(robroz_card::briefing_badge()));
        assert_eq!(cards[1].path, "tasks/t-brief/result.txt");
    }

    #[test]
    fn reapplying_a_snapshot_is_idempotent() {
        let events = vec![
            text("m1", "hi"),
            status("t1", "Ada", TaskState::Done, false),
            delivery("d1", "tasks/t1/result.txt", false),
        ];
        let mut feed = Feed::default();
        for e in &events {
            feed.apply(e);
        }
        let before: Vec<Row> = feed.rows().cloned().collect();
        for e in &events {
            feed.apply(e);
        }
        assert_eq!(feed.rows().cloned().collect::<Vec<_>>(), before);
    }

    #[test]
    fn attachments_are_labelled_rows() {
        let mut feed = Feed::default();
        feed.apply(&Event::Message {
            id: "m9".into(),
            kind: MessageKind::Image,
            body: "hero.png".into(),
            created_at: "2026-10-07T19:00:00.000Z".into(),
        });
        assert_eq!(
            feed.rows().next(),
            Some(&Row::Attachment {
                kind: MessageKind::Image,
                name: "hero.png".into()
            })
        );
    }

    #[test]
    fn answered_choice_replaces_open_card_in_place() {
        let card = |choice: Option<&str>| Event::Choice {
            id: "c1".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            prompt: "用哪个？".into(),
            options: vec![
                robroz_contract::ChoiceOption {
                    id: "a".into(),
                    label: "A".into(),
                    primary: true,
                },
                robroz_contract::ChoiceOption {
                    id: "b".into(),
                    label: "B".into(),
                    primary: false,
                },
            ],
            multi: false,
            choice: choice.map(|v| vec![v.to_string()]),
            created_at: "2026-10-03T19:00:00.000Z".into(),
            answered_at: choice.map(|_| "2026-10-03T19:01:00.000Z".into()),
        };
        let mut feed = Feed::default();
        feed.apply(&card(None));
        feed.apply(&card(Some("b")));
        assert_eq!(feed.len(), 1);
        let row = feed.rows().next().unwrap().clone();
        match row {
            Row::Card(Card::Choice(c)) => {
                assert!(matches!(
                    c.state,
                    robroz_ui::cards::ChoiceState::Answered { ref selected, .. } if selected == &["b"]
                ));
                assert_eq!(c.options.len(), 2);
            }
            other => panic!("expected choice row, got {other:?}"),
        }
    }

    #[test]
    fn approval_and_draft_cards_update_in_place_by_id() {
        use robroz_ui::cards::{ActionKind, ApprovalCard, ApprovalState};
        let card = |state| {
            Card::Approval(ApprovalCard {
                id: "a1".into(),
                bot: "后端".into(),
                kind: ActionKind::Delete,
                summary: "清理 12 个旧日志文件".into(),
                path: Some("logs/2026-10/".into()),
                state,
            })
        };
        let mut feed = Feed::default();
        feed.apply(&text("m1", "先问一下"));
        feed.apply_card(card(ApprovalState::Pending));
        feed.apply(&text("m2", "后面的话"));
        feed.apply_card(card(ApprovalState::Approved { at: 0 }));
        assert_eq!(feed.len(), 3, "one approval card, not two");
        assert!(matches!(
            feed.card("a1"),
            Some(Card::Approval(ApprovalCard {
                state: ApprovalState::Approved { .. },
                ..
            }))
        ));
        assert!(
            matches!(feed.rows().nth(1), Some(Row::Card(_))),
            "kept its slot"
        );
    }
}
