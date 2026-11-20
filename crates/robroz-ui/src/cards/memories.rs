//! 记忆 page view model (robroz#62 `/api/memories`): what each bro may
//! remember about the person, grouped by who may use it, with inline edit,
//! a one-step confirm before 忘掉, and a small form to write one down.

use chrono::{DateTime, Local, TimeZone};
use robroz_contract::{Memory, MemoryKind, MemoryScope, MemoryStatus};

pub const TITLE: &str = "记忆";
pub const EMPTY_TITLE: &str = "还没有记下任何东西";
pub const EMPTY_HINT: &str = "点「记一条」写下想让 bro 记住的事";
pub const ALL_BROS: &str = "所有 bro";
pub const PENDING_TAG: &str = "待确认";
pub const PENDING_HEAD: &str = "待确认";
pub const KEPT_HEAD: &str = "已记下";
pub const ADD: &str = "记一条";
pub const EDIT: &str = "改";
pub const SAVE: &str = "保存";
pub const CANCEL: &str = "取消";
pub const FORGET: &str = "忘掉";
pub const FORGET_ASK: &str = "忘掉这条？bro 以后不会再用到它。";
pub const NEW_PLACEHOLDER: &str = "想让 bro 记住什么";
pub const TOO_LONG: &str = "最多 2000 字";
pub const EMPTY_TEXT: &str = "写点什么再保存";
/// Shown next to the time right after a save.
pub const SAVED: &str = "已保存";
/// Under the scope choices until one is picked.
pub const PICK_SCOPE: &str = "选一下谁能用这条";
/// Inline failure: 「没保存：<reason>」.
pub fn failed(reason: &str) -> String {
    format!("没保存：{reason}")
}

/// 资料 / 习惯 / 项目.
pub fn kind_label(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Profile => "资料",
        MemoryKind::Habit => "习惯",
        MemoryKind::Project => "项目",
    }
}

/// 「所有 bro」 or 「只给 Ada」. An unknown bro shows its id.
pub fn scope_label(scope: &str, bros: &[(String, String)]) -> String {
    match MemoryScope::parse(scope) {
        Some(MemoryScope::Bot(id)) => {
            let name = bros
                .iter()
                .find(|(b, _)| *b == id)
                .map_or(id.as_str(), |(_, n)| n.as_str());
            format!("只给 {name}")
        }
        _ => ALL_BROS.to_string(),
    }
}

/// The add form's hint row once a scope is picked: 「所有 bro 都能用」
/// or 「只给 Bob 用」. Same row as [`PICK_SCOPE`], so 保存 never moves.
pub fn chosen_scope_text(scope: &str, bros: &[(String, String)]) -> String {
    match MemoryScope::parse(scope) {
        Some(MemoryScope::Bot(_)) => format!("{} 用", scope_label(scope, bros)),
        _ => format!("{ALL_BROS} 都能用"),
    }
}

/// Updated time in local time: `11月20日 10:31`, or `今天 10:31`.
pub fn updated_text_in<Tz: TimeZone>(at: &str, now: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Ok(t) = DateTime::parse_from_rfc3339(at) else {
        return String::new();
    };
    let local = t.with_timezone(tz);
    let today = tz
        .timestamp_opt(now, 0)
        .single()
        .map(|n| n.date_naive() == local.date_naive())
        .unwrap_or(false);
    if today {
        local.format("今天 %H:%M").to_string()
    } else {
        local.format("%-m月%-d日 %H:%M").to_string()
    }
}

pub fn updated_text(at: &str, now: i64) -> String {
    updated_text_in(at, now, &Local)
}

/// One scope's memories: pending apart from kept, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group<'a> {
    /// The wire scope (`user` or `bot:<id>`).
    pub scope: String,
    pub title: String,
    pub pending: Vec<&'a Memory>,
    pub kept: Vec<&'a Memory>,
}

/// The new-memory form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMemory {
    /// `None` until the person picks one; 保存 stays grey until then.
    pub scope: Option<String>,
    pub kind: MemoryKind,
    pub text: String,
}

impl Default for NewMemory {
    fn default() -> Self {
        Self {
            scope: None,
            kind: MemoryKind::Profile,
            text: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    pub memories: Vec<Memory>,
    /// `(bot id, name)` in the host's order, for scope titles and the form.
    pub bros: Vec<(String, String)>,
    pub loaded: bool,
    /// Inline edit: `(id, draft)`.
    pub editing: Option<(String, String)>,
    /// 忘掉 asked once, waiting for the second click.
    pub confirm_forget: Option<String>,
    /// The 记一条 form, when open.
    pub adding: Option<NewMemory>,
    /// A request is in flight.
    pub busy: bool,
    /// Page-level problem (the list did not load), shown at the top.
    pub note: Option<String>,
    /// A failed save or forget, shown under that entry: `(id, text)`.
    pub entry_note: Option<(String, String)>,
    /// A failed 记一条, shown in the form.
    pub add_note: Option<String>,
    /// The entry saved last, marked 已保存 until the next action.
    pub just_saved: Option<String>,
}

/// Why a text cannot be saved, or `None` when it can.
pub fn text_problem(text: &str) -> Option<&'static str> {
    let t = text.trim();
    if t.is_empty() {
        Some(EMPTY_TEXT)
    } else if t.chars().count() > robroz_contract::MEMORY_TEXT_MAX {
        Some(TOO_LONG)
    } else {
        None
    }
}

impl Book {
    fn clear_notes(&mut self) {
        self.note = None;
        self.entry_note = None;
        self.add_note = None;
        self.just_saved = None;
    }

    /// The inline note under this entry, if its last request failed.
    pub fn note_for(&self, id: &str) -> Option<&str> {
        self.entry_note
            .as_ref()
            .filter(|(e, _)| e == id)
            .map(|(_, n)| n.as_str())
    }

    /// A save or forget of this entry failed: say so under it.
    pub fn entry_failed(&mut self, id: &str, reason: &str) {
        self.busy = false;
        self.entry_note = Some((id.to_string(), failed(reason)));
    }

    /// 记一条 failed: say so in the form and keep what was typed.
    pub fn add_failed(&mut self, reason: &str) {
        self.busy = false;
        self.add_note = Some(failed(reason));
    }

    /// 改 can save: text not empty, within the limit, and changed.
    pub fn can_save_edit(&self) -> bool {
        let Some((id, draft)) = &self.editing else {
            return false;
        };
        let Some(m) = self.memories.iter().find(|m| &m.id == id) else {
            return false;
        };
        !self.busy && text_problem(draft).is_none() && draft.trim() != m.text.trim()
    }

    /// 记一条 can save: a scope picked and some text within the limit.
    pub fn can_add(&self) -> bool {
        self.adding
            .as_ref()
            .is_some_and(|n| !self.busy && n.scope.is_some() && text_problem(&n.text).is_none())
    }

    pub fn load(&mut self, mut memories: Vec<Memory>) {
        // Rejected ones are never listed; the host leaves them out by default.
        memories.retain(|m| m.status != MemoryStatus::Rejected);
        memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.memories = memories;
        self.loaded = true;
        if let Some((id, _)) = &self.editing {
            if !self.memories.iter().any(|m| &m.id == id) {
                self.editing = None;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.memories.is_empty()
    }

    /// 「所有 bro」 first, then one group per bro in the host's bro order,
    /// then bros the host no longer lists. Empty scopes are left out.
    pub fn groups(&self) -> Vec<Group<'_>> {
        let mut scopes: Vec<String> = vec![MemoryScope::User.to_string()];
        scopes.extend(
            self.bros
                .iter()
                .map(|(id, _)| MemoryScope::for_bot(id).to_string()),
        );
        for m in &self.memories {
            let s = match MemoryScope::parse(&m.scope) {
                Some(s) => s.to_string(),
                None => m.scope.clone(),
            };
            if !scopes.contains(&s) {
                scopes.push(s);
            }
        }
        scopes
            .into_iter()
            .filter_map(|scope| {
                let mine = |m: &&Memory| {
                    MemoryScope::parse(&m.scope)
                        .map(|s| s.to_string())
                        .as_deref()
                        == Some(scope.as_str())
                        || m.scope == scope
                };
                let pending: Vec<_> = self
                    .memories
                    .iter()
                    .filter(mine)
                    .filter(|m| m.status == MemoryStatus::Pending)
                    .collect();
                let kept: Vec<_> = self
                    .memories
                    .iter()
                    .filter(mine)
                    .filter(|m| m.status == MemoryStatus::Kept)
                    .collect();
                (!pending.is_empty() || !kept.is_empty()).then(|| Group {
                    title: scope_label(&scope, &self.bros),
                    scope,
                    pending,
                    kept,
                })
            })
            .collect()
    }

    pub fn start_edit(&mut self, id: &str) {
        if let Some(m) = self.memories.iter().find(|m| m.id == id) {
            self.editing = Some((id.to_string(), m.text.clone()));
            self.confirm_forget = None;
            self.clear_notes();
        }
    }

    pub fn edit_text(&mut self, text: String) {
        if let Some((_, draft)) = &mut self.editing {
            *draft = text;
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
        self.clear_notes();
    }

    /// The PATCH to send for the open edit: `(id, trimmed text)`, or a note.
    pub fn edit_to_save(&mut self) -> Option<(String, String)> {
        if !self.can_save_edit() {
            return None;
        }
        let (id, draft) = self.editing.clone()?;
        self.entry_note = None;
        Some((id, draft.trim().to_string()))
    }

    /// The host's row after PATCH replaces ours and closes the edit.
    pub fn saved(&mut self, memory: Memory) {
        let id = memory.id.clone();
        if let Some(slot) = self.memories.iter_mut().find(|m| m.id == memory.id) {
            *slot = memory;
        }
        self.memories
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.busy = false;
        self.editing = None;
        self.clear_notes();
        self.just_saved = Some(id);
    }

    /// First click on 忘掉: ask once.
    pub fn ask_forget(&mut self, id: &str) {
        self.confirm_forget = Some(id.to_string());
        self.editing = None;
        self.clear_notes();
    }

    pub fn cancel_forget(&mut self) {
        self.confirm_forget = None;
    }

    /// Second click: the id to DELETE, if one was asked.
    pub fn forget_to_send(&mut self) -> Option<String> {
        self.confirm_forget.take()
    }

    /// DELETE succeeded (204, also when it was already gone): drop it.
    pub fn forgotten(&mut self, id: &str) {
        self.busy = false;
        self.memories.retain(|m| m.id != id);
        if self.editing.as_ref().is_some_and(|(e, _)| e == id) {
            self.editing = None;
        }
    }

    pub fn open_add(&mut self) {
        self.adding = Some(NewMemory::default());
        self.editing = None;
        self.confirm_forget = None;
        self.clear_notes();
    }

    pub fn close_add(&mut self) {
        self.adding = None;
        self.clear_notes();
    }

    /// The POST to send for the form, or a note.
    pub fn add_to_send(&mut self) -> Option<NewMemory> {
        if !self.can_add() {
            return None;
        }
        let new = self.adding.clone()?;
        self.add_note = None;
        Some(NewMemory {
            text: new.text.trim().to_string(),
            ..new
        })
    }

    /// POST succeeded: newest first, form closed, marked 已保存.
    pub fn added(&mut self, memory: Memory) {
        self.busy = false;
        let id = memory.id.clone();
        self.memories.insert(0, memory);
        self.adding = None;
        self.clear_notes();
        self.just_saved = Some(id);
    }

    /// Scope choices for the form: 所有 bro, then 只给 each bro.
    /// The add form's hint row: [`PICK_SCOPE`] until a scope is picked,
    /// then the picked result. `None` when the form is closed.
    pub fn add_scope_hint(&self) -> Option<String> {
        let new = self.adding.as_ref()?;
        Some(match &new.scope {
            None => PICK_SCOPE.to_string(),
            Some(scope) => chosen_scope_text(scope, &self.bros),
        })
    }

    pub fn scope_choices(&self) -> Vec<(String, String)> {
        let mut out = vec![(MemoryScope::User.to_string(), ALL_BROS.to_string())];
        out.extend(
            self.bros
                .iter()
                .map(|(id, name)| (MemoryScope::for_bot(id).to_string(), format!("只给 {name}"))),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::MemorySource;

    fn mem(id: &str, scope: &str, kind: MemoryKind, status: MemoryStatus, at: &str) -> Memory {
        Memory {
            id: id.into(),
            scope: scope.into(),
            kind,
            text: format!("记忆 {id}"),
            source: MemorySource::User,
            status,
            created_at: at.into(),
            updated_at: at.into(),
            confirm_id: None,
        }
    }

    fn book() -> Book {
        let mut b = Book {
            bros: vec![
                ("bot-ada".into(), "Ada".into()),
                ("bot-bob".into(), "Bob".into()),
            ],
            ..Book::default()
        };
        b.load(vec![
            mem(
                "1",
                "bot:bot-bob",
                MemoryKind::Habit,
                MemoryStatus::Kept,
                "2026-11-20T09:00:00Z",
            ),
            mem(
                "2",
                "user",
                MemoryKind::Profile,
                MemoryStatus::Kept,
                "2026-11-20T08:00:00Z",
            ),
            mem(
                "3",
                "bot:bot-ada",
                MemoryKind::Project,
                MemoryStatus::Pending,
                "2026-11-20T10:00:00Z",
            ),
            mem(
                "4",
                "user",
                MemoryKind::Habit,
                MemoryStatus::Pending,
                "2026-11-20T07:00:00Z",
            ),
            mem(
                "5",
                "bot:bot-ada",
                MemoryKind::Habit,
                MemoryStatus::Kept,
                "2026-11-20T06:00:00Z",
            ),
            mem(
                "6",
                "user",
                MemoryKind::Project,
                MemoryStatus::Rejected,
                "2026-11-20T11:00:00Z",
            ),
        ]);
        b
    }

    #[test]
    fn labels() {
        assert_eq!(kind_label(MemoryKind::Profile), "资料");
        assert_eq!(kind_label(MemoryKind::Habit), "习惯");
        assert_eq!(kind_label(MemoryKind::Project), "项目");
        let bros = vec![("bot-ada".to_string(), "Ada".to_string())];
        assert_eq!(scope_label("user", &bros), "所有 bro");
        assert_eq!(scope_label("bot:bot-ada", &bros), "只给 Ada");
        assert_eq!(scope_label("bot:bot-zed", &bros), "只给 bot-zed");
    }

    #[test]
    fn all_bros_first_then_each_bro_pending_apart_from_kept() {
        let b = book();
        let g = b.groups();
        let titles: Vec<_> = g.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["所有 bro", "只给 Ada", "只给 Bob"]);
        let ids = |v: &Vec<&Memory>| v.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&g[0].pending), ["4"]);
        assert_eq!(ids(&g[0].kept), ["2"]);
        assert_eq!(ids(&g[1].pending), ["3"]);
        assert_eq!(ids(&g[1].kept), ["5"]);
        assert_eq!(ids(&g[2].kept), ["1"]);
        assert!(b
            .memories
            .iter()
            .all(|m| m.status != MemoryStatus::Rejected));
    }

    #[test]
    fn empty_book_has_no_groups() {
        let mut b = Book::default();
        b.load(vec![]);
        assert!(b.is_empty() && b.loaded && b.groups().is_empty());
    }

    #[test]
    fn forget_needs_a_second_click_then_the_row_is_gone() {
        let mut b = book();
        assert_eq!(b.forget_to_send(), None, "nothing asked, nothing sent");
        b.ask_forget("2");
        b.cancel_forget();
        assert_eq!(b.forget_to_send(), None);
        b.ask_forget("2");
        assert_eq!(b.forget_to_send().as_deref(), Some("2"));
        assert_eq!(b.confirm_forget, None);
        b.forgotten("2");
        assert!(!b.memories.iter().any(|m| m.id == "2"));
    }

    #[test]
    fn edit_save_is_grey_until_the_text_changes_and_marks_saved() {
        let mut b = book();
        assert!(!b.can_save_edit(), "nothing open");
        b.start_edit("2");
        assert_eq!(b.editing.as_ref().unwrap().1, "记忆 2");
        assert!(!b.can_save_edit(), "unchanged");
        assert_eq!(b.edit_to_save(), None);
        b.edit_text("  记忆 2 ".into());
        assert!(!b.can_save_edit(), "only spaces changed");
        b.edit_text("   ".into());
        assert!(!b.can_save_edit(), "empty");
        b.edit_text("x".repeat(2001));
        assert!(!b.can_save_edit(), "too long");
        b.edit_text("  喜欢简短的周报 ".into());
        assert!(b.can_save_edit());
        assert_eq!(
            b.edit_to_save(),
            Some(("2".into(), "喜欢简短的周报".into()))
        );
        b.busy = true;
        assert!(!b.can_save_edit(), "grey while in flight");
        let mut row = b.memories.iter().find(|m| m.id == "2").unwrap().clone();
        row.text = "喜欢简短的周报".into();
        row.updated_at = "2026-11-20T12:00:00Z".into();
        b.saved(row);
        assert!(!b.busy);
        assert_eq!(b.editing, None);
        assert_eq!(b.memories[0].id, "2", "newest first after the edit");
        assert_eq!(
            b.memories[0].updated_at, "2026-11-20T12:00:00Z",
            "time is the host's new one"
        );
        assert_eq!(b.just_saved.as_deref(), Some("2"), "marked 已保存");
        b.start_edit("1");
        assert_eq!(b.just_saved, None, "the mark goes with the next action");
    }

    #[test]
    fn failed_save_stays_open_with_an_inline_note() {
        let mut b = book();
        b.start_edit("2");
        b.edit_text("新的".into());
        let _ = b.edit_to_save().unwrap();
        b.busy = true;
        b.entry_failed("2", "连不上主机");
        assert!(!b.busy);
        assert_eq!(b.note_for("2"), Some("没保存：连不上主机"));
        assert_eq!(b.note_for("1"), None);
        assert_eq!(
            b.editing.as_ref().unwrap().1,
            "新的",
            "draft kept for a retry"
        );

        b.open_add();
        b.adding.as_mut().unwrap().scope = Some("user".into());
        b.adding.as_mut().unwrap().text = "x".into();
        let _ = b.add_to_send().unwrap();
        b.add_failed("scope 只能是 user 或 bot:<botId>");
        assert_eq!(
            b.add_note.as_deref(),
            Some("没保存：scope 只能是 user 或 bot:<botId>")
        );
        assert_eq!(b.adding.as_ref().unwrap().text, "x", "typed text kept");
    }

    #[test]
    fn add_form_hint_row_stays_and_shows_the_picked_scope() {
        let mut b = book();
        assert_eq!(b.add_scope_hint(), None, "form closed");
        b.open_add();
        assert_eq!(b.add_scope_hint().as_deref(), Some(PICK_SCOPE));
        b.adding.as_mut().unwrap().scope = Some("user".into());
        assert_eq!(b.add_scope_hint().as_deref(), Some("所有 bro 都能用"));
        b.adding.as_mut().unwrap().scope = Some("bot:bot-bob".into());
        assert_eq!(b.add_scope_hint().as_deref(), Some("只给 Bob 用"));
        b.adding.as_mut().unwrap().scope = Some("bot:bot-zed".into());
        assert_eq!(
            b.add_scope_hint().as_deref(),
            Some("只给 bot-zed 用"),
            "unknown bot falls back to its id"
        );
    }

    #[test]
    fn add_needs_a_scope_and_text_then_inserts_on_top() {
        let mut b = book();
        assert!(!b.can_add(), "form closed");
        b.open_add();
        assert_eq!(
            b.adding.as_ref().unwrap().scope,
            None,
            "no scope until picked"
        );
        b.adding.as_mut().unwrap().text = "  我在伦敦  ".into();
        assert!(!b.can_add(), "no scope yet");
        assert_eq!(b.add_to_send(), None);
        b.adding.as_mut().unwrap().scope = Some("bot:bot-ada".into());
        assert!(b.can_add());
        b.adding.as_mut().unwrap().text = " ".into();
        assert!(!b.can_add(), "empty text");
        b.adding.as_mut().unwrap().text = "  我在伦敦  ".into();
        let new = b.add_to_send().unwrap();
        assert_eq!(new.text, "我在伦敦");
        let scope = new.scope.clone().unwrap();
        let mut row = mem(
            "9",
            &scope,
            new.kind,
            MemoryStatus::Kept,
            "2026-11-20T12:00:00Z",
        );
        row.text = new.text;
        b.added(row);
        assert_eq!(b.adding, None);
        assert_eq!(b.memories[0].id, "9");
        assert_eq!(b.just_saved.as_deref(), Some("9"));
        let choices: Vec<_> = b.scope_choices().into_iter().map(|(_, l)| l).collect();
        assert_eq!(choices, ["所有 bro", "只给 Ada", "只给 Bob"]);
    }

    #[test]
    fn updated_time_is_local() {
        let tz = chrono::FixedOffset::east_opt(0).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-11-20T15:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            updated_text_in("2026-11-20T10:31:00.000Z", now, &tz),
            "今天 10:31"
        );
        assert_eq!(
            updated_text_in("2026-11-19T22:05:00Z", now, &tz),
            "11月19日 22:05"
        );
        assert_eq!(updated_text_in("bad", now, &tz), "");
    }
}
