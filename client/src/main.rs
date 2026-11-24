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
mod event_decode;
mod gallery;
mod host_state;
mod model;
mod prefs;
mod source;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{window, Alignment, Element, Fill, Font, Length, Padding, Size, Subscription, Task};
use robroz_contract::{
    ApprovalDecision, Bro, ConfirmSubject, Event, McpServer, McpTestResult, McpTool, Memory, Said,
    ScheduleEntry, Skill, Unread,
};
use robroz_ui::cards::confirm::{self, ConfirmState, Fold};
use robroz_ui::cards::interact::{CardUi, Note};
use robroz_ui::cards::mcp::{self, Book as McpBook};
use robroz_ui::cards::memories::Book;

use robroz_ui::cards::model::{self as model_page, Current as ModelCurrent, Page as ModelPage};
use robroz_ui::cards::review_rules::{self, Page as ReviewRulesPage, Rule as ReviewRule};

use robroz_ui::cards::routines::{RoutineRow, Routines};
use robroz_ui::cards::search::{self as search_page, Current as SearchCurrent, Page as SearchPage};
use robroz_ui::cards::skills::Book as SkillBook;
use robroz_ui::cards::{ApprovalCard, ApprovalExtras};
use robroz_ui::cards::{ApprovalState, Card, ChoiceState, DraftState};
use robroz_ui::conversations::Conversations;
use robroz_ui::i18n::{self, Key as Text, Locale};
use robroz_ui::view::{self, color, Action, Caps};
use robroz_ui::{Mode, Skin};

use api::{ApiError, CardApi, FakeApi, HttpApi};
use conn::Conn;
use gallery::Gallery;
use host_state::HostState;
use model::{attachment_label, change_time, DeliveryCard, Feed, Row, StatusCard};
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
    Skills,
    Mcp,
    Search,

    Model,
    ReviewRules,
}

#[derive(Debug, Clone)]
enum Message {
    OpenRoutines,
    OpenMemories,
    OpenSkills,
    /// 联网搜索 page (sidebar, or 「去设置」 on a delivery).
    OpenSearch,
    SearchLoaded(Result<SearchCurrent, ApiError>),
    SearchSaved(Result<SearchCurrent, ApiError>),
    SearchTested(Result<(bool, Option<String>), ApiError>),

    OpenReviewRules,
    ReviewRulesLoaded(Result<Vec<ReviewRule>, ApiError>),
    ReviewRuleProposed(Result<Event, ApiError>),
    /// 模型 page (sidebar, or 「去设置」 on a delivery).
    OpenModel,
    ModelLoaded(Result<ModelCurrent, ApiError>),
    ModelSaved(Result<ModelCurrent, ApiError>),
    ModelTested(Result<(bool, Option<String>), ApiError>),

    MemoriesLoaded(Result<Vec<Memory>, ApiError>),
    SkillsLoaded(Result<Vec<Skill>, ApiError>),
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
        result: Result<Option<(Event, Option<ApprovalExtras>)>, ApiError>,
    },
    /// PATCH (edit) or POST (new) answered on the 技能 page.
    SkillSaved {
        /// `None` for 加一项, the slug for 改.
        slug: Option<String>,
        result: Result<Skill, ApiError>,
    },
    SkillForgot {
        slug: String,
        result: Result<(), ApiError>,
    },
    /// 保存 / 不用 from the 技能 page, answered on the entry's confirm card.
    SkillDecided {
        slug: String,
        keep: bool,
        result: Result<Option<(Event, Option<ApprovalExtras>)>, ApiError>,
    },
    SkillToggled {
        slug: String,
        group: String,
        on: bool,
        result: Result<(), ApiError>,
    },
    GroupSkillsLoaded {
        group: String,
        result: Result<Vec<Skill>, ApiError>,
    },
    OpenMcp,
    McpLoaded(Result<Vec<McpServer>, ApiError>),
    McpSaved {
        result: Result<McpServer, ApiError>,
    },
    McpDeleted {
        id: String,
        result: Result<(), ApiError>,
    },
    McpToolsLoaded {
        id: String,
        result: Result<Vec<McpTool>, ApiError>,
    },
    McpTested {
        id: String,
        result: Result<McpTestResult, ApiError>,
    },
    McpEnabled {
        id: String,
        result: Result<McpServer, ApiError>,
    },
    McpGroupToggled {
        id: String,
        group: String,
        on: bool,
        result: Result<(), ApiError>,
    },
    GroupMcpLoaded {
        group: String,
        result: Result<Vec<McpServer>, ApiError>,
    },
    McpCommandApproved {
        id: String,
        result: Result<McpServer, ApiError>,
    },
    Tick,
    /// Ask the host for the groups again after the first listing failed.
    RetryLoad,
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
        result: Result<Option<(Event, Option<ApprovalExtras>)>, ApiError>,
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
    /// Language row on the Settings (模型) page: English / 中文.
    SetLocale(Locale),
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
    /// The event behind each chat's list preview (see `model::Previews`).
    previews: model::Previews,
    selected: Option<String>,
    page: Page,
    ui: HashMap<String, CardUi>,
    /// Tasks whose folded confirm cards were opened (「还有 N 条待确认」).
    unfolded: HashSet<String>,
    routines: Routines,
    /// 记忆 page (robroz#62).
    memories: Book,
    /// 技能 page (robroz#71).
    skills: SkillBook,
    /// MCP page (robroz#81).
    mcp: McpBook,
    /// 联网搜索 page (robroz#76).
    search: SearchPage,

    /// 放行规则 page.
    review_rules: ReviewRulesPage,
    /// 模型 page (robroz#102).
    model_settings: ModelPage,

    mode: Mode,
    width: f32,
    loaded: bool,
    /// The latest attempt at the first group listing failed.
    load_failed: bool,
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
        // Before anything builds text: fake and gallery runs ignore the
        // saved choice so their screenshots are reproducible.
        i18n::set_locale(prefs::initial_locale(!fake));
        type Parts = (Box<dyn EventSource>, Arc<dyn CardApi>, String, Option<Conn>);
        let (source, api, origin, conn): Parts = if fake {
            let (unread, bros) = fake_chats();
            (
                Box::new(FakeSource::new()),
                Arc::new(FakeApi::new(fake_schedules()).with_chats(unread, bros)),
                Text::ClientFakeOrigin.into(),
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
            previews: model::Previews::default(),
            selected: None,
            page: Page::Group,
            ui: HashMap::new(),
            unfolded: HashSet::new(),
            routines: Routines::default(),
            memories: Book::default(),
            skills: SkillBook::default(),
            mcp: McpBook::default(),
            search: SearchPage::default(),

            review_rules: ReviewRulesPage::default(),
            model_settings: ModelPage::default(),

            mode,
            width: env_size().map_or(1040.0, |s| s.0),
            loaded: false,
            load_failed: false,
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

    /// Whether the host has answered the first group listing.
    fn host_state(&self) -> HostState {
        HostState::of(self.loaded, self.load_failed)
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
        self.apply_event_with(group, event, None);
    }

    fn apply_event_with(&mut self, group: String, event: &Event, extras: Option<&ApprovalExtras>) {
        // Settings-page rule cards never belong in a chat, stream or unread.
        if event_decode::is_settings_approval(event) {
            self.sync_review_pending(event, extras);
            return;
        }
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
        if let Event::Confirm {
            subject: ConfirmSubject::Skill,
            slug: Some(slug),
            decision: Some(d),
            ..
        } = event
        {
            self.skills.answered(slug, *d == ApprovalDecision::Approved);
        }
        // Newest change time wins the list preview; a tie goes to the
        // outcome (see `model::preview_wins`), not to arrival order.
        let at = change_time(event);
        let shown = self.previews.offer(&group, event);
        self.convs.touch(&group, shown, Some(at));
        self.feeds
            .entry(group)
            .or_default()
            .apply_with(event, extras);
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
        self.sync_review_pending(event, extras);
    }

    /// Keep the settings-page confirm card in sync if this event is for it.
    fn sync_review_pending(&mut self, event: &Event, extras: Option<&ApprovalExtras>) {
        let Some(card) = ApprovalCard::from_event_with(event, extras) else {
            return;
        };
        let Some(pending) = self.review_rules.pending.as_ref() else {
            return;
        };
        if pending.id != card.id {
            return;
        }
        match card.state {
            ApprovalState::Expired { at } => self.review_rules.expire_pending(at),
            ApprovalState::Approved { .. } | ApprovalState::Denied { .. } => {
                self.review_rules.finish_pending();
            }
            ApprovalState::Pending | ApprovalState::NeedsOwner => {
                if let Some(pending) = self.review_rules.pending.as_mut() {
                    pending.state = card.state;
                }
            }
        }
    }

    fn refresh_review_rules(&self) -> Task<Message> {
        Task::perform(
            call(&self.api, |api| api.review_rules()),
            Message::ReviewRulesLoaded,
        )
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

    fn load_skills(&mut self) -> Task<Message> {
        self.sync_skill_context();
        let groups = self.skills.begin_enable_loads();
        let mut tasks = vec![
            Task::perform(call(&self.api, |api| api.skills()), Message::SkillsLoaded),
            self.load_bros(),
        ];
        tasks.extend(groups.into_iter().map(|g| self.fetch_group_skills(g)));
        Task::batch(tasks)
    }

    fn fetch_group_skills(&self, group: String) -> Task<Message> {
        let key = group.clone();
        let job = call(&self.api, move |api| api.group_skills(&key));
        Task::perform(job, move |result| Message::GroupSkillsLoaded {
            group,
            result,
        })
    }

    /// Fetch enable lists for chats that have not been asked yet.
    fn ensure_group_skill_fetches(&mut self) -> Task<Message> {
        self.sync_skill_context();
        let groups = self.skills.take_groups_to_load();
        Task::batch(groups.into_iter().map(|g| self.fetch_group_skills(g)))
    }

    /// Bro names and chats for the 技能 page (scope labels and toggles).
    fn sync_skill_context(&mut self) {
        self.skills.bros = self
            .bros
            .iter()
            .map(|b| (b.id.clone(), b.name.clone()))
            .collect();
        self.skills.chats = self
            .convs
            .rows()
            .iter()
            .map(|c| (c.group_id.clone(), c.name.clone()))
            .collect();
    }

    fn load_mcp(&mut self) -> Task<Message> {
        self.sync_mcp_context();
        let groups = self.mcp.begin_enable_loads();
        let mut tasks = vec![Task::perform(
            call(&self.api, |api| api.mcp_servers()),
            Message::McpLoaded,
        )];
        tasks.extend(groups.into_iter().map(|g| self.fetch_group_mcp(g)));
        Task::batch(tasks)
    }

    fn fetch_group_mcp(&self, group: String) -> Task<Message> {
        let key = group.clone();
        let job = call(&self.api, move |api| api.group_mcp(&key));
        Task::perform(job, move |result| Message::GroupMcpLoaded { group, result })
    }

    fn ensure_group_mcp_fetches(&mut self) -> Task<Message> {
        self.sync_mcp_context();
        let groups = self.mcp.take_groups_to_load();
        Task::batch(groups.into_iter().map(|g| self.fetch_group_mcp(g)))
    }

    fn sync_mcp_context(&mut self) {
        self.mcp.chats = self
            .convs
            .rows()
            .iter()
            .map(|c| (c.group_id.clone(), c.name.clone()))
            .collect();
    }

    fn fetch_mcp_tools(&self, id: String) -> Task<Message> {
        let key = id.clone();
        let job = call(&self.api, move |api| api.mcp_tools(&key));
        Task::perform(job, move |result| Message::McpToolsLoaded { id, result })
    }

    fn test_mcp(&self, id: String) -> Task<Message> {
        let key = id.clone();
        let job = call(&self.api, move |api| api.test_mcp_server(&key));
        Task::perform(job, move |result| Message::McpTested { id, result })
    }

    fn on_mcp_action(&mut self, action: Action) -> Task<Message> {
        let book = &mut self.mcp;
        match action {
            Action::McpAddOpen => book.open_add(),
            Action::McpFormClose => book.close_form(),
            Action::McpFormName { text } => {
                if let Some(f) = book.form_mut() {
                    f.name = text;
                }
            }
            Action::McpFormTransport { http } => {
                if let Some(f) = book.form_mut() {
                    f.transport = if http {
                        mcp::Transport::Http
                    } else {
                        mcp::Transport::Stdio
                    };
                }
            }
            Action::McpFormCommand { text } => {
                if let Some(f) = book.form_mut() {
                    f.command = text;
                }
            }
            Action::McpFormArgs { text } => {
                if let Some(f) = book.form_mut() {
                    f.args = text;
                }
            }
            Action::McpFormUrl { text } => {
                if let Some(f) = book.form_mut() {
                    f.url = text;
                }
            }
            Action::McpSecretName { index, text } => {
                if let Some(f) = book.form_mut() {
                    if let Some(s) = f.secrets.get_mut(index) {
                        s.name = text;
                    }
                }
            }
            Action::McpSecretPlacement { index, header } => {
                if let Some(f) = book.form_mut() {
                    if let Some(s) = f.secrets.get_mut(index) {
                        s.placement = if header {
                            robroz_contract::McpSecretIn::Header
                        } else {
                            robroz_contract::McpSecretIn::Env
                        };
                    }
                }
            }
            Action::McpSecretKey { index, text } => {
                if let Some(f) = book.form_mut() {
                    if let Some(s) = f.secrets.get_mut(index) {
                        s.key.draft = text;
                    }
                }
            }
            Action::McpSecretClear { index } => {
                if let Some(f) = book.form_mut() {
                    if let Some(s) = f.secrets.get_mut(index) {
                        s.key.clear();
                    }
                }
            }
            Action::McpSecretCancel { index } => {
                if let Some(f) = book.form_mut() {
                    if let Some(s) = f.secrets.get_mut(index) {
                        s.key.cancel();
                    }
                }
            }
            Action::McpSecretAdd => book.add_secret(),
            Action::McpSecretRemove { index } => {
                if let Some(f) = book.form_mut() {
                    if index < f.secrets.len() {
                        f.secrets.remove(index);
                    }
                }
            }
            Action::McpSave => {
                let Some(save) = book.save_to_send() else {
                    return Task::none();
                };
                let job = call(&self.api, move |api| {
                    if save.id.is_some() {
                        api.update_mcp_server(&save)
                    } else {
                        api.create_mcp_server(&save)
                    }
                });
                return Task::perform(job, move |result| Message::McpSaved { result });
            }
            Action::McpEdit { id } => book.start_edit(&id),
            Action::McpOpen { id } => {
                if book.open_server(&id) {
                    return self.fetch_mcp_tools(id);
                }
            }
            Action::McpDeleteAsk { id } => book.ask_delete(&id),
            Action::McpDeleteCancel => book.cancel_delete(),
            Action::McpDeleteConfirm => {
                let Some(id) = book.delete_to_send() else {
                    return Task::none();
                };
                let key = id.clone();
                let job = call(&self.api, move |api| api.delete_mcp_server(&key));
                return Task::perform(job, move |result| Message::McpDeleted {
                    id: id.clone(),
                    result,
                });
            }
            Action::McpToggleEnabled { id } => {
                let Some(on) = book.toggle_enabled_to_send(&id) else {
                    return Task::none();
                };
                let key = id.clone();
                let job = call(&self.api, move |api| api.set_mcp_enabled(&key, on));
                return Task::perform(job, move |result| Message::McpEnabled {
                    id: id.clone(),
                    result,
                });
            }
            Action::McpToggleGroup { id, group } => {
                let Some(on) = book.toggle_group_to_send(&id, &group) else {
                    return Task::none();
                };
                let (sid, gid) = (id.clone(), group.clone());
                let job = call(&self.api, move |api| api.set_group_mcp(&gid, &sid, on));
                return Task::perform(job, move |result| Message::McpGroupToggled {
                    id: id.clone(),
                    group: group.clone(),
                    on,
                    result,
                });
            }
            Action::McpApproveCommand { id } => {
                let Some(key) = book.approve_command_to_send(&id) else {
                    return Task::none();
                };
                let job = call(&self.api, move |api| api.approve_mcp_command(&key));
                return Task::perform(job, move |result| Message::McpCommandApproved {
                    id: id.clone(),
                    result,
                });
            }
            Action::McpCancelCommand { id } => book.cancel_command_prompt(&id),
            _ => {}
        }
        Task::none()
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

    fn on_skill_action(&mut self, action: Action) -> Task<Message> {
        let book = &mut self.skills;
        match action {
            Action::SkillAddOpen => book.open_add(),
            Action::SkillAddClose => book.close_add(),
            Action::SkillAddSlug { text } => {
                if let Some(new) = &mut book.adding {
                    new.slug = text;
                }
            }
            Action::SkillAddTitle { text } => {
                if let Some(new) = &mut book.adding {
                    new.title = text;
                }
            }
            Action::SkillAddInstruction { text } => {
                if let Some(new) = &mut book.adding {
                    new.instruction = text;
                }
            }
            Action::SkillAddScope { scope } => {
                if let Some(new) = &mut book.adding {
                    new.scope = Some(scope);
                }
            }
            Action::SkillAddSave => {
                if book.busy {
                    return Task::none();
                }
                let Some(new) = book.add_to_send() else {
                    return Task::none();
                };
                book.busy = true;
                let scope = new.scope.clone().unwrap_or_default();
                let job = call(&self.api, move |api| {
                    api.create_skill(&scope, &new.slug, &new.title, &new.instruction)
                });
                return Task::perform(job, |result| Message::SkillSaved { slug: None, result });
            }
            Action::SkillEdit { slug } => book.start_edit(&slug),
            Action::SkillEditTitle { text } => book.edit_title(text),
            Action::SkillEditInstruction { text } => book.edit_instruction(text),
            Action::SkillEditCancel => book.cancel_edit(),
            Action::SkillEditSave => {
                if book.busy {
                    return Task::none();
                }
                let Some((slug, title, instruction)) = book.edit_to_save() else {
                    return Task::none();
                };
                book.busy = true;
                let key = slug.clone();
                let job = call(&self.api, move |api| {
                    api.update_skill(&key, &title, &instruction)
                });
                return Task::perform(job, move |result| Message::SkillSaved {
                    slug: Some(slug.clone()),
                    result,
                });
            }
            Action::SkillDecide { slug, keep } => {
                let Some(card) = book.decide_to_send(&slug) else {
                    return Task::none();
                };
                let job = call(&self.api, move |api| api.decide(&card, keep));
                return Task::perform(job, move |result| Message::SkillDecided {
                    slug: slug.clone(),
                    keep,
                    result,
                });
            }
            Action::SkillDeleteAsk { slug } => book.ask_delete(&slug),
            Action::SkillDeleteCancel => book.cancel_delete(),
            Action::SkillDeleteConfirm => {
                if book.busy {
                    return Task::none();
                }
                let Some(slug) = book.delete_to_send() else {
                    return Task::none();
                };
                book.busy = true;
                let key = slug.clone();
                let job = call(&self.api, move |api| api.forget_skill(&key));
                return Task::perform(job, move |result| Message::SkillForgot {
                    slug: slug.clone(),
                    result,
                });
            }
            Action::SkillToggleInstruction { slug } => book.toggle_instruction(&slug),
            Action::SkillToggleGroup { slug, group } => {
                let Some(on) = book.toggle_to_send(&slug, &group) else {
                    return Task::none();
                };
                let (s, g) = (slug.clone(), group.clone());
                let job = call(&self.api, move |api| api.set_skill_enabled(&g, &s, on));
                return Task::perform(job, move |result| Message::SkillToggled {
                    slug: slug.clone(),
                    group: group.clone(),
                    on,
                    result,
                });
            }
            _ => {}
        }
        Task::none()
    }

    fn on_search_action(&mut self, action: Action) -> Task<Message> {
        let page = &mut self.search;
        match action {
            Action::SearchPick { provider } => page.pick(provider),
            Action::SearchUrl { text } => page.url = text,
            Action::SearchKey { text } => page.key.draft = text,
            Action::SearchClearKey => page.clear_key(),
            Action::SearchCancelKey => page.cancel_key(),
            Action::SearchSave => {
                if page.busy {
                    return Task::none();
                }
                let Some(put) = page.to_send() else {
                    return Task::none();
                };
                page.busy = true;
                page.test = None;
                return Task::perform(
                    call(&self.api, move |api| api.put_search(&put)),
                    Message::SearchSaved,
                );
            }
            _ => {}
        }
        Task::none()
    }

    fn on_review_action(&mut self, action: Action) -> Task<Message> {
        let page = &mut self.review_rules;
        match action {
            Action::ReviewDraft { text } => page.set_draft(text),
            Action::ReviewAdd => {
                if page.locked() {
                    return Task::none();
                }
                let Some(text) = page.to_add() else {
                    return Task::none();
                };
                page.begin_add(text.clone());
                return Task::perform(
                    call(&self.api, move |api| {
                        let id = api.propose_review_rule(&text)?;
                        api.approval(&id)
                    }),
                    Message::ReviewRuleProposed,
                );
            }
            Action::ReviewDelete { id } => {
                if page.locked() {
                    return Task::none();
                }
                let Some(text) = page
                    .rules
                    .as_ref()
                    .and_then(|rules| rules.iter().find(|r| r.id == id))
                    .map(|r| r.text.clone())
                else {
                    return Task::none();
                };
                page.begin_delete(id.clone(), text);
                return Task::perform(
                    call(&self.api, move |api| {
                        let id = api.propose_delete_review_rule(&id)?;
                        api.approval(&id)
                    }),
                    Message::ReviewRuleProposed,
                );
            }
            _ => {}
        }
        Task::none()
    }

    fn on_model_action(&mut self, action: Action) -> Task<Message> {
        let page = &mut self.model_settings;
        match action {
            Action::ModelPick { preset } => {
                if page.busy {
                    return Task::none();
                }
                page.pick(preset);
            }
            Action::ModelUrl { text } => page.set_url(text),
            Action::ModelName { text } => page.set_model(text),
            Action::ModelKey { text } => page.key.draft = text,
            Action::ModelClearKey => {
                if !page.busy {
                    page.clear_key();
                }
            }
            Action::ModelCancelKey => page.cancel_key(),
            Action::ModelSave => {
                if page.busy {
                    return Task::none();
                }
                let Some(put) = page.to_send() else {
                    return Task::none();
                };
                page.busy = true;
                page.test = None;
                return Task::perform(
                    call(&self.api, move |api| api.put_model(&put)),
                    Message::ModelSaved,
                );
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
            Action::SkillAddOpen
            | Action::SkillAddClose
            | Action::SkillAddSlug { .. }
            | Action::SkillAddTitle { .. }
            | Action::SkillAddInstruction { .. }
            | Action::SkillAddScope { .. }
            | Action::SkillAddSave
            | Action::SkillEdit { .. }
            | Action::SkillEditTitle { .. }
            | Action::SkillEditInstruction { .. }
            | Action::SkillEditSave
            | Action::SkillEditCancel
            | Action::SkillDeleteAsk { .. }
            | Action::SkillDeleteConfirm
            | Action::SkillDeleteCancel
            | Action::SkillDecide { .. }
            | Action::SkillToggleInstruction { .. }
            | Action::SkillToggleGroup { .. } => self.on_skill_action(action),
            Action::McpAddOpen
            | Action::McpFormClose
            | Action::McpFormName { .. }
            | Action::McpFormTransport { .. }
            | Action::McpFormCommand { .. }
            | Action::McpFormArgs { .. }
            | Action::McpFormUrl { .. }
            | Action::McpSecretName { .. }
            | Action::McpSecretPlacement { .. }
            | Action::McpSecretKey { .. }
            | Action::McpSecretClear { .. }
            | Action::McpSecretCancel { .. }
            | Action::McpSecretAdd
            | Action::McpSecretRemove { .. }
            | Action::McpSave
            | Action::McpEdit { .. }
            | Action::McpOpen { .. }
            | Action::McpDeleteAsk { .. }
            | Action::McpDeleteConfirm
            | Action::McpDeleteCancel
            | Action::McpToggleEnabled { .. }
            | Action::McpToggleGroup { .. }
            | Action::McpApproveCommand { .. }
            | Action::McpCancelCommand { .. } => self.on_mcp_action(action),
            Action::SearchPick { .. }
            | Action::SearchUrl { .. }
            | Action::SearchKey { .. }
            | Action::SearchClearKey
            | Action::SearchCancelKey
            | Action::SearchSave => self.on_search_action(action),

            Action::ReviewDraft { .. } | Action::ReviewAdd | Action::ReviewDelete { .. } => {
                self.on_review_action(action)
            }
            Action::ModelPick { .. }
            | Action::ModelUrl { .. }
            | Action::ModelName { .. }
            | Action::ModelKey { .. }
            | Action::ModelClearKey
            | Action::ModelCancelKey
            | Action::ModelSave => self.on_model_action(action),

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
                if self
                    .review_rules
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.id == card)
                {
                    self.review_rules.busy = true;
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
            Message::OpenSearch => {
                self.page = Page::Search;
                self.narrow_open = true;
                self.search.test = None;
                self.search.note = None;
                return Task::perform(
                    call(&self.api, |api| api.search_settings()),
                    Message::SearchLoaded,
                );
            }
            Message::SearchLoaded(result) => match result {
                Ok(cur) => self.search.load(cur),
                Err(err) => self.search.note = Some(err.reason()),
            },
            Message::SearchSaved(result) => match result {
                Ok(cur) => {
                    self.search.saved(cur);
                    return Task::perform(
                        call(&self.api, |api| api.test_search()),
                        Message::SearchTested,
                    );
                }
                Err(err) => self.search.save_failed(&err.reason()),
            },
            Message::SearchTested(result) => match result {
                Ok((ok, error)) => self.search.tested(ok, error),
                Err(err) => self.search.tested(false, Some(err.reason())),
            },
            Message::OpenModel => {
                self.page = Page::Model;
                self.narrow_open = true;
                self.model_settings.test = None;
                self.model_settings.note = None;
                return Task::perform(
                    call(&self.api, |api| api.model_settings()),
                    Message::ModelLoaded,
                );
            }
            Message::ModelLoaded(result) => match result {
                Ok(cur) => self.model_settings.load(cur),
                Err(err) => self.model_settings.note = Some(err.reason()),
            },
            Message::ModelSaved(result) => match result {
                Ok(cur) => {
                    self.model_settings.saved(cur);
                    return Task::perform(
                        call(&self.api, |api| api.test_model()),
                        Message::ModelTested,
                    );
                }
                Err(err) => self.model_settings.save_failed(&err.reason()),
            },
            Message::ModelTested(result) => match result {
                Ok((ok, error)) => self.model_settings.tested(ok, error),
                Err(err) => self.model_settings.tested(false, Some(err.reason())),
            },
            Message::OpenSkills => {
                self.page = Page::Skills;
                self.narrow_open = true;
                return self.load_skills();
            }
            Message::OpenMcp => {
                self.page = Page::Mcp;
                self.narrow_open = true;
                return self.load_mcp();
            }
            Message::OpenReviewRules => {
                self.page = Page::ReviewRules;
                self.narrow_open = true;
                self.review_rules.reset_for_open();
                return self.refresh_review_rules();
            }
            Message::ReviewRulesLoaded(result) => match result {
                Ok(rules) => self.review_rules.load(rules),
                Err(err) => self.review_rules.load_failed(&err.reason()),
            },
            Message::ReviewRuleProposed(result) => match result {
                Ok(event) => self.review_rules.proposed_event(&event),
                Err(err) => self.review_rules.propose_failed(&err.reason()),
            },
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
                    if let Some((event @ Event::Confirm { .. }, extras)) = reply {
                        let card = robroz_ui::cards::confirm::ConfirmCard::from_event(&event);
                        let group = card.and_then(|c| self.find_card(&c.id)).map(|(g, _)| g);
                        if let Some(group) = group {
                            self.apply_event_with(group, &event, extras.as_ref());
                        }
                    }
                }
                Err(ApiError::Conflict(_)) => {
                    // Answered elsewhere: say so here and fetch the real state.
                    self.memories
                        .decide_failed(&id, confirm::ALREADY_ANSWERED.as_str());
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
            Message::SkillsLoaded(result) => {
                self.sync_skill_context();
                match result {
                    Ok(list) => {
                        self.skills.load(list);
                        self.skills.note = None;
                    }
                    Err(err) => self.skills.note = Some(err.reason()),
                }
                return self.ensure_group_skill_fetches();
            }
            Message::SkillSaved { slug, result } => match (slug, result) {
                (None, Ok(s)) => self.skills.added(s),
                (Some(_), Ok(s)) => self.skills.saved(s),
                (None, Err(err)) => self.skills.add_failed(&err.reason()),
                (Some(slug), Err(err)) => self.skills.entry_failed(&slug, &err.reason()),
            },
            Message::SkillDecided { slug, keep, result } => match result {
                Ok(reply) => {
                    self.skills.decided(&slug, keep);
                    if let Some((event @ Event::Confirm { .. }, extras)) = reply {
                        let card = robroz_ui::cards::confirm::ConfirmCard::from_event(&event);
                        let group = card.and_then(|c| self.find_card(&c.id)).map(|(g, _)| g);
                        if let Some(group) = group {
                            self.apply_event_with(group, &event, extras.as_ref());
                        }
                    }
                }
                Err(ApiError::Conflict(_)) => {
                    self.skills
                        .decide_failed(&slug, confirm::ALREADY_ANSWERED.as_str());
                    return Task::perform(
                        call(&self.api, |api| api.skills()),
                        Message::SkillsLoaded,
                    );
                }
                Err(err) => self
                    .skills
                    .decide_failed(&slug, &confirm::decide_failed(&err.reason())),
            },
            Message::SkillForgot { slug, result } => match result {
                Ok(()) | Err(ApiError::NotFound(_)) => self.skills.deleted(&slug),
                Err(err) => self.skills.entry_failed(&slug, &err.reason()),
            },
            Message::SkillToggled {
                slug,
                group,
                on,
                result,
            } => match result {
                Ok(()) => self.skills.toggled(&slug, &group, on),
                Err(err) => self.skills.entry_failed(&slug, &err.reason()),
            },
            Message::GroupSkillsLoaded { group, result } => match result {
                Ok(list) => self.skills.apply_group_skills(&group, &list),
                Err(_) => self.skills.group_skills_failed(&group),
            },
            Message::McpLoaded(result) => {
                self.sync_mcp_context();
                match result {
                    Ok(list) => {
                        self.mcp.load(list);
                        self.mcp.note = None;
                    }
                    Err(err) => self.mcp.note = Some(err.reason()),
                }
                return self.ensure_group_mcp_fetches();
            }
            Message::McpSaved { result } => match result {
                Ok(server) => {
                    let id = server.id.clone();
                    self.mcp.saved(server);
                    return Task::batch([self.test_mcp(id.clone()), self.fetch_mcp_tools(id)]);
                }
                Err(err) => self.mcp.save_failed(&err.reason()),
            },
            Message::McpDeleted { id, result } => match result {
                Ok(()) | Err(ApiError::NotFound(_)) => self.mcp.deleted(&id),
                Err(err) => self.mcp.delete_failed(&id, &err.reason()),
            },
            Message::McpToolsLoaded { id, result } => match result {
                Ok(tools) => self.mcp.tools_loaded(&id, tools),
                Err(err) => {
                    if let Some(denied) = err.command_not_approved() {
                        self.mcp.show_command_prompt(
                            &id,
                            denied.command.clone(),
                            denied.args.clone(),
                        );
                    } else {
                        self.mcp.tools_failed(&id, &err.reason());
                    }
                }
            },
            Message::McpTested { id, result } => match result {
                Ok(test) => self.mcp.tested(&id, &test),
                Err(err) => {
                    if let Some(denied) = err.command_not_approved() {
                        self.mcp.show_command_prompt(
                            &id,
                            denied.command.clone(),
                            denied.args.clone(),
                        );
                    } else {
                        self.mcp.test_failed(&id, &err.reason());
                    }
                }
            },
            Message::McpCommandApproved { id, result } => match result {
                Ok(server) => {
                    let sid = server.id.clone();
                    self.mcp.command_approved(server);
                    return Task::batch([self.test_mcp(sid.clone()), self.fetch_mcp_tools(sid)]);
                }
                Err(err) => self.mcp.approve_command_failed(&id, &err.reason()),
            },
            Message::McpEnabled { id, result } => match result {
                Ok(server) => self.mcp.set_enabled(&server.id, server.enabled),
                Err(err) => self.mcp.entry_failed(&id, &err.reason()),
            },
            Message::McpGroupToggled {
                id,
                group,
                on,
                result,
            } => match result {
                Ok(()) => self.mcp.group_toggled(&id, &group, on),
                Err(err) => self.mcp.entry_failed(&id, &err.reason()),
            },
            Message::GroupMcpLoaded { group, result } => match result {
                Ok(list) => self.mcp.apply_group_mcp(&group, &list),
                Err(_) => self.mcp.group_mcp_failed(&group),
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
                    if self.page == Page::Skills {
                        tasks.push(self.ensure_group_skill_fetches());
                    }
                    if self.page == Page::Mcp {
                        tasks.push(self.ensure_group_mcp_fetches());
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
                Err(err) => self.error = Some(Text::ClientNewBroFailed.fill(&[&err.reason()])),
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
                        self.rename_note =
                            Some((group, Text::ClientRenameFailed.fill(&[&err.reason()])));
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
                            .insert(group, Text::ClientSendFailed.fill(&[&err.reason()]));
                    }
                }
            }
            Message::Live((group, item)) => match item {
                StreamItem::Connected => {
                    if self.selected.as_deref() == Some(group.as_str()) {
                        self.stream_up = true;
                    }
                }
                StreamItem::Event { event, extras } => {
                    self.apply_event_with(group, &event, extras.as_ref())
                }
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
                        self.load_failed = false;
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
                    Err(err) => {
                        self.load_failed = !self.loaded;
                        self.error = Some(err.0);
                    }
                }
            }
            Message::RetryLoad => {
                if !self.in_flight && self.host_state().retries() {
                    self.in_flight = true;
                    let load = blocking(&self.source, |s| s.groups());
                    return Task::perform(load, Message::Loaded);
                }
            }
            Message::Polled(result) => {
                self.in_flight = false;
                match result {
                    Ok(polled) => {
                        for ge in polled.events {
                            self.apply_event_with(
                                ge.group_id,
                                &ge.event,
                                ge.approval_extras.as_ref(),
                            );
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
                                    by: Some(robroz_ui::cards::ME_MARK.into()),
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
                        let in_page = self
                            .review_rules
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.id == card);
                        if in_page {
                            match &reply {
                                Some((event, extras)) => {
                                    self.sync_review_pending(event, extras.as_ref());
                                }
                                None => self.review_rules.finish_pending(),
                            }
                            if self
                                .review_rules
                                .pending
                                .as_ref()
                                .is_some_and(|p| matches!(p.state, ApprovalState::Expired { .. }))
                            {
                                return Task::none();
                            }
                            return self.refresh_review_rules();
                        }
                        match (reply, found) {
                            // The host's answered card replaces the pending one.
                            (Some((event, extras)), Some((group, _))) => {
                                self.apply_event_with(group, &event, extras.as_ref());
                            }
                            // No echo (fake API): show the answer locally.
                            (None, Some((group, Card::Approval(mut c)))) => {
                                let at = now_unix();
                                c.state = if approve {
                                    ApprovalState::Approved {
                                        by: Some(robroz_ui::cards::ME_MARK.into()),
                                        at,
                                    }
                                } else {
                                    ApprovalState::Denied {
                                        by: Some(robroz_ui::cards::ME_MARK.into()),
                                        at,
                                    }
                                };
                                self.set_card(&group, Card::Approval(c));
                            }
                            (None, Some((group, Card::Confirm(mut c)))) => {
                                let (by, at) = (Some(robroz_ui::cards::ME_MARK.into()), now_unix());
                                c.state = if approve {
                                    ConfirmState::Kept { by, at }
                                } else {
                                    ConfirmState::Dropped { by, at }
                                };
                                if let Some(m) = &c.memory_id {
                                    self.memories.answered(m, approve);
                                }
                                if let Some(slug) = &c.slug {
                                    self.skills.answered(slug, approve);
                                }
                                self.set_card(&group, Card::Confirm(c));
                            }
                            _ => {}
                        }
                    }
                    Err(ApiError::Conflict(_)) => {
                        if self
                            .review_rules
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.id == card)
                        {
                            self.review_rules.answered_elsewhere();
                            return self.refresh_review_rules();
                        }
                        // Answered already (elsewhere): fetch the real state;
                        // the answered card clears this note when it lands.
                        ui.action_done(Some(Note::AlreadyDecided));
                        self.has_more = true;
                        return self.poll();
                    }
                    Err(err) => {
                        if self
                            .review_rules
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.id == card)
                        {
                            self.review_rules.busy = false;
                            self.review_rules.note = Some(err.reason());
                        }
                        ui.action_done(Some(err.note()));
                    }
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
            Message::SetLocale(locale) => {
                if locale != i18n::locale() {
                    i18n::set_locale(locale);
                    // Stored text was drawn in the old language. Cards and
                    // statuses render on the fly; previews are cached.
                    for (group, line) in self.previews.redraw() {
                        self.convs.set_preview(group, line);
                    }
                    if !self.fake {
                        prefs::save(locale);
                    }
                }
            }
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
        if self.host_state().retries() && self.gallery.is_none() {
            subs.push(iced::time::every(self.interval).map(|_| Message::RetryLoad));
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
            if !self.host_state().shows_sidebar() || (self.narrow_open && self.page == Page::Group)
            {
                self.page_view(&t)
            } else if self.narrow_open {
                let back = button(text(Text::ViewBackToChats).size(t.type_.row_name))
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
            (Some(_), true) => Text::ClientLive.as_str(),
            (Some(_), false) => Text::ClientPolling.as_str(),
        };
        // While the host has never answered, the main pane already shows
        // why; the footer keeps only the theme toggle and drops the raw
        // "Can't reach the host … io: … (os error …)" line.
        let status = if self.host_state() == HostState::Unreachable {
            None
        } else {
            Some(match &self.error {
                Some(err) => err.clone(),
                None => format!("{}{live}", self.origin),
            })
        };
        let theme = button(
            text(match self.mode {
                Mode::Light => Text::ClientThemeDark.as_str(),
                Mode::Dark => Text::ClientThemeLight.as_str(),
            })
            .size(t.type_.foot),
        )
        .style(view::nav_button_style(t, false))
        .on_press(Message::ToggleTheme);
        let footer = match status {
            Some(status) => row![
                text(status)
                    .size(t.type_.foot)
                    .color(color(t.palette.text3)),
                space::horizontal(),
                theme,
            ],
            None => row![space::horizontal(), theme],
        }
        .align_y(Alignment::Center);
        container(column![
            main,
            container(footer).padding(Padding::from([4.0, 10.0]))
        ])
        .style(view::page_style(t))
        .into()
    }

    /// The one Settings row for the UI language: label on the left,
    /// English / 中文 (each in its own language) on the right, the active
    /// one highlighted. Sizes follow the text, so longer copy never clips.
    fn language_row(&self, t: &Skin) -> Element<'_, Message> {
        let active = i18n::locale();
        let mut choices = row![]
            .spacing(t.controls.button_gap)
            .align_y(Alignment::Center);
        for locale in Locale::ALL {
            choices = choices.push(
                button(text(locale.native_name()).size(t.type_.body))
                    .height(Length::Fixed(t.controls.button_h))
                    .style(view::nav_button_style(*t, locale == active))
                    .on_press(Message::SetLocale(locale)),
            );
        }
        let r = row![
            text(Text::SettingsLanguage)
                .size(t.type_.body)
                .color(color(t.palette.text)),
            space::horizontal(),
            choices,
        ]
        .align_y(Alignment::Center);
        container(container(r).padding(Padding::from([
            t.metrics.row_pad_h * 0.75,
            t.metrics.row_pad_h,
        ])))
        .width(Fill)
        .style(view::card_style(*t))
        .into()
    }

    fn group_list(&self, t: &Skin, width: Length) -> Element<'_, Message> {
        if !self.host_state().shows_sidebar() {
            // Nothing to list until the host answers: no empty-account hint.
            return container(space::horizontal())
                .width(width)
                .height(Fill)
                .padding(8)
                .style(view::sidebar_style(*t))
                .into();
        }
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
        let skills = button(text(robroz_ui::cards::skills::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::Skills))
            .on_press(Message::OpenSkills);
        let model_nav = button(text(model_page::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::Model))
            .on_press(Message::OpenModel);
        let search = button(text(search_page::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::Search))
            .on_press(Message::OpenSearch);
        let mcp = button(text(robroz_ui::cards::mcp::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::Mcp))
            .on_press(Message::OpenMcp);
        let review_rules = button(text(review_rules::TITLE).size(t.type_.row_name))
            .width(Fill)
            .height(Length::Fixed(
                t.controls.touch_target.max(t.controls.button_h),
            ))
            .padding(Padding::from([0.0, 12.0]))
            .style(view::nav_button_style(*t, self.page == Page::ReviewRules))
            .on_press(Message::OpenReviewRules);
        container(
            column![
                scrollable(list).height(Fill),
                routines,
                memories,
                skills,
                model_nav,
                search,
                mcp,
                review_rules
            ]
            .spacing(4),
        )
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
        let state = self.host_state();
        if !state.shows_sidebar() {
            // Before the host answers: blank while waiting, or 「连不上宿主」
            // with the address on a lighter line below, a short reason
            // 12px to the right when we have one, and the same Language
            // row as the Model page so the language can still be changed.
            let notice = state.notice().map(|key| {
                let mut detail = row![text(self.origin.as_str())
                    .size(t.type_.foot)
                    .color(color(t.palette.text3))]
                .spacing(12)
                .align_y(Alignment::Center);
                if let Some(reason) = self.error.as_deref().and_then(host_state::connect_reason) {
                    detail = detail.push(
                        text(reason)
                            .size(t.type_.foot)
                            .color(color(t.palette.text3)),
                    );
                }
                let head = column![
                    text(key).size(t.type_.body).color(color(t.palette.text)),
                    detail,
                ]
                .spacing(4)
                .align_x(Alignment::Center);
                if state.shows_language_row() {
                    Element::from(
                        column![head, self.language_row(t)]
                            .spacing(t.metrics.row_pad_h)
                            .align_x(Alignment::Center)
                            .width(Fill),
                    )
                } else {
                    Element::from(head)
                }
            });
            return container(
                notice.map_or_else(|| Element::from(space::horizontal()), Element::from),
            )
            .padding(t.metrics.stream_padding)
            .center(Fill)
            .into();
        }
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
        if self.page == Page::Skills {
            return scrollable(
                container(
                    view::skills_panel(t, &self.skills, now_unix(), self.content_width(t))
                        .map(Message::Ui),
                )
                .padding(t.metrics.stream_padding)
                .width(Fill),
            )
            .height(Fill)
            .width(Fill)
            .into();
        }
        if self.page == Page::Mcp {
            return scrollable(
                container(
                    view::mcp_panel(t, &self.mcp, now_unix(), self.content_width(t))
                        .map(Message::Ui),
                )
                .padding(t.metrics.stream_padding)
                .width(Fill),
            )
            .height(Fill)
            .width(Fill)
            .into();
        }
        if self.page == Page::Model {
            return scrollable(
                container(
                    column![
                        view::model_panel(t, &self.model_settings).map(Message::Ui),
                        self.language_row(t),
                    ]
                    .spacing(t.metrics.row_pad_h),
                )
                .padding(t.metrics.stream_padding)
                .width(Fill),
            )
            .height(Fill)
            .width(Fill)
            .into();
        }
        if self.page == Page::Search {
            return scrollable(
                container(view::search_panel(t, &self.search).map(Message::Ui))
                    .padding(t.metrics.stream_padding)
                    .width(Fill),
            )
            .height(Fill)
            .width(Fill)
            .into();
        }
        if self.page == Page::ReviewRules {
            return scrollable(
                container(view::review_rules_panel(t, &self.review_rules).map(Message::Ui))
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
            return container(text(Text::ClientPickChat).color(color(t.palette.text2)))
                .center(Fill)
                .into();
        };
        let width = self.content_width(t);
        let in_group = self.convs.get(id).is_some_and(|c| !c.dm);
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
                Fold::Show => Some(row_view(t, r, &self.ui, &empty, self.caps, width, in_group)),
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
        let section = |name: &'static str,
                       samples: &'a [gallery::Sample]|
         -> Element<'a, Message> {
            let mut c = column![title(name)].spacing(6);
            for s in samples {
                c = c.push(caption(s.caption)).push(
                    view::card(t, &s.card, &s.ui, Caps::FAKE, col_width, false).map(Message::Ui),
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
    in_group: bool,
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
            view::card(t, card, state, caps, width, in_group).map(Message::Ui)
        }
    }
}

/// Frozen status card: palette from `robroz-card`, outline from the skin.
fn status_view<'a>(t: &Skin, card: &'a StatusCard, width: f32) -> Element<'a, Message> {
    let line = progress_cell(t, card, width);
    let mut line = row![line, text(model::display(&card.status_line).into_owned())]
        .spacing(12)
        .align_y(Alignment::Center);
    if let Some(action) = card.action {
        line = line.push(
            button(text(model::display(action).into_owned()).size(t.type_.button))
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
    let right = unit(&model::display(&card.status_line)) * t.type_.body * t.type_.unit_width
        + card.action.map_or(0.0, |a| {
            unit(&model::display(a)) * t.type_.button * t.type_.unit_width
                + 2.0 * t.controls.card_pad_h
                + 12.0
        })
        + 12.0;
    let room = width - 2.0 * (t.metrics.status_pad_h + t.stroke.card) - right;
    let cols = (room / (size * t.type_.unit_width)).max(8.0) as usize;
    let fitted = robroz_ui::cards::status::clamp_lines(&model::display(progress), cols, 1);
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

fn delivery_setup(card: &DeliveryCard) -> Vec<(Message, Text, Text)> {
    let mut out = Vec::new();
    if card.needs_model_setup {
        out.push((
            Message::OpenModel,
            model_page::GO_SETUP,
            model_page::NEEDS_SETUP_HINT,
        ));
    }
    if card.needs_search_setup {
        out.push((
            Message::OpenSearch,
            search_page::GO_SETUP,
            search_page::NEEDS_SETUP_HINT,
        ));
    }
    out
}

fn delivery_view<'a>(t: &Skin, card: &'a DeliveryCard, width: f32) -> Element<'a, Message> {
    // Did not get done: the same grey as 「没写出结果」, no path, no buttons.
    if let Some(failure) = &card.failure {
        let gray = robroz_card::palette(robroz_card::Tone::Gray);
        let line = text(model::display(&model::failure_line(failure)).into_owned());
        let setup = delivery_setup(card);
        let body: Element<'a, Message> = if setup.is_empty() {
            line.into()
        } else {
            let mut r = row![line, space::horizontal()];
            for (msg, label, _) in setup {
                r = r.push(
                    button(text(label).size(t.type_.button))
                        .style(view::nav_button_style(*t, false))
                        .on_press(msg),
                );
            }
            r.align_y(Alignment::Center).into()
        };
        return container(body)
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
    for (msg, label, _) in delivery_setup(card) {
        head = head.push(
            button(text(label).size(t.type_.button))
                .style(view::nav_button_style(*t, false))
                .on_press(msg),
        );
    }
    // Display only: plain text, not a link or button.
    let path_line = text(path)
        .wrapping(iced::widget::text::Wrapping::None)
        .font(Font::MONOSPACE)
        .size(t.type_.mono)
        .color(color(t.palette.text2));
    let mut body = column![head, path_line].spacing(6);
    for (_, _, hint) in delivery_setup(card) {
        body = body.push(text(hint).size(t.type_.foot).color(color(t.palette.text2)));
    }
    container(body)
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
