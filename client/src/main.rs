//! robroz desktop client.
//!
//! Left: groups and 例行任务. Right: the selected group's message stream or
//! the routines panel. Events come from an [`EventSource`]: the live host by
//! default, or the scripted fake with `--fake` / `ROBROZ_FAKE=1`. Card
//! clicks go through [`api::CardApi`]. `--gallery` shows every card state
//! for design review. Visuals come from the `robroz-ui` skin
//! (`ROBROZ_THEME=dark` starts dark; the footer button toggles).

mod api;
mod gallery;
mod model;
mod source;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{window, Alignment, Element, Fill, Font, Length, Padding, Size, Subscription, Task};
use robroz_contract::{Event, ScheduleEntry};
use robroz_ui::cards::interact::CardUi;
use robroz_ui::cards::routines::{RoutineRow, Routines};
use robroz_ui::cards::{ApprovalState, Card, ChoiceState, DraftState};
use robroz_ui::view::{self, color, Action, Caps};
use robroz_ui::{Mode, Skin};

use api::{ApiError, CardApi, FakeApi, HttpApi};
use gallery::Gallery;
use model::{attachment_label, DeliveryCard, Feed, Row, StatusCard};
use source::{
    fake_schedules, EventSource, FakeSource, GroupCard, GroupEvent, GroupTab, HttpSource,
    SourceError,
};

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

/// Name used in local footnotes after an action (「Panda 批准」).
fn user_name() -> String {
    std::env::var("ROBROZ_USER_NAME").unwrap_or_else(|_| "我".into())
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
}

#[derive(Debug, Clone)]
enum Message {
    Select(String),
    OpenRoutines,
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
    RoutinesLoaded(Result<Vec<ScheduleEntry>, ApiError>),
    Paused {
        id: String,
        wanted: bool,
        result: Result<ScheduleEntry, ApiError>,
    },
    ClearPauseNote(String),
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
    origin: String,
    interval: Duration,
    groups: Vec<GroupTab>,
    feeds: HashMap<String, Feed>,
    selected: Option<String>,
    page: Page,
    ui: HashMap<String, CardUi>,
    routines: Routines,
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
        let (source, api, origin): (Box<dyn EventSource>, Arc<dyn CardApi>, String) = if fake {
            (
                Box::new(FakeSource::new()),
                Arc::new(FakeApi::new(fake_schedules())),
                "假事件".into(),
            )
        } else {
            let http = HttpSource::from_env();
            let origin = http.base().to_string();
            let api = Arc::new(HttpApi::new(origin.clone()));
            (Box::new(http), api, origin)
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
            origin,
            interval,
            groups: Vec::new(),
            feeds: HashMap::new(),
            selected: None,
            page: Page::Group,
            ui: HashMap::new(),
            routines: Routines::default(),
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
        if app.gallery.is_some() {
            // `ROBROZ_SHOT=<file>`: write the window's RGBA after layout
            // settles, then exit. Used for review screenshots.
            let task = if std::env::var("ROBROZ_SHOT").is_ok() {
                Task::perform(tokio_sleep(Duration::from_millis(2500)), |_| ())
                    .then(|_| window::latest())
                    .map(Message::Shoot)
            } else {
                Task::none()
            };
            return (app, task);
        }
        let load = blocking(&app.source, |s| s.groups());
        (app, Task::perform(load, Message::Loaded))
    }

    fn skin(&self) -> Skin {
        Skin::for_mode(self.mode).for_input(self.touch, self.width)
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

    fn on_action(&mut self, action: Action) -> Task<Message> {
        match action {
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
                // Remember the decision so the local state can follow.
                let job = async move { (job.await, approve) };
                Task::perform(job, move |(result, approve)| Message::Acted {
                    card: format!("{}{card}", if approve { "+" } else { "-" }),
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
            Message::Select(id) => {
                self.selected = Some(id);
                self.page = Page::Group;
            }
            Message::OpenRoutines => {
                self.page = Page::Routines;
                return self.load_routines();
            }
            Message::Tick => return self.poll(),
            Message::Loaded(result) => {
                self.in_flight = false;
                match result {
                    Ok(groups) => {
                        self.selected = groups.first().map(|g| g.id.clone());
                        self.groups = groups;
                        self.loaded = true;
                        self.error = None;
                        return self.poll();
                    }
                    Err(err) => self.error = Some(err.0),
                }
            }
            Message::Polled(result) => {
                self.in_flight = false;
                match result {
                    Ok(polled) => {
                        for ge in polled.events {
                            self.feeds.entry(ge.group_id).or_default().apply(&ge.event);
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
                                    elsewhere: false,
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
            Message::Acted { card, result } => {
                let (tag, id) = card.split_at(1);
                let id = id.to_string();
                let ui = self.ui.entry(id.clone()).or_default();
                match result {
                    Ok(()) => {
                        ui.action_done(None);
                        let (by, at) = (user_name(), now_unix());
                        match self.find_card(&id) {
                            Some((group, Card::Approval(mut c))) => {
                                c.state = if tag == "+" {
                                    ApprovalState::Approved { by, at }
                                } else {
                                    ApprovalState::Denied { by, at }
                                };
                                self.set_card(&group, Card::Approval(c));
                            }
                            Some((group, Card::Draft(mut c))) => {
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
            subs.push(iced::time::every(self.interval).map(|_| Message::Tick));
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
        let main = row![self.group_list(&t), self.page_view(&t)].height(Fill);
        let status = match &self.error {
            Some(err) => err.as_str(),
            None => self.origin.as_str(),
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

    fn group_list(&self, t: &Skin) -> Element<'_, Message> {
        let nav = |label: String, count: Option<usize>, selected: bool, msg: Message| {
            let mut content = row![text(label).size(t.type_.row_name)].spacing(8);
            if let Some(n) = count {
                content = content.push(space::horizontal()).push(
                    text(n.to_string())
                        .size(t.type_.foot)
                        .color(color(t.palette.text3)),
                );
            }
            button(content.align_y(Alignment::Center))
                .width(Fill)
                .height(Length::Fixed(
                    t.controls.touch_target.max(t.controls.button_h),
                ))
                .padding(Padding::from([0.0, 12.0]))
                .style(view::nav_button_style(*t, selected))
                .on_press(msg)
        };
        let mut items: Vec<Element<'_, Message>> = self
            .groups
            .iter()
            .map(|g| {
                let selected =
                    self.page == Page::Group && self.selected.as_deref() == Some(g.id.as_str());
                let count = self.feeds.get(&g.id).map_or(0, Feed::len);
                nav(
                    g.name.clone(),
                    Some(count),
                    selected,
                    Message::Select(g.id.clone()),
                )
                .into()
            })
            .collect();
        items.push(space::vertical().into());
        items.push(
            nav(
                robroz_ui::cards::routines::TITLE.into(),
                None,
                self.page == Page::Routines,
                Message::OpenRoutines,
            )
            .into(),
        );
        container(column(items).spacing(4))
            .width(Length::Fixed(t.controls.sidebar_w))
            .height(Fill)
            .padding(8)
            .style(view::sidebar_style(*t))
            .into()
    }

    fn content_width(&self, t: &Skin) -> f32 {
        let chrome = if self.gallery.is_some() {
            0.0
        } else {
            t.controls.sidebar_w
        };
        (self.width - chrome - 2.0 * t.metrics.stream_padding).max(200.0)
    }

    fn page_view(&self, t: &Skin) -> Element<'_, Message> {
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
            return container(text("选一个群").color(color(t.palette.text2)))
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
            .map(|r| row_view(t, r, &self.ui, &empty, self.caps, width));
        scrollable(
            column(rows)
                .spacing(t.metrics.stream_spacing)
                .padding(t.metrics.stream_padding)
                .width(Fill),
        )
        .anchor_bottom()
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
        Row::Status(card) => status_view(t, card),
        Row::Delivery(card) => delivery_view(t, card, width),
        Row::Card(card) => {
            let state = ui.get(card.id()).unwrap_or(empty);
            view::card(t, card, state, caps, width).map(Message::Ui)
        }
    }
}

/// Frozen status card: palette from `robroz-card`, outline from the skin.
fn status_view<'a>(t: &Skin, card: &'a StatusCard) -> Element<'a, Message> {
    let mut line = row![
        text(&card.bot_name),
        space::horizontal(),
        text(card.status_line)
    ]
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

fn delivery_view<'a>(t: &Skin, card: &'a DeliveryCard, width: f32) -> Element<'a, Message> {
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
