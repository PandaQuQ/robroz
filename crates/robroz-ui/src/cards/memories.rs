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
    pub scope: String,
    pub kind: MemoryKind,
    pub text: String,
}

impl Default for NewMemory {
    fn default() -> Self {
        Self {
            scope: MemoryScope::User.to_string(),
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
    /// Last problem, shown at the top until the next action.
    pub note: Option<String>,
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
            self.note = None;
        }
    }

    pub fn edit_text(&mut self, text: String) {
        if let Some((_, draft)) = &mut self.editing {
            *draft = text;
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
        self.note = None;
    }

    /// The PATCH to send for the open edit: `(id, trimmed text)`, or a note.
    pub fn edit_to_save(&mut self) -> Option<(String, String)> {
        let (id, draft) = self.editing.clone()?;
        if let Some(p) = text_problem(&draft) {
            self.note = Some(p.to_string());
            return None;
        }
        Some((id, draft.trim().to_string()))
    }

    /// The host's row after PATCH replaces ours and closes the edit.
    pub fn saved(&mut self, memory: Memory) {
        if let Some(slot) = self.memories.iter_mut().find(|m| m.id == memory.id) {
            *slot = memory;
        }
        self.memories
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.editing = None;
        self.note = None;
    }

    /// First click on 忘掉: ask once.
    pub fn ask_forget(&mut self, id: &str) {
        self.confirm_forget = Some(id.to_string());
        self.editing = None;
        self.note = None;
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
        self.memories.retain(|m| m.id != id);
        if self.editing.as_ref().is_some_and(|(e, _)| e == id) {
            self.editing = None;
        }
    }

    pub fn open_add(&mut self) {
        self.adding = Some(NewMemory::default());
        self.editing = None;
        self.confirm_forget = None;
        self.note = None;
    }

    pub fn close_add(&mut self) {
        self.adding = None;
        self.note = None;
    }

    /// The POST to send for the form, or a note.
    pub fn add_to_send(&mut self) -> Option<NewMemory> {
        let new = self.adding.clone()?;
        if let Some(p) = text_problem(&new.text) {
            self.note = Some(p.to_string());
            return None;
        }
        Some(NewMemory {
            text: new.text.trim().to_string(),
            ..new
        })
    }

    /// POST succeeded: newest first, form closed.
    pub fn added(&mut self, memory: Memory) {
        self.memories.insert(0, memory);
        self.adding = None;
        self.note = None;
    }

    /// Scope choices for the form: 所有 bro, then 只给 each bro.
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
    fn inline_edit_checks_text_and_takes_the_hosts_row() {
        let mut b = book();
        b.start_edit("2");
        assert_eq!(b.editing.as_ref().unwrap().1, "记忆 2");
        b.edit_text("   ".into());
        assert_eq!(b.edit_to_save(), None);
        assert_eq!(b.note.as_deref(), Some(EMPTY_TEXT));
        b.edit_text("x".repeat(2001));
        assert_eq!(b.edit_to_save(), None);
        assert_eq!(b.note.as_deref(), Some(TOO_LONG));
        b.edit_text("  喜欢简短的周报 ".into());
        assert_eq!(
            b.edit_to_save(),
            Some(("2".into(), "喜欢简短的周报".into()))
        );
        let mut row = b.memories.iter().find(|m| m.id == "2").unwrap().clone();
        row.text = "喜欢简短的周报".into();
        row.updated_at = "2026-11-20T12:00:00Z".into();
        b.saved(row);
        assert_eq!(b.editing, None);
        assert_eq!(b.memories[0].id, "2", "newest first after the edit");
    }

    #[test]
    fn add_form_defaults_to_all_bros_and_inserts_on_top() {
        let mut b = book();
        b.open_add();
        let form = b.adding.as_mut().unwrap();
        assert_eq!(form.scope, "user");
        assert_eq!(form.kind, MemoryKind::Profile);
        form.text = "  我在伦敦  ".into();
        form.scope = "bot:bot-ada".into();
        let new = b.add_to_send().unwrap();
        assert_eq!(new.text, "我在伦敦");
        let mut row = mem(
            "9",
            &new.scope,
            new.kind,
            MemoryStatus::Kept,
            "2026-11-20T12:00:00Z",
        );
        row.text = new.text;
        b.added(row);
        assert_eq!(b.adding, None);
        assert_eq!(b.memories[0].id, "9");
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
