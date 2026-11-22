//! Maps contract events to the card models the view draws.
//!
//! Every colour, status line, action and badge comes from `robroz-card`.
//! This module only decides where a card sits in the stream.

use robroz_card::{Palette, Tone};
use robroz_contract::{Event, MessageKind, TaskState, WaitingFor};
use robroz_ui::cards::confirm::ConfirmCard;
use robroz_ui::cards::{ApprovalCard, Card, ChoiceCard};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusCard {
    pub task_id: String,
    pub bot_name: String,
    /// From `robroz-card`, or 「没做成：…」 when the host sent a `failure`.
    pub status_line: String,
    pub tone: Tone,
    pub palette: Palette,
    pub action: Option<&'static str>,
    /// What the bro is doing right now, one line. Only on a live run
    /// (`running` with the container up and no `failure`).
    pub progress: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryCard {
    pub summary: String,
    /// Volume-relative path, shown as-is. Display only.
    pub path: String,
    pub badge: Option<&'static str>,
    /// Why the task did not get done. Drawn grey like 「没写出结果」, no buttons.
    pub failure: Option<String>,
    /// `needsSetup: "search"` (robroz#76): offer 「去设置」 on the card.
    pub needs_search_setup: bool,
}

/// Status line while the task waits on an approval card (`waitingFor:
/// "approval"`). Grey, no button: the approval card is the only way to act.
pub const AWAITING_APPROVAL: &str = "等你批准";

/// Blocked on an unanswered approval card (`waitingFor: "approval"`, #57).
/// `takeover` or an absent field (older host) keeps amber 卡住 + 去私聊接管.
fn awaits_approval(state: TaskState, waiting_for: Option<WaitingFor>) -> bool {
    state == TaskState::Blocked && waiting_for == Some(WaitingFor::Approval)
}

/// How much a preview candidate counts when two events share a change time.
/// An outcome (`failure`) beats the answer that caused it, and 「等你批准」
/// beats the approval card it points at; an answered card is the weakest
/// because its result follows it.
pub fn preview_rank(event: &Event) -> u8 {
    match event {
        Event::Status {
            failure: Some(_), ..
        }
        | Event::Delivery {
            failure: Some(_), ..
        } => 2,
        // The status summarises the wait; its approval card is created at
        // the same instant and must not take over the preview.
        Event::Status {
            state, waiting_for, ..
        } if awaits_approval(*state, *waiting_for) => 2,
        Event::Choice {
            answered_at: Some(_),
            ..
        }
        | Event::Approval {
            answered_at: Some(_),
            ..
        } => 0,
        _ => 1,
    }
}

/// Whether a new preview `(change time, rank)` replaces the current one.
/// Event order across types is not guaranteed (README, #54), so the newest
/// change time wins and a tie goes to the higher rank, not to arrival order.
pub fn preview_wins(new: (&str, u8), cur: Option<(&str, u8)>) -> bool {
    let Some((cur_at, cur_rank)) = cur else {
        return true;
    };
    let parse = |t: &str| chrono::DateTime::parse_from_rfc3339(t).ok();
    match (parse(new.0), parse(cur_at)) {
        (Some(a), Some(b)) if a != b => a > b,
        (Some(_), Some(_)) => new.1 >= cur_rank,
        // Unparseable time: keep the old "last one wins".
        _ => true,
    }
}

/// The grey line for a task the host says did not get done.
pub fn failure_line(failure: &str) -> String {
    format!("没做成：{failure}")
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
        }
        | Event::Confirm {
            answered_at: Some(at),
            ..
        } => at,
        Event::Choice { created_at, .. }
        | Event::Approval { created_at, .. }
        | Event::Confirm { created_at, .. }
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
            failure: Some(failure),
            ..
        } => format!("{bot_name}：{}", failure_line(failure)),
        Event::Status {
            bot_name,
            state,
            waiting_for,
            ..
        } if awaits_approval(*state, *waiting_for) => format!("{bot_name}：{AWAITING_APPROVAL}"),
        // A live run with a progress line: say what the bro is doing, not 进行.
        Event::Status {
            bot_name,
            state,
            container_up,
            failure,
            progress,
            ..
        } if progress_line(*state, *container_up, failure.as_deref(), progress).is_some() => {
            let line = progress_line(*state, *container_up, failure.as_deref(), progress);
            format!("{bot_name}：{}", line.unwrap_or_default())
        }
        Event::Status {
            bot_name,
            state,
            container_up,
            ..
        } => format!(
            "{bot_name}：{}",
            robroz_card::status_line(*state, *container_up)
        ),
        Event::Delivery {
            failure: Some(failure),
            ..
        } => failure_line(failure),
        Event::Delivery { summary, .. } => summary.clone(),
        Event::Choice {
            bot_name, prompt, ..
        } => format!("{bot_name}：{prompt}"),
        Event::Approval {
            bot_name, summary, ..
        } => format!("{bot_name}：{summary}"),
        Event::Confirm {
            bot_name,
            text,
            title,
            ..
        } => format!(
            "{bot_name}：{}",
            text.as_deref().or(title.as_deref()).unwrap_or_default()
        ),
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
            failure,
            progress,
            waiting_for,
            ..
        } => {
            // A failed task is grey with no action, like a dead run, never
            // the green 完成.
            let (look, status_line) = match failure {
                Some(f) => (
                    robroz_card::appearance(TaskState::Running, false),
                    failure_line(f),
                ),
                // Waiting on an approval card: grey, no takeover button.
                // Amber 卡住 + 去私聊接管 stays for real takeovers.
                None if awaits_approval(*state, *waiting_for) => (
                    robroz_card::appearance(TaskState::Running, false),
                    AWAITING_APPROVAL.to_string(),
                ),
                None => (
                    robroz_card::appearance(*state, *container_up),
                    robroz_card::status_line(*state, *container_up).to_string(),
                ),
            };
            let card = StatusCard {
                task_id: task_id.clone(),
                bot_name: bot_name.clone(),
                status_line,
                tone: look.tone,
                palette: robroz_card::palette(look.tone),
                action: look.action,
                progress: progress_line(*state, *container_up, failure.as_deref(), progress),
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
            failure,
            needs_setup,
            ..
        } => {
            let card = DeliveryCard {
                summary: summary.clone(),
                path: path.clone(),
                badge: briefing.then(robroz_card::briefing_badge),
                failure: failure.clone(),
                needs_search_setup: needs_setup.as_deref() == Some("search"),
            };
            (Key::Event(id.clone()), Row::Delivery(card))
        }
        // Keyed by id, so the answered snapshot replaces the open card in place.
        Event::Choice { id, .. } => {
            let card = ChoiceCard::from_event(event)
                .expect("choice event")
                .seen_by(crate::user_name());
            (Key::Event(id.clone()), Row::Card(Card::Choice(card)))
        }
        // Same as choice: the answered snapshot replaces the pending card.
        Event::Approval { id, .. } => {
            let card = ApprovalCard::from_event(event)
                .expect("approval event")
                .seen_by(crate::user_name());
            (Key::Event(id.clone()), Row::Card(Card::Approval(card)))
        }
        // A bro's proposal; the answered snapshot replaces the pending card.
        Event::Confirm { id, .. } => {
            let card = ConfirmCard::from_event(event)
                .expect("confirm event")
                .seen_by(crate::user_name());
            (Key::Event(id.clone()), Row::Card(Card::Confirm(card)))
        }
    }
}

/// The progress line a status card shows, if any. The host already sends it
/// only on a live run, but the card also checks, so an old or odd event never
/// puts a stale line next to 完成 or 没做成. Whitespace collapses to one line.
fn progress_line(
    state: TaskState,
    container_up: bool,
    failure: Option<&str>,
    progress: &Option<String>,
) -> Option<String> {
    if state != TaskState::Running || !container_up || failure.is_some() {
        return None;
    }
    let line = progress
        .as_deref()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!line.is_empty()).then_some(line)
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

    fn confirm(decision: Option<robroz_contract::ApprovalDecision>) -> Event {
        Event::Confirm {
            id: "c1".into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            subject: robroz_contract::ConfirmSubject::Memory,
            task_id: Some("t1".into()),
            memory_id: Some("m1".into()),
            kind: Some(robroz_contract::MemoryKind::Habit),
            text: Some("周报先说结论".into()),
            scope: Some("user".into()),
            slug: None,
            title: None,
            instruction: None,
            decision,
            created_at: "2026-11-20T11:00:00.000Z".into(),
            answered_at: decision.map(|_| "2026-11-20T11:05:00.000Z".into()),
            answered_by: decision.map(|_| "Panda 的手机".into()),
        }
    }

    #[test]
    fn confirm_card_is_drawn_and_answered_in_place() {
        use robroz_ui::cards::confirm::ConfirmState;
        let mut feed = Feed::default();
        feed.apply(&confirm(None));
        let Some(Card::Confirm(c)) = feed.card("c1") else {
            panic!("confirm card expected");
        };
        assert!(c.is_pending());
        assert_eq!(c.scope_line.as_deref(), Some("所有 bro"));
        assert_eq!(preview(&confirm(None)), "Ada：周报先说结论");
        // The answered snapshot (here: someone else, after our 409) replaces it.
        feed.apply(&confirm(Some(robroz_contract::ApprovalDecision::Denied)));
        assert_eq!(feed.len(), 1, "one card, updated in place");
        let Some(Card::Confirm(c)) = feed.card("c1") else {
            panic!("confirm card expected");
        };
        assert!(
            matches!(&c.state, ConfirmState::Dropped { by: Some(b), .. } if b == "Panda 的手机")
        );
    }

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
            failure: None,
            progress: None,
            waiting_for: None,
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
            failure: None,
            needs_setup: None,
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

    fn actual(card: &StatusCard) -> (&str, Tone, Palette, Option<&'static str>) {
        (&card.status_line, card.tone, card.palette, card.action)
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

    fn with_progress(mut e: Event, text: Option<&str>) -> Event {
        if let Event::Status { progress, .. } = &mut e {
            *progress = text.map(Into::into);
        }
        e
    }

    #[test]
    fn progress_shows_on_a_live_run_and_updates_in_place() {
        let mut feed = Feed::default();
        let run = status("t1", "Ada", TaskState::Running, true);
        feed.apply(&with_progress(run.clone(), Some("开始干活")));
        assert_eq!(status_cards(&feed)[0].progress.as_deref(), Some("开始干活"));

        feed.apply(&with_progress(run.clone(), Some("在沙箱里跑")));
        assert_eq!(feed.len(), 1, "same card, new line");
        assert_eq!(
            status_cards(&feed)[0].progress.as_deref(),
            Some("在沙箱里跑")
        );

        // The host drops the field: the card goes back to one line.
        feed.apply(&with_progress(run, None));
        assert_eq!(status_cards(&feed)[0].progress, None);
        assert_eq!(
            actual(status_cards(&feed)[0]),
            expected(TaskState::Running, true)
        );
    }

    #[test]
    fn progress_is_one_line_and_never_on_a_finished_or_stuck_card() {
        let line = |e: Event| match to_row(&e).1 {
            Row::Status(c) => c.progress,
            _ => unreachable!(),
        };
        let p = Some("  在想\n怎么   回 ");
        assert_eq!(
            line(with_progress(
                status("t", "Ada", TaskState::Running, true),
                p
            ))
            .as_deref(),
            Some("在想 怎么 回")
        );
        assert_eq!(
            line(with_progress(
                status("t", "Ada", TaskState::Running, true),
                Some(" \n ")
            )),
            None,
            "blank progress takes no room"
        );
        for (state, up) in [
            (TaskState::Queued, false),
            (TaskState::Running, false),
            (TaskState::Blocked, false),
            (TaskState::Done, false),
        ] {
            assert_eq!(line(with_progress(status("t", "Ada", state, up), p)), None);
        }
        let mut failed = with_progress(status("t", "Ada", TaskState::Running, true), p);
        if let Event::Status { failure, .. } = &mut failed {
            *failure = Some("模型没回：超时".into());
        }
        assert_eq!(line(failed), None, "没做成 never carries a progress line");
    }

    #[test]
    fn list_preview_shows_progress_while_running() {
        let run = status("t1", "Ada", TaskState::Running, true);
        assert_eq!(
            preview(&with_progress(run.clone(), Some("在查上周的\n工单"))),
            "Ada：在查上周的 工单"
        );
        // No progress: the plain state word, as before.
        assert_eq!(
            preview(&run),
            format!(
                "Ada：{}",
                robroz_card::status_line(TaskState::Running, true)
            )
        );
        // Never on a finished, stuck or failed card.
        let done = with_progress(status("t1", "Ada", TaskState::Done, false), Some("在查"));
        assert_eq!(
            preview(&done),
            format!("Ada：{}", robroz_card::status_line(TaskState::Done, false))
        );
        let mut failed = with_progress(run, Some("在查"));
        if let Event::Status { failure, .. } = &mut failed {
            *failure = Some("模型没回：超时".into());
        }
        assert_eq!(preview(&failed), "Ada：没做成：模型没回：超时");
    }

    #[test]
    fn long_progress_preview_is_cut_with_an_ellipsis() {
        let long = "在查上周的工单，按客户和优先级整理成一份周报草稿，再把超时没回的单子单独列出来，最后发给你确认";
        let run = with_progress(status("t1", "Ada", TaskState::Running, true), Some(long));
        // The list row fits the stored preview to its width with `…`.
        let shown = robroz_ui::conversations::fit(&preview(&run), 30);
        assert!(shown.starts_with("Ada：在查上周的"));
        assert!(shown.ends_with('…'), "{shown}");
    }

    #[test]
    fn progress_from_host_json() {
        let json = serde_json::json!({
            "type": "status", "id": "t1", "taskId": "t1", "botId": "bot-ada",
            "botName": "Ada", "state": "running", "label": "进行中",
            "createdAt": "2026-11-20T10:00:00.000Z", "updatedAt": "2026-11-20T10:00:01.000Z",
            "containerUp": true, "progress": "在沙箱里跑"
        });
        let event: Event = serde_json::from_value(json).unwrap();
        let mut feed = Feed::default();
        feed.apply(&event);
        assert_eq!(
            status_cards(&feed)[0].progress.as_deref(),
            Some("在沙箱里跑")
        );
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
    fn failed_task_is_grey_without_action_or_done() {
        let mut ev = status("t4", "Ada", TaskState::Done, false);
        if let Event::Status { failure, .. } = &mut ev {
            *failure = Some("模型没回：模型服务返回 500".into());
        }
        let mut feed = Feed::default();
        feed.apply(&status("t4", "Ada", TaskState::Running, true));
        feed.apply(&ev);
        let cards = status_cards(&feed);
        assert_eq!(cards.len(), 1, "updates in place");
        assert_eq!(cards[0].status_line, "没做成：模型没回：模型服务返回 500");
        assert_eq!(cards[0].tone, Tone::Gray);
        assert_eq!(cards[0].palette, expected(TaskState::Running, false).2);
        assert_ne!(cards[0].palette, expected(TaskState::Done, false).2);
        assert_eq!(cards[0].action, None);
        assert_eq!(preview(&ev), "Ada：没做成：模型没回：模型服务返回 500");
    }

    #[test]
    fn failed_delivery_carries_failure_and_old_json_is_unchanged() {
        let mut ev = delivery("d9", "tasks/t9/result.txt", false);
        if let Event::Delivery { failure, .. } = &mut ev {
            *failure = Some("你拒绝了：发送周报".into());
        }
        let mut feed = Feed::default();
        feed.apply(&ev);
        assert!(matches!(
            feed.rows().next(),
            Some(Row::Delivery(DeliveryCard { failure: Some(f), .. })) if f == "你拒绝了：发送周报"
        ));
        assert_eq!(preview(&ev), "没做成：你拒绝了：发送周报");

        let old: Event = serde_json::from_value(serde_json::json!({
            "type": "delivery", "id": "d1", "summary": "已写入结果",
            "path": "tasks/t1/result.txt", "briefing": false,
            "createdAt": "2026-10-07T19:00:00.000Z"
        }))
        .unwrap();
        let mut feed = Feed::default();
        feed.apply(&old);
        assert!(matches!(
            feed.rows().next(),
            Some(Row::Delivery(DeliveryCard { failure: None, .. }))
        ));
        assert_eq!(preview(&old), "已写入结果");

        let mut setup = ev.clone();
        if let Event::Delivery {
            failure,
            needs_setup,
            ..
        } = &mut setup
        {
            *failure = Some("还没设置联网搜索".into());
            *needs_setup = Some("search".into());
        }
        let mut feed = Feed::default();
        feed.apply(&setup);
        assert!(matches!(
            feed.rows().next(),
            Some(Row::Delivery(DeliveryCard {
                needs_search_setup: true,
                ..
            }))
        ));

        let mut setup_ok = ev.clone();
        if let Event::Delivery {
            failure,
            needs_setup,
            ..
        } = &mut setup_ok
        {
            *failure = None;
            *needs_setup = Some("search".into());
        }
        let mut feed = Feed::default();
        feed.apply(&setup_ok);
        assert!(
            matches!(
                feed.rows().next(),
                Some(Row::Delivery(DeliveryCard {
                    needs_search_setup: true,
                    failure: None,
                    ..
                }))
            ),
            "去设置 when needsSetup is set, even without failure"
        );

        let mut text_only = ev;
        if let Event::Delivery {
            failure,
            needs_setup,
            ..
        } = &mut text_only
        {
            *failure = Some("还没设置联网搜索".into());
            *needs_setup = None;
        }
        let mut feed = Feed::default();
        feed.apply(&text_only);
        assert!(
            matches!(
                feed.rows().next(),
                Some(Row::Delivery(DeliveryCard {
                    needs_search_setup: false,
                    ..
                }))
            ),
            "no 去设置 without needsSetup"
        );

        let old_status: Event = serde_json::from_value(serde_json::json!({
            "type": "status", "id": "t1", "taskId": "t1", "botId": "b", "botName": "Ada",
            "state": "done", "label": "完成", "createdAt": "2026-10-07T19:00:00.000Z",
            "updatedAt": "2026-10-07T19:00:00.000Z", "containerUp": false
        }))
        .unwrap();
        let mut feed = Feed::default();
        feed.apply(&old_status);
        assert_eq!(
            actual(status_cards(&feed)[0]),
            expected(TaskState::Done, false)
        );
    }

    fn blocked_waiting(waiting: Option<WaitingFor>) -> Event {
        let mut ev = status("t5", "Ada", TaskState::Blocked, false);
        if let Event::Status { waiting_for, .. } = &mut ev {
            *waiting_for = waiting;
        }
        ev
    }

    #[test]
    fn waiting_for_approval_is_grey_without_take_over() {
        let ev = blocked_waiting(Some(WaitingFor::Approval));
        let mut feed = Feed::default();
        feed.apply(&ev);
        let card = status_cards(&feed)[0];
        assert_eq!(card.status_line, "等你批准");
        assert_eq!(card.tone, Tone::Gray);
        assert_eq!(card.palette, expected(TaskState::Running, false).2);
        assert_eq!(
            card.action, None,
            "the approval card is the only way to act"
        );
        assert_eq!(preview(&ev), "Ada：等你批准");

        // Real takeovers (captcha) and older hosts without the field keep amber.
        for waiting in [Some(WaitingFor::Takeover), None] {
            let mut feed = Feed::default();
            feed.apply(&blocked_waiting(waiting));
            assert_eq!(
                actual(status_cards(&feed)[0]),
                expected(TaskState::Blocked, false),
                "{waiting:?}"
            );
            assert_eq!(status_cards(&feed)[0].action, Some("去私聊接管"));
        }
        // Once answered the next status has no waitingFor and updates in place.
        feed.apply(&status("t5", "Ada", TaskState::Queued, false));
        assert_eq!(feed.len(), 1);
        assert_eq!(
            actual(status_cards(&feed)[0]),
            expected(TaskState::Queued, false)
        );
    }

    #[test]
    fn takeover_to_approval_replaces_the_same_card_in_place() {
        // First poll (list) says takeover, then SSE says approval for the
        // same task: one card, updated where it was.
        let mut feed = Feed::default();
        feed.apply(&text("m1", "帮我发周报"));
        feed.apply(&blocked_waiting(Some(WaitingFor::Takeover)));
        feed.apply(&text("m2", "之后"));
        feed.apply(&blocked_waiting(Some(WaitingFor::Approval)));
        feed.apply(&blocked_waiting(Some(WaitingFor::Approval)));
        assert_eq!(feed.len(), 3, "one status card, not two");
        let cards = status_cards(&feed);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].status_line, "等你批准");
        assert_eq!(cards[0].action, None);
        assert!(
            matches!(feed.rows().nth(1), Some(Row::Status(_))),
            "kept its slot"
        );
        // And back: approval -> takeover (card answered elsewhere, captcha next).
        feed.apply(&blocked_waiting(Some(WaitingFor::Takeover)));
        assert_eq!(status_cards(&feed).len(), 1);
        assert_eq!(status_cards(&feed)[0].action, Some("去私聊接管"));
    }

    #[test]
    fn awaiting_status_beats_its_approval_card_in_the_preview() {
        let at = "2026-11-20T10:29:00.000Z";
        let status = blocked_waiting(Some(WaitingFor::Approval));
        assert_eq!(preview_rank(&status), 2);
        let card = (at, 1); // unanswered approval created at the same instant
        assert!(!preview_wins(card, Some((at, preview_rank(&status)))));
        assert!(preview_wins((at, preview_rank(&status)), Some(card)));
        assert_eq!(
            preview_rank(&blocked_waiting(Some(WaitingFor::Takeover))),
            1
        );
    }

    #[test]
    fn waiting_for_from_host_json() {
        let json = |extra: serde_json::Value| {
            let mut v = serde_json::json!({
                "type": "status", "id": "t1", "taskId": "t1", "botId": "b", "botName": "Ada",
                "state": "blocked", "label": "卡住", "createdAt": "2026-11-20T02:00:01.000Z",
                "updatedAt": "2026-11-20T02:00:02.000Z", "containerUp": false
            });
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<Event>(v).unwrap()
        };
        let line = |ev: Event| {
            let mut feed = Feed::default();
            feed.apply(&ev);
            status_cards(&feed)[0].status_line.clone()
        };
        assert_eq!(
            line(json(serde_json::json!({"waitingFor": "approval"}))),
            "等你批准"
        );
        assert_eq!(
            line(json(serde_json::json!({"waitingFor": "takeover"}))),
            "卡住"
        );
        assert_eq!(
            line(json(serde_json::json!({}))),
            "卡住",
            "older host: unchanged"
        );
    }

    #[test]
    fn list_preview_prefers_newest_then_outcome_over_answer() {
        let deny_at = "2026-11-20T10:10:07.502Z";
        let failed = ("2026-11-20T10:10:07.503Z", 2);
        let answered = (deny_at, 0);
        // Host writes the answer first, then the task outcome: outcome wins
        // whichever order the two events arrive in.
        assert!(preview_wins(failed, Some(answered)));
        assert!(!preview_wins(answered, Some(failed)));
        // Same millisecond: the outcome still wins the tie.
        assert!(!preview_wins((deny_at, 0), Some((deny_at, 2))));
        assert!(preview_wins((deny_at, 2), Some((deny_at, 0))));
        // A newer message always takes over; equal rank and time: last wins.
        assert!(preview_wins(
            ("2026-11-20T10:11:00Z", 1),
            Some((deny_at, 2))
        ));
        assert!(preview_wins((deny_at, 1), Some((deny_at, 1))));
        assert!(preview_wins((deny_at, 1), None));
    }

    #[test]
    fn preview_rank_orders_outcome_message_answer() {
        let mut failed = status("t6", "Ada", TaskState::Done, false);
        if let Event::Status { failure, .. } = &mut failed {
            *failure = Some("你拒绝了：发送周报".into());
        }
        assert_eq!(preview_rank(&failed), 2);
        assert_eq!(preview_rank(&text("m1", "hi")), 1);
        assert_eq!(
            preview_rank(&status("t6", "Ada", TaskState::Done, false)),
            1
        );
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
            answered_by: None,
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
        feed.apply_card(card(ApprovalState::Approved { by: None, at: 0 }));
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
