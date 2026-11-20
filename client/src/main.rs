//! robroz desktop client: an IM for bros.
//!
//! Left: the conversation list (groups and bro DMs, newest first, unread
//! badges, 「＋ 新 bro」) and 例行任务. Right: the open chat's message stream
//! or the routines panel. At ≤480 wide the list and the chat each take the
//! whole window (tap in, 「‹ 聊天」 back).
//!
//! Events: the open chat streams live over SSE ([`source::stream`]); every
//! chat is also polled incrementally with `?since=` — every tick while the
//! stream is down, as a slow safety net while it is up. Unread counts come
//! from `GET /api/unread`; opening a chat marks it read. `--fake` /
//! `ROBROZ_FAKE=1` runs offline on a script. Card clicks go through
//! [`api::CardApi`]. `--gallery` shows every card state for design review.
//! Visuals come from the `robroz-ui` skin (`ROBROZ_THEME=dark` starts dark;
//! the footer button toggles).

mod api;
mod conn;
mod gallery;
mod model;
mod source;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{window, Alignment, Element, Fill, Font, Length, Padding, Size, Subscription, Task};
use robroz_contract::{ApprovalDecision, Bro, Event, Memory, Said, ScheduleEntry, Unread};
use robroz_ui::cards::confirm::{self, ConfirmState, Fold};
use robroz_ui::cards::interact::{CardUi, Note};
use robroz_ui::cards::memories::Book;
use robroz_ui::cards::routines::{RoutineRow, Routines};
use robroz_ui::cards::{ApprovalState, Card, ChoiceState, DraftState};
use robroz_ui::conversations::Conversations;
use robroz_ui::view::{self, color, Action, Caps};
use robroz_ui::{Mode, Skin};

use api::{ApiError, CardApi, FakeApi, HttpApi};
use conn::Conn;
use gallery::Gallery;
use model::{attachment_label, change_time, preview, DeliveryCard, Feed, Row, StatusCard};
use source::stream::{self, StreamItem};
use source::{
    fake_chats, fake_schedules, EventSource, FakeSource, GroupCard, GroupEvent, GroupTab,
    HttpSource, SourceError,
};

/// How often the conversation list refreshes unread counts.
const UNREAD_EVERY: Duration = Duration::from_secs(3);

pub fn main() -> iced::Result {
    let size = env_size().unwrap_or((1040.0, 680.0));
    iced::application(App::boot, App::update, App::view)
        .title("robroz")
        .subscription(App::subscription)
        .theme(App::theme)
        .scale_factor(App::scale_factor)
        .window_size(size)
        .run()
}

fn has_flag(flag: &str) -> bool {
    std::env::args().any(|a| a == flag)
}

fn wants_fake() -> bool {
    has_flag("--fake") || std::env::var("ROBROZ_FAKE").is_ok_and(|v| v == "1")
}

/// `ROBROZ_WINDOW=WxH` (logical px), mainly for gallery screenshots.
fn env_size() -> Option<(f32, f32)> {
    let v = std::env::var("ROBROZ_WINDOW").ok()?;
    let (w, h) = v.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// This client's name: sent as `answeredBy` when it answers a card, and
/// shown as 「我」 when a card comes back with it. `ROBROZ_USER_NAME`, else
/// the computer's hostname, else `robroz`. Two devices must not share it, or
/// each shows the other's answers as 「我」.
fn user_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| pick_name(std::env::var("ROBROZ_USER_NAME").ok(), hostname()))
}

/// First usable of `env` and `host`, cut to what the host stores
/// (one line, at most `ANSWERED_BY_MAX` characters); else `robroz`.
fn pick_name(env: Option<String>, host: Option<String>) -> String {
    [env, host]
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
            Some(
                line.chars()
                    .take(robroz_contract::ANSWERED_BY_MAX)
                    .collect::<String>(),
            )
        })
        .find_map(|n| robroz_contract::clean_answered_by(Some(&n)).ok().flatten())
        .unwrap_or_else(|| "robroz".into())
}

/// The computer's hostname, if it can be read (`hostname` exists on
/// Linux, macOS and Windows).
fn hostname() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
}

type Shared = Arc<Mutex<Box<dyn EventSource>>>;

#[derive(Debug, Clone)]
struct Polled {
    events: Vec<GroupEvent>,
    cards: Vec<GroupCard>,
    has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Group,
    Routines,
    Memories,
}

#[derive(Debug, Clone)]
enum Message {
    OpenRoutines,
    OpenMemories,
    MemoriesLoaded(Result<Vec<Memory>, ApiError>),
    /// PATCH (edit) or POST (new) answered.
    MemorySaved {
        /// `None` for 记一条, the memory id for 改.
        id: Option<String>,
        result: Result<Memory, ApiError>,
    },
    MemoryForgot {
        id: String,
        result: Result<(), ApiError>,
    },
    /// 记下 / 不用 from the 记忆 page, answered on the entry's confirm card.
    MemoryDecided {
        id: String,
        keep: bool,
        result: Result<Option<Event>, ApiError>,
    },
    Tick,
    Loaded(Result<Vec<GroupTab>, SourceError>),
    Polled(Result<Polled, SourceError>),
    TakeOver(String),
    Ui(Action),
    Answered {
        card: String,
        sent: Vec<String>,
        result: Result<Option<Event>, ApiError>,
    },
    Acted {
        card: String,
        result: Result<(), ApiError>,
    },
    /// Approval answered (or refused) by the host.
    Decided {
        card: String,
        approve: bool,
        result: Result<Option<Event>, ApiError>,
    },
    RoutinesLoaded(Result<Vec<ScheduleEntry>, ApiError>),
    Paused {
        id: String,
        wanted: bool,
        result: Result<ScheduleEntry, ApiError>,
    },
    ClearPauseNote(String),
    UnreadTick,
    UnreadLoaded(Result<Vec<Unread>, ApiError>),
    MarkedRead(String, Result<Unread, ApiError>),
    BrosLoaded(Result<Vec<Bro>, ApiError>),
    BroCreated(Result<Bro, ApiError>),
    GroupsReloaded(Result<Vec<GroupTab>, SourceError>),
    Live(stream::Live),
    /// Rename result for the bro behind DM `group`.
    Renamed {
        group: String,
        result: Result<Bro, ApiError>,
    },
    /// Composer result from `/say`: the stored message (+ task, if any).
    Sent {
        group: String,
        result: Result<Said, ApiError>,
    },
    /// Narrow screens: back from a chat to the list.
    Back,
    ToggleTheme,
    Resized(Size),
    TouchSeen,
    Shoot(Option<window::Id>),
    Shot(window::Screenshot),
}

struct App {
    source: Shared,
    api: Arc<dyn CardApi>,
    /// Which card buttons the backend can serve (drafts: fake only).
    caps: Caps,
    /// Live host connection for the SSE stream; `None` in fake mode.
    conn: Option<Conn>,
    convs: Conversations,
    /// The open chat's stream is connected (polling slows down).
    stream_up: bool,
    /// Narrow screens: a chat (or routines) is open over the list.
    narrow_open: bool,
    /// Known bros (DM owners, the rename box).
    bros: Vec<Bro>,
    /// Composer text per chat.
    drafts: HashMap<String, String>,
    sending: bool,
    /// Last composer problem, per chat.
    send_note: HashMap<String, String>,
    /// `--fake` / gallery: canned data, test-only env knobs allowed.
    fake: bool,
    /// Bro rename in progress: `(dm group, draft)`.
    rename: Option<(String, String)>,
    renaming: bool,
    /// Last rename error: `(dm group, note)`.
    rename_note: Option<(String, String)>,
    origin: String,
    interval: Duration,
    groups: Vec<GroupTab>,
    feeds: HashMap<String, Feed>,
    /// Change time and rank of the event behind each chat's list preview.
    preview_key: HashMap<String, (String, u8)>,
    selected: Option<String>,
    page: Page,
    ui: HashMap<String, CardUi>,
    /// Tasks whose folded confirm cards were opened (「还有 N 条待确认」).
    unfolded: HashSet<String>,
    routines: Routines,
    /// 记忆 page (robroz#62).
    memories: Book,
    mode: Mode,
    width: f32,
    loaded: bool,
    in_flight: bool,
    has_more: bool,
    error: Option<String>,
    gallery: Option<Gallery>,
    scale: f32,
    /// Touch input seen (or `ROBROZ_TOUCH=1`): raises hit areas to the floor.
    touch: bool,
}

/// Run a blocking source call off the UI thread.
fn blocking<T: Send + 'static>(
    source: &Shared,
    f: impl FnOnce(&mut dyn EventSource) -> T + Send + 'static,
) -> impl std::future::Future<Output = T> {
    let source = source.clone();
    async move {
        tokio::task::spawn_blocking(move || {
            let mut guard = source.lock().expect("event source lock poisoned");
            f(guard.as_mut())
        })
        .await
        .expect("event source task panicked")
    }
}

/// Run a blocking API call off the UI thread.
fn call<T: Send + 'static>(
    api: &Arc<dyn CardApi>,
    f: impl FnOnce(&dyn CardApi) -> T + Send + 'static,
) -> impl std::future::Future<Output = T> {
    let api = api.clone();
    async move {
        tokio::task::spawn_blocking(move || f(api.as_ref()))
            .await
            .expect("api task panicked")
    }
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let gallery = has_flag("--gallery").then(Gallery::new);
        let fake = wants_fake() || gallery.is_some();
        type Parts = (Box<dyn EventSource>, Arc<dyn CardApi>, String, Option<Conn>);
        let (source, api, origin, conn): Parts = if fake {
            let (unread, bros) = fake_chats();
            (
                Box::new(FakeSource::new()),
                Arc::new(FakeApi::new(fake_schedules()).with_chats(unread, bros)),
                "假事件".into(),
                None,
            )
        } else {
            let http = HttpSource::from_env();
            let conn = http.conn().clone();
            let origin = http.base().to_string();
            let api = Arc::new(HttpApi::new(conn.clone()).answering_as(user_name()));
            (Box::new(http), api, origin, Some(conn))
        };
        let interval = source.interval();
        let mode = Mode::from_env_value(std::env::var("ROBROZ_THEME").ok().as_deref());
        let scale = std::env::var("ROBROZ_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        let app = Self {
            source: Arc::new(Mutex::new(source)),
            api,
            caps: if fake { Caps::FAKE } else { Caps::HOST },
            conn,
            convs: Conversations::default(),
            stream_up: false,
            narrow_open: false,
            bros: Vec::new(),
            drafts: HashMap::new(),
            sending: false,
            send_note: HashMap::new(),
            fake,
            rename: None,
            renaming: false,
            rename_note: None,
            origin,
            interval,
            groups: Vec::new(),
            feeds: HashMap::new(),
            preview_key: HashMap::new(),
            selected: None,
            page: Page::Group,
            ui: HashMap::new(),
            unfolded: HashSet::new(),
            routines: Routines::default(),
            memories: Book::default(),
            mode,
            width: env_size().map_or(1040.0, |s| s.0),
            loaded: false,
            in_flight: gallery.is_none(),
            has_more: gallery.is_none(),
            error: None,
            gallery,
            scale,
            touch: std::env::var("ROBROZ_TOUCH").is_ok_and(|v| v == "1"),
        };
        // `ROBROZ_SHOT=<file>` (gallery or fake): write the window's RGBA
        // after `ROBROZ_SHOT_DELAY` ms (default 2500), then exit. Used for
        // review screenshots.
        let shot = if std::env::var("ROBROZ_SHOT").is_ok() && fake {
            let ms = std::env::var("ROBROZ_SHOT_DELAY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2500);
            Task::perform(tokio_sleep(Duration::from_millis(ms)), |_| ())
                .then(|_| window::latest())
                .map(Message::Shoot)
        } else {
            Task::none()
        };
        if app.gallery.is_some() {
            return (app, shot);
        }
        let load = blocking(&app.source, |s| s.groups());
        (
            app,
            Task::batch([Task::perform(load, Message::Loaded), shot]),
        )
    }

    fn skin(&self) -> Skin {
        Skin::for_mode(self.mode).for_input(self.touch, self.width)
    }

    fn narrow(&self) -> bool {
        self.skin().narrow
    }

    fn load_unread(&self) -> Task<Message> {
        Task::perform(call(&self.api, |api| api.unread()), Message::UnreadLoaded)
    }

    fn load_bros(&self) -> Task<Message> {
        Task::perform(call(&self.api, |api| api.bros()), Message::BrosLoaded)
    }

    /// Open a chat: select it, drop its badge, tell the host it is read.
    fn open_chat(&mut self, group: String) -> Task<Message> {
        if self.selected.as_deref() != Some(group.as_str()) {
            // A new subscription starts for this chat; poll fast until it is up.
            self.stream_up = false;
        }
        self.selected = Some(group.clone());
        self.page = Page::Group;
        self.narrow_open = true;
        self.mark_read(group)
    }

    fn mark_read(&mut self, group: String) -> Task<Message> {
        self.convs.mark_read(&group);
        let id = group.clone();
        Task::perform(call(&self.api, move |api| api.mark_read(&id)), move |r| {
            Message::MarkedRead(group.clone(), r)
        })
    }

    /// Make a group known everywhere: list, conversation rows, source.
    fn learn_group(&mut self, tab: GroupTab, dm: bool) -> Task<Message> {
        self.convs.upsert(&tab.id, &tab.name, dm);
        match self.groups.iter_mut().find(|g| g.id == tab.id) {
            Some(g) => {
                g.name = tab.name;
                Task::none()
            }
            None => {
                self.groups.push(tab.clone());
                self.has_more = true;
                Task::perform(blocking(&self.source, move |s| s.add_group(tab)), |_| {
                    Message::Tick
                })
            }
        }
    }

    /// The bro whose DM is `group`.
    fn dm_bro(&self, group: &str) -> Option<&Bro> {
        self.bros
            .iter()
            .find(|b| b.dm_group_id.as_deref() == Some(group))
    }

    /// Click on a bro DM's name: edit it in place.
    fn start_rename(&mut self) -> Task<Message> {
        let Some(group) = self.selected.clone() else {
            return Task::none();
        };
        let Some(bro) = self.dm_bro(&group) else {
            return Task::none();
        };
        self.rename = Some((group, bro.name.clone()));
        self.rename_note = None;
        iced::widget::operation::focus(view::RENAME_INPUT)
    }

    fn save_rename(&mut self) -> Task<Message> {
        if self.renaming {
            return Task::none();
        }
        let Some((group, draft)) = self.rename.clone() else {
            return Task::none();
        };
        let Some(bro) = self.dm_bro(&group).cloned() else {
            self.rename = None;
            return Task::none();
        };
        if draft.trim() == bro.name {
            self.rename = None;
            return Task::none();
        }
        self.renaming = true;
        Task::perform(
            call(&self.api, move |api| api.rename_bro(&bro.id, &draft)),
            move |result| Message::Renamed {
                group: group.clone(),
                result,
            },
        )
    }

    /// Composer: one `/say`; the host stores it and picks the bro, if any.
    fn send(&mut self) -> Task<Message> {
        if self.sending {
            return Task::none();
        }
        let Some(group) = self.selected.clone() else {
            return Task::none();
        };
        let draft = self.drafts.get(&group).cloned().unwrap_or_default();
        let body = draft.trim().to_string();
        if body.is_empty() {
            return Task::none();
        }
        self.sending = true;
        let g = group.clone();
        let job = call(&self.api, move |api| api.say(&g, &body));
        Task::perform(job, move |result| Message::Sent {
            group: group.clone(),
            result,
        })
    }

    fn apply_event(&mut self, group: String, event: &Event) {
        // A confirm card answered anywhere (chat, page, another device, or a
        // forgotten pending memory) moves its entry on the 记忆 page too.
        if let Event::Confirm {
            memory_id: Some(memory),
            decision: Some(d),
            ..
        } = event
        {
            self.memories
                .answered(memory, *d == ApprovalDecision::Approved);
        }
        // Newest change time wins the list preview; a tie goes to the
        // outcome (see `model::preview_wins`), not to arrival order.
        let at = change_time(event);
        let rank = model::preview_rank(event);
        let cur = self.preview_key.get(&group);
        let wins = model::preview_wins((at, rank), cur.map(|(t, r)| (t.as_str(), *r)));
        let shown = wins.then(|| preview(event));
        if wins {
            self.preview_key
                .insert(group.clone(), (at.to_string(), rank));
        }
        self.convs.touch(&group, shown, Some(at));
        self.feeds.entry(group).or_default().apply(event);
        // A 409 note ("refreshing") goes once the answered card arrives.
        let answered = match event {
            Event::Choice {
                id,
                answered_at: Some(_),
                ..
            }
            | Event::Approval {
                id,
                decision: Some(_),
                ..
            }
            | Event::Confirm {
                id,
                decision: Some(_),
                ..
            } => Some(id),
            _ => None,
        };
        if let Some(ui) = answered.and_then(|id| self.ui.get_mut(id)) {
            if matches!(ui.note, Some(Note::AlreadyAnswered | Note::AlreadyDecided)) {
                ui.note = None;
            }
        }
    }

    fn poll(&mut self) -> Task<Message> {
        if self.in_flight || !self.loaded || !self.has_more {
            return Task::none();
        }
        self.in_flight = true;
        let poll = blocking(&self.source, |s| {
            s.poll().map(|events| Polled {
                events,
                cards: s.poll_cards(),
                has_more: s.has_more(),
            })
        });
        Task::perform(poll, Message::Polled)
    }

    /// The card with this id and the group it is in.
    fn find_card(&self, id: &str) -> Option<(String, Card)> {
        self.feeds
            .iter()
            .find_map(|(g, f)| f.card(id).map(|c| (g.clone(), c.clone())))
    }

    fn set_card(&mut self, group: &str, card: Card) {
        self.feeds
            .entry(group.to_string())
            .or_default()
            .apply_card(card);
    }

    fn load_routines(&self) -> Task<Message> {
        let groups: Vec<String> = self.groups.iter().map(|g| g.id.clone()).collect();
        let job = call(&self.api, move |api| {
            let mut all = Vec::new();
            for g in groups {
                all.extend(api.schedules(&g)?);
            }
            Ok(all)
        });
        Task::perform(job, Message::RoutinesLoaded)
    }

    fn load_memories(&mut self) -> Task<Message> {
        self.sync_memory_bros();
        Task::batch([
            Task::perform(
                call(&self.api, |api| api.memories()),
                Message::MemoriesLoaded,
            ),
            self.load_bros(),
        ])
    }

    /// Bro names for 「只给 …」 and the new-memory scope choices.
    fn sync_memory_bros(&mut self) {
        self.memories.bros = self
            .bros
            .iter()
            .map(|b| (b.id.clone(), b.name.clone()))
            .collect();
    }

    fn on_memory_action(&mut self, action: Action) -> Task<Message> {
        let book = &mut self.memories;
        match action {
            Action::MemoryAddOpen => book.open_add(),
            Action::MemoryAddClose => book.close_add(),
            Action::MemoryAddText { text } => {
                if let Some(new) = &mut book.adding {
                    new.text = text;
                }
            }
            Action::MemoryAddScope { scope } => {
                if let Some(new) = &mut book.adding {
                    new.scope = Some(scope);
                }
            }
            Action::MemoryAddKind { kind } => {
                if let Some(new) = &mut book.adding {
                    new.kind = kind;
                }
            }
            Action::MemoryAddSave => {
                if book.busy {
                    return Task::none();
                }
                let Some(new) = book.add_to_send() else {
                    return Task::none();
                };
                book.busy = true;
                let scope = new.scope.clone().unwrap_or_default();
                let job = call(&self.api, move |api| {
                    api.create_memory(&scope, new.kind, &new.text)
                });
                return Task::perform(job, |result| Message::MemorySaved { id: None, result });
            }
            Action::MemoryEdit { id } => book.start_edit(&id),
            Action::MemoryEditText { text } => book.edit_text(text),
            Action::MemoryEditCancel => book.cancel_edit(),
            Action::MemoryEditSave => {
                if book.busy {
                    return Task::none();
                }
                let Some((id, text)) = book.edit_to_save() else {
                    return Task::none();
                };
                book.busy = true;
                let mid = id.clone();
                let job = call(&self.api, move |api| api.update_memory(&mid, &text));
                return Task::perform(job, move |result| Message::MemorySaved {
                    id: Some(id.clone()),
                    result,
                });
            }
            Action::MemoryDecide { id, keep } => {
                let Some(card) = book.decide_to_send(&id) else {
                    return Task::none();
                };
                let job = call(&self.api, move |api| api.decide(&card, keep));
                return Task::perform(job, move |result| Message::MemoryDecided {
                    id: id.clone(),
                    keep,
                    result,
                });
            }
            Action::MemoryForgetAsk { id } => book.ask_forget(&id),
            Action::MemoryForgetCancel => book.cancel_forget(),
            Action::MemoryForgetConfirm => {
                if book.busy {
                    return Task::none();
                }
                let Some(id) = book.forget_to_send() else {
                    return Task::none();
                };
                book.busy = true;
                let rid = id.clone();
                let job = call(&self.api, move |api| api.forget_memory(&rid));
                return Task::perform(job, move |result| Message::MemoryForgot {
                    id: id.clone(),
                    result,
                });
            }
            _ => {}
        }
        Task::none()
    }

    fn on_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::MemoryAddOpen
            | Action::MemoryAddClose
            | Action::MemoryAddText { .. }
            | Action::MemoryAddScope { .. }
            | Action::MemoryAddKind { .. }
            | Action::MemoryAddSave
            | Action::MemoryEdit { .. }
            | Action::MemoryEditText { .. }
            | Action::MemoryEditSave
            | Action::MemoryEditCancel
            | Action::MemoryForgetAsk { .. }
            | Action::MemoryForgetConfirm
            | Action::MemoryForgetCancel
            | Action::MemoryDecide { .. } => self.on_memory_action(action),
            Action::ToggleInstruction { card } => {
                let ui = self.ui.entry(card).or_default();
                ui.more_open = !ui.more_open;
                Task::none()
            }
            Action::UnfoldConfirms { task } => {
                self.unfolded.insert(task);
                Task::none()
            }
            Action::Choose { card, option } => {
                let Some((_, Card::Choice(c))) = self.find_card(&card) else {
                    return Task::none();
                };
                let ui = self.ui.entry(card.clone()).or_default();
                let Some(ids) = ui.begin_choice(&c, Some(&option)) else {
                    return Task::none();
                };
                self.send_choice(card, ids)
            }
            Action::TogglePick { card, option } => {
                if let Some((_, Card::Choice(c))) = self.find_card(&card) {
                    self.ui.entry(card).or_default().toggle_pick(&c, &option);
                }
                Task::none()
            }
            Action::ConfirmChoice { card } => {
                let Some((_, Card::Choice(c))) = self.find_card(&card) else {
                    return Task::none();
                };
                let ui = self.ui.entry(card.clone()).or_default();
                let Some(ids) = ui.begin_choice(&c, None) else {
                    return Task::none();
                };
                self.send_choice(card, ids)
            }
            Action::Decide { card, approve } => {
                if !self.ui.entry(card.clone()).or_default().begin_action() {
                    return Task::none();
                }
                let id = card.clone();
                let job = call(&self.api, move |api| api.decide(&id, approve));
                Task::perform(job, move |result| Message::Decided {
                    card: card.clone(),
                    approve,
                    result,
                })
            }
            // Drafts are phase 2: the button is not drawn against a real host,
            // and a stray action is dropped rather than sent to a 404.
            Action::SendDraft { .. } | Action::EditDraftInDm { .. } if !self.caps.draft_actions => {
                Task::none()
            }
            Action::SendDraft { card } => {
                if !self.ui.entry(card.clone()).or_default().begin_action() {
                    return Task::none();
                }
                let id = card.clone();
                let job = call(&self.api, move |api| api.send_draft(&id));
                Task::perform(job, move |result| Message::Acted {
                    card: format!(">{card}"),
                    result,
                })
            }
            // Opening a bot's DM is not wired yet; this only logs.
            Action::EditDraftInDm { card } => {
                eprintln!("修改: open DM with draft {card} (not wired yet)");
                Task::none()
            }
            Action::TakeOver { card } => {
                eprintln!("去私聊接管: card {card} (not wired yet)");
                Task::none()
            }
            Action::ToggleBotName { card } => {
                if let Some(g) = &mut self.gallery {
                    g.toggle_name(&card);
                }
                let ui = self.ui.entry(card).or_default();
                ui.name_expanded = !ui.name_expanded;
                Task::none()
            }
            Action::OpenChat { group } => self.open_chat(group),
            Action::BackToList => {
                self.narrow_open = false;
                Task::none()
            }
            Action::ComposerEdit { text } => {
                if let Some(g) = self.selected.clone() {
                    self.send_note.remove(&g);
                    self.drafts.insert(g, text);
                }
                Task::none()
            }
            Action::ComposerSend => self.send(),
            Action::RenameStart => self.start_rename(),
            Action::RenameEdit { text } => {
                if let Some((_, draft)) = &mut self.rename {
                    *draft = text;
                }
                self.rename_note = None;
                Task::none()
            }
            Action::RenameCancel => {
                if !self.renaming {
                    self.rename = None;
                    self.rename_note = None;
                }
                Task::none()
            }
            Action::RenameSave => self.save_rename(),
            Action::NewBro => {
                Task::perform(call(&self.api, |api| api.create_bro()), Message::BroCreated)
            }
            Action::TogglePause { id } => {
                let Some(wanted) = self.routines.toggle(&id) else {
                    return Task::none();
                };
                let rid = id.clone();
                let job = call(&self.api, move |api| api.set_paused(&rid, wanted));
                Task::perform(job, move |result| Message::Paused {
                    id: id.clone(),
                    wanted,
                    result,
                })
            }
        }
    }

    fn send_choice(&self, card: String, ids: Vec<String>) -> Task<Message> {
        let (id, body) = (card.clone(), ids.clone());
        let job = call(&self.api, move |api| api.answer_choice(&id, &body));
        Task::perform(job, move |result| Message::Answered {
            card: card.clone(),
            sent: ids.clone(),
            result,
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenRoutines => {
                self.page = Page::Routines;
                self.narrow_open = true;
                return self.load_routines();
            }
            Message::OpenMemories => {
                self.page = Page::Memories;
                self.narrow_open = true;
                return self.load_memories();
            }
            Message::MemoriesLoaded(result) => match result {
                Ok(list) => {
                    self.memories.load(list);
                    self.memories.note = None;
                }
                Err(err) => self.memories.note = Some(err.reason()),
            },
            Message::MemorySaved { id, result } => match (id, result) {
                (None, Ok(m)) => self.memories.added(m),
                (Some(_), Ok(m)) => self.memories.saved(m),
                (None, Err(err)) => self.memories.add_failed(&err.reason()),
                (Some(id), Err(err)) => self.memories.entry_failed(&id, &err.reason()),
            },
            Message::MemoryDecided { id, keep, result } => match result {
                Ok(reply) => {
                    self.memories.decided(&id, keep);
                    // The chat card shows the answer at once; the stream
                    // brings the same snapshot again later.
                    if let Some(event @ Event::Confirm { .. }) = reply {
                        let card = robroz_ui::cards::confirm::ConfirmCard::from_event(&event);
                        let group = card.and_then(|c| self.find_card(&c.id)).map(|(g, _)| g);
                        if let Some(group) = group {
                            self.apply_event(group, &event);
                        }
                    }
                }
                Err(ApiError::Conflict(_)) => {
                    // Answered elsewhere: say so here and fetch the real state.
                    self.memories.decide_failed(&id, confirm::ALREADY_ANSWERED);
                    return Task::perform(
                        call(&self.api, |api| api.memories()),
                        Message::MemoriesLoaded,
                    );
                }
                Err(err) => self
                    .memories
                    .decide_failed(&id, &confirm::decide_failed(&err.reason())),
            },
            Message::MemoryForgot { id, result } => match result {
                Ok(()) | Err(ApiError::NotFound(_)) => self.memories.forgotten(&id),
                Err(err) => self.memories.entry_failed(&id, &err.reason()),
            },
            Message::Back => self.narrow_open = false,
            Message::UnreadTick => return self.load_unread(),
            Message::UnreadLoaded(result) => match result {
                Ok(list) => {
                    let mut unknown = false;
                    let mut read_open = None;
                    for u in &list {
                        unknown |= !self.groups.iter().any(|g| g.id == u.group_id);
                        self.convs.apply_unread(
                            &u.group_id,
                            u.unread,
                            u.last_activity_at.as_deref(),
                        );
                        // New things in the chat you are looking at are read.
                        let looking = self.page == Page::Group
                            && self.selected.as_deref() == Some(u.group_id.as_str())
                            && (!self.narrow() || self.narrow_open);
                        if looking && u.unread > 0 {
                            read_open = Some(u.group_id.clone());
                        }
                    }
                    let mut tasks = Vec::new();
                    if let Some(g) = read_open {
                        tasks.push(self.mark_read(g));
                    }
                    if unknown {
                        // A group or DM made elsewhere: fetch names, start polling it.
                        tasks.push(Task::perform(
                            blocking(&self.source, |s| s.groups()),
                            Message::GroupsReloaded,
                        ));
                        tasks.push(self.load_bros());
                    }
                    return Task::batch(tasks);
                }
                // The list keeps its last state; the footer shows why.
                Err(err) => self.error = Some(err.note().text()),
            },
            Message::MarkedRead(group, result) => {
                if let Ok(u) = result {
                    self.convs
                        .apply_unread(&group, u.unread, u.last_activity_at.as_deref());
                }
            }
            Message::BrosLoaded(result) => {
                if let Ok(bros) = result {
                    self.bros = bros.clone();
                    self.sync_memory_bros();
                    // `--fake` + `ROBROZ_RENAME=<draft>`: open the rename box
                    // on the selected DM (screenshots).
                    let mut tasks: Vec<_> = bros
                        .into_iter()
                        .filter_map(|b| b.dm_group_id.map(|id| GroupTab { id, name: b.name }))
                        .map(|tab| self.learn_group(tab, true))
                        .collect();
                    if let Some(draft) = std::env::var("ROBROZ_RENAME").ok().filter(|_| self.fake) {
                        if let Some(g) = self.selected.clone().filter(|g| self.dm_bro(g).is_some())
                        {
                            self.rename = Some((g, draft));
                            // `ROBROZ_RENAME_SAVE=1`: also press 保存.
                            if std::env::var("ROBROZ_RENAME_SAVE").is_ok_and(|v| v == "1") {
                                tasks.push(self.save_rename());
                            }
                        }
                    }
                    return Task::batch(tasks);
                }
            }
            Message::BroCreated(result) => match result {
                Ok(bro) => {
                    if !self.bros.iter().any(|b| b.id == bro.id) {
                        self.bros.push(bro.clone());
                    }
                    let Some(dm) = bro.dm_group_id else {
                        return Task::none();
                    };
                    let learn = self.learn_group(
                        GroupTab {
                            id: dm.clone(),
                            name: bro.name,
                        },
                        true,
                    );
                    let now = chrono::Utc::now().to_rfc3339();
                    self.convs.touch(&dm, None, Some(&now));
                    let open = self.open_chat(dm);
                    return Task::batch([learn, open]);
                }
                Err(err) => self.error = Some(format!("没能引入新 bro：{}", err.reason())),
            },
            Message::GroupsReloaded(result) => {
                if let Ok(groups) = result {
                    let tasks: Vec<_> = groups
                        .into_iter()
                        .map(|g| self.learn_group(g, false))
                        .collect();
                    return Task::batch(tasks);
                }
            }
            Message::Renamed { group, result } => {
                self.renaming = false;
                match result {
                    Ok(bro) => {
                        self.rename = None;
                        self.rename_note = None;
                        if let Some(b) = self.bros.iter_mut().find(|b| b.id == bro.id) {
                            b.name = bro.name.clone();
                        }
                        return self.learn_group(
                            GroupTab {
                                id: bro.dm_group_id.unwrap_or(group),
                                name: bro.name,
                            },
                            true,
                        );
                    }
                    Err(err) => {
                        self.rename_note = Some((group, format!("没改成：{}", err.reason())));
                    }
                }
            }
            Message::Sent { group, result } => {
                self.sending = false;
                match result {
                    Ok(Said { message: m, .. }) => {
                        self.drafts.remove(&group);
                        self.send_note.remove(&group);
                        let event = Event::Message {
                            id: m.id,
                            kind: m.kind,
                            body: m.body,
                            created_at: m.created_at,
                        };
                        self.apply_event(group.clone(), &event);
                        // Pick up the new status card without waiting a tick.
                        return self.poll();
                    }
                    Err(err) => {
                        self.send_note
                            .insert(group, format!("没发出去：{}", err.reason()));
                    }
                }
            }
            Message::Live((group, item)) => match item {
                StreamItem::Connected => {
                    if self.selected.as_deref() == Some(group.as_str()) {
                        self.stream_up = true;
                    }
                }
                StreamItem::Event(event) => self.apply_event(group, &event),
                StreamItem::Ping => {}
                // Polling takes over at full speed until it reconnects.
                StreamItem::Down(_) => self.stream_up = false,
            },
            Message::Tick => return self.poll(),
            Message::Loaded(result) => {
                self.in_flight = false;
                match result {
                    Ok(groups) => {
                        for g in &groups {
                            self.convs.upsert(&g.id, &g.name, false);
                        }
                        self.groups = groups;
                        self.loaded = true;
                        self.error = None;
                        let mut tasks = vec![self.poll(), self.load_unread(), self.load_bros()];
                        // `--fake` + `ROBROZ_OPEN=<group>`: open that chat
                        // (screenshots of the chat screen on a phone).
                        let wanted = std::env::var("ROBROZ_OPEN")
                            .ok()
                            .filter(|g| self.fake && self.groups.iter().any(|x| &x.id == g));
                        if let Some(g) = wanted {
                            tasks.push(self.open_chat(g));
                        } else if let Some(first) = self.groups.first().map(|g| g.id.clone()) {
                            // Desktop opens the first chat; narrow starts on the list.
                            if !self.narrow() {
                                tasks.push(self.open_chat(first));
                            }
                        }
                        return Task::batch(tasks);
                    }
                    Err(err) => self.error = Some(err.0),
                }
            }
            Message::Polled(result) => {
                self.in_flight = false;
                match result {
                    Ok(polled) => {
                        for ge in polled.events {
                            self.apply_event(ge.group_id, &ge.event);
                        }
                        for gc in polled.cards {
                            self.set_card(&gc.group_id, gc.card);
                        }
                        self.has_more = polled.has_more;
                        self.error = None;
                    }
                    // Keep what is on screen and try again next tick.
                    Err(err) => self.error = Some(err.0),
                }
            }
            // Opening the private chat is not wired yet; this only logs.
            Message::TakeOver(task_id) => eprintln!("去私聊接管: task {task_id} (not wired yet)"),
            Message::Ui(action) => return self.on_action(action),
            Message::Answered { card, sent, result } => {
                let found = self.find_card(&card);
                let ui = self.ui.entry(card.clone()).or_default();
                match result {
                    Ok(reply) => {
                        ui.choice_sent();
                        match (reply, found) {
                            (Some(event), Some((group, _))) => {
                                self.feeds.entry(group).or_default().apply(&event);
                            }
                            // No echo (fake API): show it answered now; the
                            // next event for this id replaces it anyway.
                            (None, Some((group, Card::Choice(mut c)))) => {
                                c.state = ChoiceState::Answered {
                                    selected: sent,
                                    at: Some(chrono::Utc::now().to_rfc3339()),
                                    by: Some(robroz_ui::cards::ME.into()),
                                };
                                self.set_card(&group, Card::Choice(c));
                            }
                            _ => {}
                        }
                    }
                    Err(err) => {
                        let conflict = matches!(err, ApiError::Conflict(_));
                        ui.choice_failed(err.note());
                        if conflict {
                            // Someone answered first: fetch the real state.
                            self.has_more = true;
                            return self.poll();
                        }
                    }
                }
            }
            Message::Decided {
                card,
                approve,
                result,
            } => {
                let found = self.find_card(&card);
                let ui = self.ui.entry(card.clone()).or_default();
                match result {
                    Ok(reply) => {
                        ui.action_done(None);
                        match (reply, found) {
                            // The host's answered card replaces the pending one.
                            (Some(event), Some((group, _))) => self.apply_event(group, &event),
                            // No echo (fake API): show the answer locally.
                            (None, Some((group, Card::Approval(mut c)))) => {
                                let at = now_unix();
                                c.state = if approve {
                                    ApprovalState::Approved {
                                        by: Some(robroz_ui::cards::ME.into()),
                                        at,
                                    }
                                } else {
                                    ApprovalState::Denied {
                                        by: Some(robroz_ui::cards::ME.into()),
                                        at,
                                    }
                                };
                                self.set_card(&group, Card::Approval(c));
                            }
                            (None, Some((group, Card::Confirm(mut c)))) => {
                                let (by, at) = (Some(robroz_ui::cards::ME.into()), now_unix());
                                c.state = if approve {
                                    ConfirmState::Kept { by, at }
                                } else {
                                    ConfirmState::Dropped { by, at }
                                };
                                if let Some(m) = &c.memory_id {
                                    self.memories.answered(m, approve);
                                }
                                self.set_card(&group, Card::Confirm(c));
                            }
                            _ => {}
                        }
                    }
                    Err(ApiError::Conflict(_)) => {
                        // Answered already (elsewhere): fetch the real state;
                        // the answered card clears this note when it lands.
                        ui.action_done(Some(Note::AlreadyDecided));
                        self.has_more = true;
                        return self.poll();
                    }
                    Err(err) => ui.action_done(Some(err.note())),
                }
            }
            Message::Acted { card, result } => {
                let (tag, id) = card.split_at(1);
                let id = id.to_string();
                let ui = self.ui.entry(id.clone()).or_default();
                match result {
                    Ok(()) => {
                        ui.action_done(None);
                        let (by, at) = (user_name().to_string(), now_unix());
                        match self.find_card(&id) {
                            Some((group, Card::Draft(mut c))) if tag == ">" => {
                                c.state = DraftState::Sent { by, at };
                                self.set_card(&group, Card::Draft(c));
                            }
                            _ => {}
                        }
                    }
                    Err(err) => ui.action_done(Some(err.note())),
                }
            }
            Message::RoutinesLoaded(result) => match result {
                Ok(entries) => {
                    self.routines
                        .load(entries.iter().map(RoutineRow::from_entry).collect());
                    self.error = None;
                }
                Err(err) => self.error = Some(err.note().text()),
            },
            Message::Paused { id, wanted, result } => match result {
                Ok(entry) => self.routines.confirm(RoutineRow::from_entry(&entry)),
                Err(_) => {
                    self.routines.revert(&id, wanted);
                    let ms = self.skin().motion.note_visible_ms;
                    return Task::perform(
                        tokio_sleep(Duration::from_millis(ms.into())),
                        move |_| Message::ClearPauseNote(id.clone()),
                    );
                }
            },
            Message::ClearPauseNote(id) => self.routines.clear_failed(&id),
            Message::ToggleTheme => self.mode = self.mode.toggled(),
            Message::Resized(size) => self.width = size.width,
            Message::TouchSeen => self.touch = true,
            Message::Shoot(Some(id)) => return window::screenshot(id).map(Message::Shot),
            Message::Shoot(None) => return iced::exit(),
            Message::Shot(shot) => {
                if let Ok(path) = std::env::var("ROBROZ_SHOT") {
                    // Raw RGBA; a script turns it into PNG.
                    let _ = std::fs::write(&path, &shot.rgba);
                    println!("shot {}x{} -> {path}", shot.size.width, shot.size.height);
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let resize = window::resize_events().map(|(_, size)| Message::Resized(size));
        let touch = iced::event::listen_with(|event, _, _| match event {
            iced::Event::Touch(_) => Some(Message::TouchSeen),
            _ => None,
        });
        let mut subs = vec![resize];
        if !self.touch {
            subs.push(touch);
        }
        if self.loaded && self.has_more {
            let every = stream::poll_interval(self.stream_up, self.interval);
            subs.push(iced::time::every(every).map(|_| Message::Tick));
        }
        if self.loaded && self.gallery.is_none() {
            subs.push(iced::time::every(UNREAD_EVERY).map(|_| Message::UnreadTick));
        }
        if self.rename.is_some() {
            // The rename box captures Esc, so listen to captured keys too.
            subs.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::Ui(Action::RenameCancel)),
                _ => None,
            }));
        }
        if let (Some(conn), Some(group), Page::Group) = (&self.conn, &self.selected, self.page) {
            let key = stream::Key {
                conn: conn.clone(),
                group_id: group.clone(),
            };
            subs.push(Subscription::run_with(key, stream::subscribe).map(Message::Live));
        }
        Subscription::batch(subs)
    }

    fn theme(&self) -> iced::Theme {
        match self.mode {
            Mode::Light => iced::Theme::Light,
            Mode::Dark => iced::Theme::Dark,
        }
    }

    fn scale_factor(&self) -> f32 {
        self.scale
    }

    fn view(&self) -> Element<'_, Message> {
        let t = self.skin();
        if let Some(g) = &self.gallery {
            return self.gallery_view(&t, g);
        }
        let main: Element<'_, Message> = if t.narrow {
            // Phone: one screen at a time.
            if self.narrow_open && self.page == Page::Group {
                self.page_view(&t)
            } else if self.narrow_open {
                let back = button(text("‹ 聊天").size(t.type_.row_name))
                    .height(Length::Fixed(
                        t.controls.touch_target.max(t.controls.button_h),
                    ))
                    .style(view::nav_button_style(t, false))
                    .on_press(Message::Back);
                column![
                    container(back).padding(Padding::from([4.0, 8.0])),
                    self.page_view(&t)
                ]
                .height(Fill)
                .into()
            } else {
                self.group_list(&t, Length::Fill)
            }
        } else {
            row![
                self.group_list(&t, Length::Fixed(t.controls.sidebar_w)),
                self.page_view(&t)
            ]
            .height(Fill)
            .into()
        };
        let live = match (&self.conn, self.stream_up) {
            (None, _) => "",
            (Some(_), true) => " · 实时",
            (Some(_), false) => " · 轮询",
        };
        let status = match &self.error {
            Some(err) => err.clone(),
            None => format!("{}{live}", self.origin),
        };
        let footer = row![
            text(status)
                .size(t.type_.foot)
                .color(color(t.palette.text3)),
            space::horizontal(),
            button(
                text(match self.mode {
                    Mode::Light => "深色",
                    Mode::Dark => "浅色",
                })
                .size(t.type_.foot),
            )
            .style(view::nav_button_style(t, false))
            .on_press(Message::ToggleTheme),
        ]
        .align_y(Alignment::Center);
        container(column![
            main,
            container(footer).padding(Padding::from([4.0, 10.0]))
        ])
        .style(view::page_style(t))
        .into()
    }

    fn group_list(&self, t: &Skin, width: Length) -> Element<'_, Message> {
        let selected = (self.page == Page::Group)
            .then_some(self.selected.as_deref())
            .flatten();
        let list_w = match width {
            Length::Fixed(w) => w,
            _ => self.width,
        } - 16.0;
        let list =
            view::conversation_list(t, &self.convs, selected, now_unix(), list_w).map(Message::Ui);
        let routines = button(text(robroz_ui::cards::routines::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::Routines))
            .on_press(Message::OpenRoutines);
        let memories = button(text(robroz_ui::cards::memories::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::Memories))
            .on_press(Message::OpenMemories);
        container(column![scrollable(list).height(Fill), routines, memories].spacing(4))
            .width(width)
            .height(Fill)
            .padding(8)
            .style(view::sidebar_style(*t))
            .into()
    }

    fn content_width(&self, t: &Skin) -> f32 {
        let chrome = if self.gallery.is_some() || t.narrow {
            0.0
        } else {
            t.controls.sidebar_w
        };
        (self.width - chrome - 2.0 * t.metrics.stream_padding).max(200.0)
    }

    fn page_view(&self, t: &Skin) -> Element<'_, Message> {
        if self.page == Page::Memories {
            return scrollable(
                container(view::memories_panel(t, &self.memories, now_unix()).map(Message::Ui))
                    .padding(t.metrics.stream_padding)
                    .width(Fill),
            )
            .height(Fill)
            .width(Fill)
            .into();
        }
        if self.page == Page::Routines {
            return scrollable(
                container(view::routines_panel(t, &self.routines, now_unix()).map(Message::Ui))
                    .padding(t.metrics.stream_padding)
                    .width(Fill),
            )
            .height(Fill)
            .width(Fill)
            .into();
        }
        let Some(id) = &self.selected else {
            return container(text("选一个聊天").color(color(t.palette.text2)))
                .center(Fill)
                .into();
        };
        let width = self.content_width(t);
        let empty = CardUi::default();
        let rows = self
            .feeds
            .get(id)
            .into_iter()
            .flat_map(Feed::rows)
            .collect::<Vec<_>>();
        // More than three pending confirm cards from one task fold away
        // behind 「还有 N 条待确认」.
        let plan = confirm::fold_plan(
            rows.iter().map(|r| match r {
                Row::Card(Card::Confirm(c)) => c.task_id.as_deref().map(|t| (t, c.is_pending())),
                _ => None,
            }),
            &self.unfolded,
        );
        let rows = rows
            .into_iter()
            .zip(plan)
            .filter_map(|(r, fold)| match fold {
                Fold::Show => Some(row_view(t, r, &self.ui, &empty, self.caps, width)),
                Fold::Hide => None,
                Fold::More { task, n } => Some(view::confirm_more(t, &task, n).map(Message::Ui)),
            });
        let feed = scrollable(
            column(rows)
                .spacing(t.metrics.stream_spacing)
                .padding(t.metrics.stream_padding)
                .width(Fill),
        )
        .anchor_bottom()
        .height(Fill)
        .width(Fill);
        let (name, dm) = self
            .convs
            .get(id)
            .map_or((id.as_str(), false), |c| (c.name.as_str(), c.dm));
        let hint = robroz_ui::conversations::composer_hint(name, dm);
        let rename = self.dm_bro(id).map(|_| view::HeaderRename {
            draft: self
                .rename
                .as_ref()
                .filter(|(g, _)| g == id)
                .map(|(_, d)| d.as_str()),
            saving: self.renaming,
            note: self
                .rename_note
                .as_ref()
                .filter(|(g, _)| g == id)
                .map(|(_, n)| n.as_str()),
        });
        let draft = self.drafts.get(id).map_or("", String::as_str);
        let note = self.send_note.get(id).map(String::as_str);
        column![
            view::chat_header(t, name, dm, t.narrow, rename).map(Message::Ui),
            view::rule(t),
            feed,
            view::rule(t),
            view::composer(t, draft, &hint, self.sending, note).map(Message::Ui),
        ]
        .height(Fill)
        .width(Fill)
        .into()
    }

    fn gallery_view<'a>(&'a self, t: &Skin, g: &'a Gallery) -> Element<'a, Message> {
        let width = self.content_width(t);
        let two_columns = width >= 900.0;
        let col_width = if two_columns {
            (width - t.metrics.stream_padding) / 2.0
        } else {
            width
        };
        let caption = |s: &'static str| -> Element<'a, Message> {
            text(s)
                .font(Font::MONOSPACE)
                .size(t.type_.tag)
                .color(color(t.palette.text3))
                .into()
        };
        let title = |s: &'static str| -> Element<'a, Message> {
            text(s)
                .size(t.type_.list_title)
                .font(view::bold())
                .color(color(t.palette.text))
                .into()
        };
        let section =
            |name: &'static str, samples: &'a [gallery::Sample]| -> Element<'a, Message> {
                let mut c = column![title(name)].spacing(6);
                for s in samples {
                    c = c.push(caption(s.caption)).push(
                        view::card(t, &s.card, &s.ui, Caps::FAKE, col_width).map(Message::Ui),
                    );
                }
                c.into()
            };
        // The frozen status card, to review its dark-mode outline.
        let running = robroz_card::palette(robroz_card::Tone::Ink);
        let status_sample: Element<'a, Message> = container(
            row![text("文档哥"), space::horizontal(), text("进行")].align_y(Alignment::Center),
        )
        .padding(view::status_card_padding(t))
        .width(Fill)
        .style(view::status_card_style(*t, running.fill, running.text))
        .into();
        let left = column![
            section("选项卡 choice", &g.choice),
            caption("status · running（冻结的状态卡，深色加 1px 描边）"),
            status_sample,
        ]
        .spacing(6)
        .width(Fill);
        let right = column![
            section("审批卡 approval", &g.approval),
            section("草稿卡 draft", &g.draft),
            title("例行任务列表 routines"),
            caption("routines · list"),
            view::routines_panel(t, &g.routines, g.now).map(Message::Ui),
            caption("routines · empty"),
            view::routines_panel(t, &g.empty, g.now).map(Message::Ui),
        ]
        .spacing(6)
        .width(Fill);
        let body: Element<'a, Message> = if two_columns {
            row![left, right].spacing(t.metrics.stream_padding).into()
        } else {
            column![left, right].spacing(18).into()
        };
        container(scrollable(
            container(body)
                .padding(t.metrics.stream_padding)
                .width(Fill),
        ))
        .width(Fill)
        .height(Fill)
        .style(view::page_style(*t))
        .into()
    }
}

async fn tokio_sleep(d: Duration) {
    tokio::time::sleep(d).await;
}

fn row_view<'a>(
    t: &Skin,
    r: &'a Row,
    ui: &HashMap<String, CardUi>,
    empty: &CardUi,
    caps: Caps,
    width: f32,
) -> Element<'a, Message> {
    match r {
        Row::Text { body } => text(body).color(color(t.palette.text)).into(),
        Row::Attachment { kind, name } => row![
            container(
                text(attachment_label(*kind))
                    .size(t.type_.tag)
                    .color(color(t.palette.text2))
            )
            .padding(Padding::from([2.0, t.metrics.tag_pad_h]))
            .style(view::card_style(*t)),
            text(name).color(color(t.palette.text)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
        Row::Status(card) => status_view(t, card, width),
        Row::Delivery(card) => delivery_view(t, card, width),
        Row::Card(card) => {
            let state = ui.get(card.id()).unwrap_or(empty);
            view::card(t, card, state, caps, width).map(Message::Ui)
        }
    }
}

/// Frozen status card: palette from `robroz-card`, outline from the skin.
fn status_view<'a>(t: &Skin, card: &'a StatusCard, width: f32) -> Element<'a, Message> {
    let line = progress_cell(t, card, width);
    let mut line = row![line, text(&card.status_line)]
        .spacing(12)
        .align_y(Alignment::Center);
    if let Some(action) = card.action {
        line = line.push(
            button(text(action).size(t.type_.button))
                .height(Length::Fixed(
                    t.controls.touch_target.max(t.controls.button_h),
                ))
                .style(button::secondary)
                .on_press(Message::TakeOver(card.task_id.clone())),
        );
    }
    container(line)
        .padding(view::status_card_padding(t))
        .width(Fill)
        .style(view::status_card_style(
            *t,
            card.palette.fill,
            card.palette.text,
        ))
        .into()
}

/// Left side of a status card: the bro name, and under it, left-aligned,
/// what the bro is doing (`progress`). One line, cut with "…", at a fixed
/// line height, so the card is one row taller with progress and exactly
/// its old height without it, whatever the text says.
fn progress_cell<'a>(t: &Skin, card: &'a StatusCard, width: f32) -> Element<'a, Message> {
    use iced::widget::text::{LineHeight, Wrapping};
    let name = text(&card.bot_name);
    let Some(progress) = &card.progress else {
        return container(name).width(Fill).into();
    };
    let size = t.type_.status;
    // Room left of the status word (and button), in ASCII width units.
    let unit = |s: &str| {
        s.chars()
            .map(|c| if c.is_ascii() { 1.0 } else { 2.0 })
            .sum::<f32>()
    };
    let right = unit(&card.status_line) * t.type_.body * t.type_.unit_width
        + card.action.map_or(0.0, |a| {
            unit(a) * t.type_.button * t.type_.unit_width + 2.0 * t.controls.card_pad_h + 12.0
        })
        + 12.0;
    let room = width - 2.0 * (t.metrics.status_pad_h + t.stroke.card) - right;
    let cols = (room / (size * t.type_.unit_width)).max(8.0) as usize;
    let fitted = robroz_ui::cards::status::clamp_lines(progress, cols, 1);
    let rgb = robroz_ui::cards::status::progress_rgb(card.palette.fill, card.palette.text);
    let line = text(fitted)
        .size(size)
        .line_height(LineHeight::Absolute(progress_line_height(size).into()))
        .wrapping(Wrapping::None)
        .color(color(rgb));
    container(column![name, container(line).clip(true).width(Fill)].spacing(2))
        .width(Fill)
        .into()
}

/// Fixed height of the progress line: the card never jumps as the text changes.
fn progress_line_height(size: f32) -> f32 {
    (size * 1.5).round()
}

fn delivery_view<'a>(t: &Skin, card: &'a DeliveryCard, width: f32) -> Element<'a, Message> {
    // Did not get done: the same grey as 「没写出结果」, no path, no buttons.
    if let Some(failure) = &card.failure {
        let gray = robroz_card::palette(robroz_card::Tone::Gray);
        return container(text(model::failure_line(failure)))
            .padding(view::status_card_padding(t))
            .width(Fill)
            .style(view::status_card_style(*t, gray.fill, gray.text))
            .into();
    }
    let path = view::display_path(
        t,
        &card.path,
        width - 2.0 * (t.controls.card_pad_h + t.stroke.card),
    );
    let mut head = row![
        text(&card.summary).color(color(t.palette.text)),
        space::horizontal()
    ]
    .align_y(Alignment::Center);
    if let Some(badge) = card.badge {
        head = head.push(
            container(text(badge).size(t.type_.tag).color(color(t.palette.text2)))
                .padding(Padding::from([2.0, t.metrics.tag_pad_h]))
                .style(view::card_style(*t)),
        );
    }
    container(
        column![
            head,
            // Display only: plain text, not a link or button.
            text(path)
                .wrapping(iced::widget::text::Wrapping::None)
                .font(Font::MONOSPACE)
                .size(t.type_.mono)
                .color(color(t.palette.text2)),
        ]
        .spacing(6),
    )
    .padding(view::card_padding(t))
    .width(Fill)
    .style(view::card_style(*t))
    .into()
}

#[cfg(test)]
mod tests {
    use super::pick_name;

    #[test]
    fn user_name_env_then_hostname_then_robroz() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(pick_name(s(" Panda 的 Mac "), s("mbp")), "Panda 的 Mac");
        assert_eq!(pick_name(s("  "), s("mbp.local\n")), "mbp.local");
        assert_eq!(pick_name(None, s("box")), "box");
        assert_eq!(pick_name(None, None), "robroz");
        assert_eq!(pick_name(s("  "), s("")), "robroz");
        let long = "字".repeat(60);
        assert_eq!(pick_name(Some(long), None).chars().count(), 40);
    }
}
