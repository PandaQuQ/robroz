//! `confirm` cards (robroz#66): a bro proposes a memory (or, later, a skill)
//! and the person keeps it or not, once. Answered through the approval
//! endpoints (`approve` = 记下 / 保存, `deny` = 不用), so a 409 and the
//! answered-by footnote follow the approval card's rules.

use std::collections::{HashMap, HashSet};

use chrono::TimeZone;
use robroz_contract::{ApprovalDecision, ConfirmSubject, Event, MemoryKind};

use super::interact::{CardUi, Note};
use super::status::{clock_in, Dot};
use super::{as_me, memories};

/// Top-left tag, same style as the approval card's 「发送」「删除」.
pub const MEMORY_TAG: &str = "记忆";
pub const SKILL_TAG: &str = "技能";
pub const PENDING: &str = "待确认";
pub const KEEP: &str = "记下";
pub const SAVE: &str = "保存";
pub const DROP: &str = "不用";
pub const KEPT: &str = "记下了";
pub const SAVED: &str = "保存了";
pub const DROPPED: &str = "不用了";
/// 409 from the 记忆 page: the card was answered elsewhere.
pub const ALREADY_ANSWERED: &str = "已经有人处理过了，正在刷新";
/// 记下 / 不用 failed on the 记忆 page: 「没成功：<reason>」.
pub fn decide_failed(reason: &str) -> String {
    format!("没成功：{reason}")
}
/// Skill instructions show this many lines until opened.
pub const INSTRUCTION_LINES: usize = 3;
pub const SHOW_ALL: &str = "展开";
pub const SHOW_LESS: &str = "收起";
/// Pending cards shown per task before the rest fold away.
pub const FOLD_AFTER: usize = 3;

/// 「还有 N 条待确认」.
pub fn more_label(n: usize) -> String {
    format!("还有 {n} 条待确认")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmState {
    Pending,
    /// `by`: the host's `answeredBy`, own name as 「我」; `None` when unknown.
    /// `at`: `answeredAt` in unix seconds.
    Kept {
        by: Option<String>,
        at: i64,
    },
    Dropped {
        by: Option<String>,
        at: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmCard {
    pub id: String,
    pub bot: String,
    pub task_id: Option<String>,
    pub subject: ConfirmSubject,
    pub memory_id: Option<String>,
    pub kind: Option<MemoryKind>,
    pub text: Option<String>,
    /// 「所有 bro」 or 「只给 Ada」; `None` when the card has no scope.
    pub scope_line: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub instruction: Option<String>,
    pub state: ConfirmState,
}

impl ConfirmCard {
    /// Build from the `confirm` event. `None` for any other event.
    pub fn from_event(event: &Event) -> Option<Self> {
        let Event::Confirm {
            id,
            bot_id,
            bot_name,
            subject,
            task_id,
            memory_id,
            kind,
            text,
            scope,
            slug,
            title,
            instruction,
            decision,
            answered_at,
            answered_by,
            ..
        } = event
        else {
            return None;
        };
        let at = answered_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map_or(0, |t| t.timestamp());
        let by = answered_by.clone();
        let state = match decision {
            None => ConfirmState::Pending,
            Some(ApprovalDecision::Approved) => ConfirmState::Kept { by, at },
            Some(ApprovalDecision::Denied) => ConfirmState::Dropped { by, at },
        };
        // The proposing bro's own name is on the event; other bots show by id.
        let bros = [(bot_id.clone(), bot_name.clone())];
        Some(Self {
            id: id.clone(),
            bot: bot_name.clone(),
            task_id: task_id.clone(),
            subject: *subject,
            memory_id: memory_id.clone(),
            kind: *kind,
            text: text.clone(),
            scope_line: scope.as_deref().map(|s| memories::scope_label(s, &bros)),
            slug: slug.clone(),
            title: title.clone(),
            instruction: instruction.clone(),
            state,
        })
    }

    /// Show this client's own name (`me`) as 「我」.
    pub fn seen_by(mut self, me: &str) -> Self {
        if let ConfirmState::Kept { by, .. } | ConfirmState::Dropped { by, .. } = &mut self.state {
            *by = as_me(by.take(), me);
        }
        self
    }

    pub fn is_pending(&self) -> bool {
        self.state == ConfirmState::Pending
    }

    pub fn tag(&self) -> &'static str {
        match self.subject {
            ConfirmSubject::Memory => MEMORY_TAG,
            ConfirmSubject::Skill => SKILL_TAG,
        }
    }

    /// The approve button: 记下 for memories, 保存 for skills.
    pub fn keep_label(&self) -> &'static str {
        match self.subject {
            ConfirmSubject::Memory => KEEP,
            ConfirmSubject::Skill => SAVE,
        }
    }

    fn kept_word(&self) -> &'static str {
        match self.subject {
            ConfirmSubject::Memory => KEPT,
            ConfirmSubject::Skill => SAVED,
        }
    }

    /// Header status: 待确认 / 记下了 (保存了) / 不用了 (grey).
    pub fn status(&self) -> (&'static str, Dot) {
        match self.state {
            ConfirmState::Pending => (PENDING, Dot::Waiting),
            ConfirmState::Kept { .. } => (self.kept_word(), Dot::Done),
            ConfirmState::Dropped { .. } => (DROPPED, Dot::Void),
        }
    }

    /// 「记下了 · 我 · 08:48」, 「不用了 · Panda 的手机 · 08:48」, or without
    /// a name when the host does not know who answered: 「记下了 · 08:48」.
    pub fn footnote_in<Tz: TimeZone>(&self, tz: &Tz) -> Option<String>
    where
        Tz::Offset: std::fmt::Display,
    {
        let (word, by, at) = match &self.state {
            ConfirmState::Pending => return None,
            ConfirmState::Kept { by, at } => (self.kept_word(), by, *at),
            ConfirmState::Dropped { by, at } => (DROPPED, by, *at),
        };
        let mut parts = vec![word.to_string()];
        parts.extend(by.clone());
        let time = clock_in(at, tz);
        if !time.is_empty() {
            parts.push(time);
        }
        Some(parts.join(" · "))
    }
}

/// 记下 / 不用 buttons: `Some(enabled)` while pending, `None` once answered or
/// after a 409 (the answer someone else gave is being fetched).
pub fn confirm_actions(card: &ConfirmCard, ui: &CardUi) -> Option<bool> {
    if ui.note == Some(Note::AlreadyDecided) || !card.is_pending() {
        return None;
    }
    Some(!ui.busy)
}

/// What the chat does with one row when folding a task's confirm cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fold {
    Show,
    Hide,
    /// The first hidden card's place: 「还有 N 条待确认」 for this task.
    More {
        task: String,
        n: usize,
    },
}

/// Fold plan for a chat's rows. Each item is `Some((taskId, pending))` for a
/// confirm card that has a task, `None` for anything else. Per task, the
/// first [`FOLD_AFTER`] pending cards show; the rest hide behind one
/// 「还有 N 条待确认」 at the first hidden card's place, unless that task is in
/// `open`. Answered cards and cards without a task always show.
pub fn fold_plan<'a>(
    rows: impl IntoIterator<Item = Option<(&'a str, bool)>>,
    open: &HashSet<String>,
) -> Vec<Fold> {
    let rows: Vec<_> = rows.into_iter().collect();
    let mut pending: HashMap<&str, usize> = HashMap::new();
    for (task, _) in rows.iter().flatten().filter(|(_, p)| *p) {
        *pending.entry(task).or_default() += 1;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    rows.iter()
        .map(|row| match row {
            Some((task, true)) if !open.contains(*task) => {
                let n = seen.entry(task).or_default();
                *n += 1;
                match *n {
                    k if k <= FOLD_AFTER => Fold::Show,
                    k if k == FOLD_AFTER + 1 => Fold::More {
                        task: task.to_string(),
                        n: pending[task] - FOLD_AFTER,
                    },
                    _ => Fold::Hide,
                }
            }
            _ => Fold::Show,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn event(id: &str, decision: Option<ApprovalDecision>, by: Option<&str>) -> Event {
        Event::Confirm {
            id: id.into(),
            bot_id: "bot-ada".into(),
            bot_name: "Ada".into(),
            subject: ConfirmSubject::Memory,
            task_id: Some("t1".into()),
            memory_id: Some(format!("m-{id}")),
            kind: Some(MemoryKind::Habit),
            text: Some("周报先说结论".into()),
            scope: Some("bot:bot-ada".into()),
            slug: None,
            title: None,
            instruction: None,
            decision,
            created_at: "2026-11-20T08:40:00Z".into(),
            answered_at: decision.map(|_| "2026-11-20T08:48:00Z".into()),
            answered_by: by.map(Into::into),
        }
    }

    #[test]
    fn memory_card_from_contract() {
        let c = ConfirmCard::from_event(&event("c1", None, None)).unwrap();
        assert_eq!((c.tag(), c.keep_label()), (MEMORY_TAG, KEEP));
        assert_eq!(c.text.as_deref(), Some("周报先说结论"));
        assert_eq!(c.scope_line.as_deref(), Some("只给 Ada"));
        assert_eq!(c.status(), (PENDING, Dot::Waiting));
        assert_eq!(c.footnote_in(&chrono::Utc), None);
        let mut user = event("c2", None, None);
        if let Event::Confirm { scope, .. } = &mut user {
            *scope = Some("user".into());
        }
        let c = ConfirmCard::from_event(&user).unwrap();
        assert_eq!(c.scope_line.as_deref(), Some("所有 bro"));
        let msg = Event::Message {
            id: "m".into(),
            kind: robroz_contract::MessageKind::Text,
            body: "hi".into(),
            created_at: "2026-11-20T01:50:00.000Z".into(),
        };
        assert!(ConfirmCard::from_event(&msg).is_none());
    }

    #[test]
    fn answered_footnote_names_me_other_or_nobody() {
        let me = "Panda 的 Mac";
        let utc = chrono::Utc;
        for (d, by, status, want) in [
            (
                ApprovalDecision::Approved,
                Some(me),
                KEPT,
                "记下了 · 我 · 08:48",
            ),
            (
                ApprovalDecision::Denied,
                Some(me),
                DROPPED,
                "不用了 · 我 · 08:48",
            ),
            (
                ApprovalDecision::Approved,
                Some("Panda 的手机"),
                KEPT,
                "记下了 · Panda 的手机 · 08:48",
            ),
            (ApprovalDecision::Approved, None, KEPT, "记下了 · 08:48"),
            (ApprovalDecision::Denied, None, DROPPED, "不用了 · 08:48"),
        ] {
            let c = ConfirmCard::from_event(&event("c1", Some(d), by))
                .unwrap()
                .seen_by(me);
            assert_eq!(c.status().0, status);
            assert_eq!(c.footnote_in(&utc).as_deref(), Some(want));
        }
        let dropped =
            ConfirmCard::from_event(&event("c1", Some(ApprovalDecision::Denied), None)).unwrap();
        assert_eq!(dropped.status().1, Dot::Void, "不用了 is grey");
    }

    #[test]
    fn skill_card_uses_save_and_its_own_fields() {
        let mut skill = Event::Confirm {
            id: "s1".into(),
            bot_id: "b".into(),
            bot_name: "B".into(),
            subject: ConfirmSubject::Skill,
            task_id: None,
            memory_id: None,
            kind: None,
            text: None,
            scope: None,
            slug: Some("weekly".into()),
            title: Some("写周报".into()),
            instruction: Some("先列三点".into()),
            decision: Some(ApprovalDecision::Approved),
            created_at: "2026-11-20T08:40:00Z".into(),
            answered_at: Some("2026-11-20T08:48:00Z".into()),
            answered_by: None,
        };
        let c = ConfirmCard::from_event(&skill).unwrap();
        assert_eq!((c.tag(), c.keep_label()), (SKILL_TAG, SAVE));
        assert_eq!(c.scope_line, None);
        assert_eq!(
            c.footnote_in(&chrono::Utc).as_deref(),
            Some("保存了 · 08:48")
        );
        if let Event::Confirm { scope, .. } = &mut skill {
            *scope = Some("user".into());
        }
        let c = ConfirmCard::from_event(&skill).unwrap();
        assert_eq!(c.scope_line.as_deref(), Some("所有 bro"));
        if let Event::Confirm {
            scope,
            bot_id,
            bot_name,
            ..
        } = &mut skill
        {
            *bot_id = "bot-ada".into();
            *bot_name = "Ada".into();
            *scope = Some("bot:bot-ada".into());
        }
        let c = ConfirmCard::from_event(&skill).unwrap();
        assert_eq!(c.scope_line.as_deref(), Some("只给 Ada"));
    }

    #[test]
    fn buttons_only_while_pending_and_not_after_a_409() {
        let mut c = ConfirmCard::from_event(&event("c1", None, None)).unwrap();
        let mut ui = CardUi::default();
        assert_eq!(confirm_actions(&c, &ui), Some(true));
        ui.busy = true;
        assert_eq!(
            confirm_actions(&c, &ui),
            Some(false),
            "greyed while in flight"
        );
        ui.busy = false;
        ui.note = Some(Note::AlreadyDecided);
        assert_eq!(confirm_actions(&c, &ui), None, "409: someone answered");
        ui.note = None;
        c.state = ConfirmState::Kept { by: None, at: 0 };
        assert_eq!(confirm_actions(&c, &ui), None);
    }

    #[test]
    fn more_than_three_pending_per_task_fold() {
        let open = HashSet::new();
        let rows = [
            Some(("t1", true)),
            None,
            Some(("t1", true)),
            Some(("t1", false)),
            Some(("t1", true)),
            Some(("t1", true)),
            Some(("t2", true)),
            Some(("t1", true)),
        ];
        let plan = fold_plan(rows, &open);
        assert_eq!(
            plan,
            [
                Fold::Show,
                Fold::Show,
                Fold::Show,
                Fold::Show, // answered: always shown, not counted
                Fold::Show,
                Fold::More {
                    task: "t1".into(),
                    n: 2
                },
                Fold::Show, // other task
                Fold::Hide,
            ]
        );
        assert_eq!(more_label(2), "还有 2 条待确认");
        let open: HashSet<String> = ["t1".to_string()].into();
        assert!(
            fold_plan(rows, &open).iter().all(|f| *f == Fold::Show),
            "tapped open"
        );
        let three = [Some(("t1", true)); 3];
        assert!(fold_plan(three, &HashSet::new())
            .iter()
            .all(|f| *f == Fold::Show));
    }
}
