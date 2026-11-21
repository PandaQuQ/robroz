//! 技能 page view model (robroz#71 `/api/skills`): short instructions a
//! bro can follow, grouped by who may use them, with inline edit, a
//! one-step confirm before 删除, a small form to write one down, and a
//! per-group enable toggle.

use std::collections::HashSet;

use robroz_contract::{
    parse_skill_slug, MemoryScope, MemoryStatus, Skill, SKILL_INSTRUCTION_MAX, SKILL_TITLE_MAX,
};

use super::memories::{self, ALL_BROS, PICK_SCOPE};

pub const TITLE: &str = "技能";
pub const EMPTY_TITLE: &str = "还没有保存任何技能";
pub const EMPTY_HINT: &str = "点「加一项」写下想让 bro 会的做法";
pub const PENDING_TAG: &str = "待确认";
pub const PENDING_HEAD: &str = "待确认";
pub const KEPT_HEAD: &str = "已保存";
pub const ADD: &str = "加一项";
pub const EDIT: &str = "改";
pub const SAVE: &str = "保存";
pub const CANCEL: &str = "取消";
pub const DELETE: &str = "删除";
pub const DELETE_ASK: &str = "删除这项？bro 以后不会再用到它。";
pub const SLUG_PLACEHOLDER: &str = "英文短名，比如 weekly-report";
pub const TITLE_PLACEHOLDER: &str = "标题";
pub const INSTRUCTION_PLACEHOLDER: &str = "想让 bro 怎么做";
pub const EMPTY_SLUG: &str = "先填一个英文短名";
pub const BAD_SLUG: &str = "短名只能是小写字母、数字和连字符";
pub const EMPTY_TITLE_TEXT: &str = "写个标题再保存";
pub const EMPTY_INSTRUCTION: &str = "写点说明再保存";
pub const TITLE_TOO_LONG: &str = "标题最多 80 个字";
pub const INSTRUCTION_TOO_LONG: &str = "说明最多 8000 个字";
/// Shown next to the time right after a save.
pub const SAVED: &str = "已保存";
/// Section label above the per-group switches on a kept skill.
pub const GROUP_ENABLE: &str = "在这些群里用";
/// Skill instructions show this many lines until opened.
pub const INSTRUCTION_LINES: usize = 3;

/// Inline failure: 「没保存：<reason>」.
pub fn failed(reason: &str) -> String {
    format!("没保存：{reason}")
}

/// Re-export so the page uses the same scope wording as memories.
pub use memories::{chosen_scope_text, scope_label, updated_text, updated_text_in};

/// One scope's skills: pending apart from kept, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group<'a> {
    /// The wire scope (`user` or `bot:<id>`).
    pub scope: String,
    pub title: String,
    pub pending: Vec<&'a Skill>,
    pub kept: Vec<&'a Skill>,
}

impl Group<'_> {
    /// Sub-heads with counts, in page order: 「待确认 N」 when there are
    /// pending entries, and 「已保存 N」 whenever there are kept ones.
    pub fn heads(&self) -> Vec<(&'static str, usize)> {
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            out.push((PENDING_HEAD, self.pending.len()));
        }
        if !self.kept.is_empty() {
            out.push((KEPT_HEAD, self.kept.len()));
        }
        out
    }
}

/// The new-skill form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewSkill {
    /// `None` until the person picks one; 保存 stays grey until then.
    pub scope: Option<String>,
    pub slug: String,
    pub title: String,
    pub instruction: String,
}

/// Open inline edit: title and instruction drafts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSkill {
    pub slug: String,
    pub title: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    pub skills: Vec<Skill>,
    /// `(bot id, name)` in the host's order, for scope titles and the form.
    pub bros: Vec<(String, String)>,
    /// `(group id, name)` for the per-group enable toggles.
    pub chats: Vec<(String, String)>,
    /// Enabled `(group id, slug)` pairs known in this session.
    ///
    /// The host has no list of which groups have a skill on, so a freshly
    /// loaded page starts with every switch off. A successful toggle is
    /// remembered until the client exits.
    pub enabled: HashSet<(String, String)>,
    /// Slugs whose instruction is fully open.
    pub open: HashSet<String>,
    pub loaded: bool,
    pub editing: Option<EditSkill>,
    /// 删除 asked once, waiting for the second click (the slug).
    pub confirm_delete: Option<String>,
    pub adding: Option<NewSkill>,
    pub busy: bool,
    /// Page-level problem (the list did not load), shown at the top.
    pub note: Option<String>,
    /// A failed save, delete or toggle, shown under that entry: `(slug, text)`.
    pub entry_note: Option<(String, String)>,
    pub add_note: Option<String>,
    /// The entry saved last, marked 已保存 until the next action.
    pub just_saved: Option<String>,
}

/// Why a title cannot be saved, or `None` when it can.
pub fn title_problem(title: &str) -> Option<&'static str> {
    let t = title.trim();
    if t.is_empty() {
        Some(EMPTY_TITLE_TEXT)
    } else if t.chars().count() > SKILL_TITLE_MAX {
        Some(TITLE_TOO_LONG)
    } else {
        None
    }
}

/// Why an instruction cannot be saved, or `None` when it can.
pub fn instruction_problem(instruction: &str) -> Option<&'static str> {
    let t = instruction.trim();
    if t.is_empty() {
        Some(EMPTY_INSTRUCTION)
    } else if t.chars().count() > SKILL_INSTRUCTION_MAX {
        Some(INSTRUCTION_TOO_LONG)
    } else {
        None
    }
}

/// Why a slug cannot be saved, or `None` when it can.
pub fn slug_problem(slug: &str) -> Option<&'static str> {
    let t = slug.trim();
    if t.is_empty() {
        Some(EMPTY_SLUG)
    } else if parse_skill_slug(t).is_none() {
        Some(BAD_SLUG)
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
    pub fn note_for(&self, slug: &str) -> Option<&str> {
        self.entry_note
            .as_ref()
            .filter(|(e, _)| e == slug)
            .map(|(_, n)| n.as_str())
    }

    pub fn entry_failed(&mut self, slug: &str, reason: &str) {
        self.busy = false;
        self.entry_note = Some((slug.to_string(), failed(reason)));
    }

    /// 保存 / 不用 did not go through: the note under the entry, as given.
    pub fn decide_failed(&mut self, slug: &str, note: &str) {
        self.busy = false;
        self.entry_note = Some((slug.to_string(), note.to_string()));
    }

    pub fn add_failed(&mut self, reason: &str) {
        self.busy = false;
        self.add_note = Some(failed(reason));
    }

    /// 改 can save: title and instruction within the limits, and changed.
    pub fn can_save_edit(&self) -> bool {
        let Some(edit) = &self.editing else {
            return false;
        };
        let Some(s) = self.skills.iter().find(|s| s.slug == edit.slug) else {
            return false;
        };
        !self.busy
            && title_problem(&edit.title).is_none()
            && instruction_problem(&edit.instruction).is_none()
            && (edit.title.trim() != s.title.trim()
                || edit.instruction.trim() != s.instruction.trim())
    }

    /// 加一项 can save: a scope picked and every field within its limit.
    pub fn can_add(&self) -> bool {
        self.adding.as_ref().is_some_and(|n| {
            !self.busy
                && n.scope.is_some()
                && slug_problem(&n.slug).is_none()
                && title_problem(&n.title).is_none()
                && instruction_problem(&n.instruction).is_none()
        })
    }

    pub fn load(&mut self, mut skills: Vec<Skill>) {
        skills.retain(|s| s.status != MemoryStatus::Rejected);
        skills.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.skills = skills;
        self.loaded = true;
        if let Some(edit) = &self.editing {
            if !self.skills.iter().any(|s| s.slug == edit.slug) {
                self.editing = None;
            }
        }
        self.enabled
            .retain(|(_, slug)| self.skills.iter().any(|s| s.slug == *slug));
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Whether this skill is on in this group, as far as this session knows.
    pub fn is_enabled(&self, group: &str, slug: &str) -> bool {
        self.enabled
            .contains(&(group.to_string(), slug.to_string()))
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
        for s in &self.skills {
            let scope = match MemoryScope::parse(&s.scope) {
                Some(s) => s.to_string(),
                None => s.scope.clone(),
            };
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        scopes
            .into_iter()
            .filter_map(|scope| {
                let mine = |s: &&Skill| {
                    MemoryScope::parse(&s.scope)
                        .map(|v| v.to_string())
                        .as_deref()
                        == Some(scope.as_str())
                        || s.scope == scope
                };
                let pending: Vec<_> = self
                    .skills
                    .iter()
                    .filter(mine)
                    .filter(|s| s.status == MemoryStatus::Pending)
                    .collect();
                let kept: Vec<_> = self
                    .skills
                    .iter()
                    .filter(mine)
                    .filter(|s| s.status == MemoryStatus::Kept)
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

    pub fn start_edit(&mut self, slug: &str) {
        if let Some(s) = self.skills.iter().find(|s| s.slug == slug) {
            self.editing = Some(EditSkill {
                slug: slug.to_string(),
                title: s.title.clone(),
                instruction: s.instruction.clone(),
            });
            self.confirm_delete = None;
            self.clear_notes();
        }
    }

    pub fn edit_title(&mut self, title: String) {
        if let Some(edit) = &mut self.editing {
            edit.title = title;
        }
    }

    pub fn edit_instruction(&mut self, instruction: String) {
        if let Some(edit) = &mut self.editing {
            edit.instruction = instruction;
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
        self.clear_notes();
    }

    /// The PATCH to send: `(slug, title, instruction)`, or nothing.
    pub fn edit_to_save(&mut self) -> Option<(String, String, String)> {
        if !self.can_save_edit() {
            return None;
        }
        let edit = self.editing.clone()?;
        self.entry_note = None;
        Some((
            edit.slug,
            edit.title.trim().to_string(),
            edit.instruction.trim().to_string(),
        ))
    }

    /// The host's row after PATCH replaces ours and closes the edit.
    pub fn saved(&mut self, skill: Skill) {
        let slug = skill.slug.clone();
        if let Some(slot) = self.skills.iter_mut().find(|s| s.slug == skill.slug) {
            *slot = skill;
        }
        self.skills.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.busy = false;
        self.editing = None;
        self.clear_notes();
        self.just_saved = Some(slug);
    }

    /// First click on 删除: ask once.
    pub fn ask_delete(&mut self, slug: &str) {
        self.confirm_delete = Some(slug.to_string());
        self.editing = None;
        self.clear_notes();
    }

    pub fn cancel_delete(&mut self) {
        self.confirm_delete = None;
    }

    /// Second click: the slug to DELETE, if one was asked.
    pub fn delete_to_send(&mut self) -> Option<String> {
        self.confirm_delete.take()
    }

    /// 保存 / 不用 on a pending entry: the confirm card id to answer.
    pub fn decide_to_send(&mut self, slug: &str) -> Option<String> {
        if self.busy {
            return None;
        }
        let confirm = self
            .skills
            .iter()
            .find(|s| s.slug == slug && s.status == MemoryStatus::Pending)?
            .confirm_id
            .clone()?;
        self.clear_notes();
        self.busy = true;
        Some(confirm)
    }

    pub fn decided(&mut self, slug: &str, keep: bool) {
        self.busy = false;
        self.answered(slug, keep);
    }

    /// A pending entry was answered, here or on its chat card.
    pub fn answered(&mut self, slug: &str, keep: bool) {
        if !keep {
            self.skills.retain(|s| s.slug != slug);
            self.enabled.retain(|(_, s)| s != slug);
            return;
        }
        if let Some(s) = self.skills.iter_mut().find(|s| s.slug == slug) {
            s.status = MemoryStatus::Kept;
            s.confirm_id = None;
        }
    }

    pub fn deleted(&mut self, slug: &str) {
        self.busy = false;
        self.skills.retain(|s| s.slug != slug);
        self.enabled.retain(|(_, s)| s != slug);
        if self.editing.as_ref().is_some_and(|e| e.slug == slug) {
            self.editing = None;
        }
        self.open.remove(slug);
    }

    pub fn open_add(&mut self) {
        self.adding = Some(NewSkill::default());
        self.editing = None;
        self.confirm_delete = None;
        self.clear_notes();
    }

    pub fn close_add(&mut self) {
        self.adding = None;
        self.clear_notes();
    }

    /// The POST to send for the form, or nothing.
    pub fn add_to_send(&mut self) -> Option<NewSkill> {
        if !self.can_add() {
            return None;
        }
        let new = self.adding.clone()?;
        self.add_note = None;
        Some(NewSkill {
            scope: new.scope,
            slug: parse_skill_slug(&new.slug)
                .unwrap_or(new.slug.trim())
                .to_string(),
            title: new.title.trim().to_string(),
            instruction: new.instruction.trim().to_string(),
        })
    }

    /// POST succeeded: newest first, form closed, marked 已保存.
    pub fn added(&mut self, skill: Skill) {
        self.busy = false;
        let slug = skill.slug.clone();
        self.skills.insert(0, skill);
        self.adding = None;
        self.clear_notes();
        self.just_saved = Some(slug);
    }

    /// Flip the instruction open/closed for this slug.
    pub fn toggle_instruction(&mut self, slug: &str) {
        if !self.open.remove(slug) {
            self.open.insert(slug.to_string());
        }
    }

    /// The PUT (`true`) or DELETE (`false`) to send, or nothing if busy
    /// or the skill is not kept.
    pub fn toggle_to_send(&mut self, slug: &str, group: &str) -> Option<bool> {
        if self.busy {
            return None;
        }
        let kept = self
            .skills
            .iter()
            .any(|s| s.slug == slug && s.status == MemoryStatus::Kept);
        if !kept {
            return None;
        }
        self.clear_notes();
        self.busy = true;
        Some(!self.is_enabled(group, slug))
    }

    /// The host took the enable or disable.
    pub fn toggled(&mut self, slug: &str, group: &str, on: bool) {
        self.busy = false;
        let key = (group.to_string(), slug.to_string());
        if on {
            self.enabled.insert(key);
        } else {
            self.enabled.remove(&key);
        }
    }

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

    fn skill(
        slug: &str,
        scope: &str,
        status: MemoryStatus,
        at: &str,
        confirm: Option<&str>,
    ) -> Skill {
        Skill {
            id: format!("id-{slug}"),
            scope: scope.into(),
            slug: slug.into(),
            title: format!("标题 {slug}"),
            instruction: format!("说明 {slug}"),
            source: MemorySource::User,
            status,
            created_at: at.into(),
            updated_at: at.into(),
            confirm_id: confirm.map(Into::into),
        }
    }

    fn book() -> Book {
        let mut b = Book {
            bros: vec![
                ("bot-ada".into(), "Ada".into()),
                ("bot-bob".into(), "Bob".into()),
            ],
            chats: vec![
                ("g-team".into(), "作战组".into()),
                ("g-ada".into(), "Ada".into()),
            ],
            ..Book::default()
        };
        b.load(vec![
            skill(
                "bob-notes",
                "bot:bot-bob",
                MemoryStatus::Kept,
                "2026-11-20T09:00:00Z",
                None,
            ),
            skill(
                "shared",
                "user",
                MemoryStatus::Kept,
                "2026-11-20T08:00:00Z",
                None,
            ),
            skill(
                "ada-pending",
                "bot:bot-ada",
                MemoryStatus::Pending,
                "2026-11-20T10:00:00Z",
                Some("c-ada"),
            ),
            skill(
                "shared-pending",
                "user",
                MemoryStatus::Pending,
                "2026-11-20T07:00:00Z",
                Some("c-shared"),
            ),
            skill(
                "ada-notes",
                "bot:bot-ada",
                MemoryStatus::Kept,
                "2026-11-20T06:00:00Z",
                None,
            ),
            skill(
                "nope",
                "user",
                MemoryStatus::Rejected,
                "2026-11-20T11:00:00Z",
                None,
            ),
        ]);
        b
    }

    #[test]
    fn field_problems() {
        assert_eq!(slug_problem(""), Some(EMPTY_SLUG));
        assert_eq!(slug_problem("Weekly"), Some(BAD_SLUG));
        assert_eq!(slug_problem("weekly-report"), None);
        assert_eq!(title_problem("  "), Some(EMPTY_TITLE_TEXT));
        assert_eq!(title_problem(&"字".repeat(81)), Some(TITLE_TOO_LONG));
        assert_eq!(title_problem("写周报"), None);
        assert_eq!(instruction_problem(""), Some(EMPTY_INSTRUCTION));
        assert_eq!(
            instruction_problem(&"字".repeat(8001)),
            Some(INSTRUCTION_TOO_LONG)
        );
        assert_eq!(instruction_problem("先列三点"), None);
    }

    #[test]
    fn all_bros_first_then_each_bro_pending_apart_from_kept() {
        let b = book();
        let g = b.groups();
        let titles: Vec<_> = g.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["所有 bro", "只给 Ada", "只给 Bob"]);
        let slugs = |v: &Vec<&Skill>| v.iter().map(|s| s.slug.clone()).collect::<Vec<_>>();
        assert_eq!(slugs(&g[0].pending), ["shared-pending"]);
        assert_eq!(slugs(&g[0].kept), ["shared"]);
        assert_eq!(slugs(&g[1].pending), ["ada-pending"]);
        assert_eq!(slugs(&g[1].kept), ["ada-notes"]);
        assert_eq!(slugs(&g[2].kept), ["bob-notes"]);
        assert!(b.skills.iter().all(|s| s.status != MemoryStatus::Rejected));
    }

    #[test]
    fn kept_head_shows_even_without_pending() {
        let b = book();
        let groups = b.groups();
        let only_kept = groups
            .iter()
            .find(|g| g.pending.is_empty() && !g.kept.is_empty())
            .unwrap();
        assert_eq!(only_kept.heads(), [(KEPT_HEAD, only_kept.kept.len())]);
        let ada = groups.iter().find(|g| g.scope == "bot:bot-ada").unwrap();
        assert_eq!(ada.heads()[0], (PENDING_HEAD, ada.pending.len()));
        assert!(ada.heads().contains(&(KEPT_HEAD, ada.kept.len())));
    }

    #[test]
    fn empty_book_has_no_groups() {
        let mut b = Book::default();
        b.load(vec![]);
        assert!(b.is_empty() && b.loaded && b.groups().is_empty());
    }

    #[test]
    fn delete_needs_a_second_click_then_the_row_is_gone() {
        let mut b = book();
        assert_eq!(b.delete_to_send(), None, "nothing asked, nothing sent");
        b.ask_delete("shared");
        b.cancel_delete();
        assert_eq!(b.delete_to_send(), None);
        b.ask_delete("shared");
        assert_eq!(b.delete_to_send().as_deref(), Some("shared"));
        assert_eq!(b.confirm_delete, None);
        b.deleted("shared");
        assert!(b.skills.iter().all(|s| s.slug != "shared"));
    }

    #[test]
    fn edit_sends_trimmed_title_and_instruction() {
        let mut b = book();
        assert!(!b.can_save_edit());
        b.start_edit("shared");
        assert!(!b.can_save_edit(), "unchanged");
        b.edit_title("  共用  ".into());
        b.edit_instruction("  说明改了  ".into());
        assert!(b.can_save_edit());
        let (slug, title, instruction) = b.edit_to_save().unwrap();
        assert_eq!(
            (slug.as_str(), title.as_str(), instruction.as_str()),
            ("shared", "共用", "说明改了")
        );
        let mut row = skill(
            "shared",
            "user",
            MemoryStatus::Kept,
            "2026-11-20T12:00:00Z",
            None,
        );
        row.title = title;
        row.instruction = instruction;
        b.saved(row);
        assert_eq!(b.editing, None);
        assert_eq!(b.just_saved.as_deref(), Some("shared"));
        assert_eq!(b.skills[0].title, "共用");
    }

    #[test]
    fn add_needs_scope_slug_title_and_instruction() {
        let mut b = book();
        b.open_add();
        assert!(!b.can_add());
        b.adding.as_mut().unwrap().slug = "weekly-report".into();
        b.adding.as_mut().unwrap().title = "写周报".into();
        b.adding.as_mut().unwrap().instruction = "先列三点".into();
        assert!(!b.can_add(), "no scope yet");
        assert_eq!(b.add_to_send(), None);
        b.adding.as_mut().unwrap().scope = Some("bot:bot-ada".into());
        assert!(b.can_add());
        b.adding.as_mut().unwrap().slug = "Weekly".into();
        assert!(!b.can_add(), "bad slug");
        b.adding.as_mut().unwrap().slug = "  weekly-report  ".into();
        let new = b.add_to_send().unwrap();
        assert_eq!(new.slug, "weekly-report");
        let row = skill(
            &new.slug,
            new.scope.as_deref().unwrap(),
            MemoryStatus::Kept,
            "2026-11-20T12:00:00Z",
            None,
        );
        b.added(row);
        assert_eq!(b.adding, None);
        assert_eq!(b.skills[0].slug, "weekly-report");
        assert_eq!(b.just_saved.as_deref(), Some("weekly-report"));
        let choices: Vec<_> = b.scope_choices().into_iter().map(|(_, l)| l).collect();
        assert_eq!(choices, ["所有 bro", "只给 Ada", "只给 Bob"]);
    }

    #[test]
    fn pending_decide_and_chat_answer() {
        let mut b = book();
        assert_eq!(b.decide_to_send("shared"), None, "kept has no card");
        assert_eq!(b.decide_to_send("ada-pending").as_deref(), Some("c-ada"));
        assert!(b.busy);
        b.decided("ada-pending", true);
        let ada = b.skills.iter().find(|s| s.slug == "ada-pending").unwrap();
        assert_eq!(ada.status, MemoryStatus::Kept);
        assert_eq!(ada.confirm_id, None);
        b.answered("shared-pending", false);
        assert!(b.skills.iter().all(|s| s.slug != "shared-pending"));
    }

    #[test]
    fn toggle_is_off_until_a_successful_put() {
        let mut b = book();
        assert!(!b.is_enabled("g-team", "shared"));
        assert_eq!(b.toggle_to_send("shared", "g-team"), Some(true));
        assert!(b.busy);
        b.toggled("shared", "g-team", true);
        assert!(b.is_enabled("g-team", "shared"));
        assert_eq!(b.toggle_to_send("shared", "g-team"), Some(false));
        b.toggled("shared", "g-team", false);
        assert!(!b.is_enabled("g-team", "shared"));
        assert_eq!(b.toggle_to_send("ada-pending", "g-team"), None, "pending");
        b.enabled.insert(("g-team".into(), "shared".into()));
        b.deleted("shared");
        assert!(!b.is_enabled("g-team", "shared"));
    }

    #[test]
    fn instruction_open_toggles_and_clears_on_delete() {
        let mut b = book();
        b.toggle_instruction("shared");
        assert!(b.open.contains("shared"));
        b.toggle_instruction("shared");
        assert!(!b.open.contains("shared"));
        b.toggle_instruction("shared");
        b.deleted("shared");
        assert!(!b.open.contains("shared"));
    }
}
