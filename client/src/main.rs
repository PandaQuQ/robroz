//! robroz desktop client.
//!
//! Left: groups. Right: the selected group's message stream. Events come from
//! an [`EventSource`]: the live host by default, or the scripted fake with
//! `--fake` / `ROBROZ_FAKE=1`.

mod model;
mod source;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use robroz_card::Palette;
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Border, Color, Element, Fill, Font, Length, Subscription, Task, Theme};

use model::{attachment_label, ChoiceCard, DeliveryCard, Feed, Row, StatusCard};
use source::{EventSource, FakeSource, GroupEvent, GroupTab, HttpSource, SourceError};

pub fn main() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title("robroz")
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size((1040.0, 680.0))
        .run()
}

fn wants_fake() -> bool {
    std::env::args().any(|a| a == "--fake")
        || std::env::var("ROBROZ_FAKE").is_ok_and(|v| v == "1")
}

type Shared = Arc<Mutex<Box<dyn EventSource>>>;

#[derive(Debug, Clone)]
struct Polled {
    events: Vec<GroupEvent>,
    has_more: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Select(String),
    Tick,
    Loaded(Result<Vec<GroupTab>, SourceError>),
    Polled(Result<Polled, SourceError>),
    TakeOver(String),
}

struct App {
    source: Shared,
    origin: String,
    interval: Duration,
    groups: Vec<GroupTab>,
    feeds: HashMap<String, Feed>,
    selected: Option<String>,
    loaded: bool,
    in_flight: bool,
    has_more: bool,
    error: Option<String>,
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

impl App {
    fn boot() -> (Self, Task<Message>) {
        let (source, origin): (Box<dyn EventSource>, String) = if wants_fake() {
            (Box::new(FakeSource::new()), "假事件".into())
        } else {
            let http = HttpSource::from_env();
            let origin = http.base().to_string();
            (Box::new(http), origin)
        };
        let interval = source.interval();
        let app = Self {
            source: Arc::new(Mutex::new(source)),
            origin,
            interval,
            groups: Vec::new(),
            feeds: HashMap::new(),
            selected: None,
            loaded: false,
            in_flight: true,
            has_more: true,
            error: None,
        };
        let load = blocking(&app.source, |s| s.groups());
        (app, Task::perform(load, Message::Loaded))
    }

    fn poll(&mut self) -> Task<Message> {
        if self.in_flight || !self.loaded || !self.has_more {
            return Task::none();
        }
        self.in_flight = true;
        let poll = blocking(&self.source, |s| {
            s.poll().map(|events| Polled {
                events,
                has_more: s.has_more(),
            })
        });
        Task::perform(poll, Message::Polled)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Select(id) => self.selected = Some(id),
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
                        self.has_more = polled.has_more;
                        self.error = None;
                    }
                    // Keep what is on screen and try again next tick.
                    Err(err) => self.error = Some(err.0),
                }
            }
            // Opening the private chat is not wired yet; this only logs.
            Message::TakeOver(task_id) => eprintln!("去私聊接管: task {task_id} (not wired yet)"),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.loaded && self.has_more {
            iced::time::every(self.interval).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn theme(&self) -> Theme {
        Theme::Light
    }

    fn view(&self) -> Element<'_, Message> {
        let main = row![self.group_list(), self.stream()].height(Fill);
        let footer = match &self.error {
            Some(err) => text(err.as_str()).size(12),
            None => text(self.origin.as_str()).size(12),
        };
        column![main, container(footer).padding([4, 10])].into()
    }

    fn group_list(&self) -> Element<'_, Message> {
        let items = self.groups.iter().map(|g| {
            let selected = self.selected.as_deref() == Some(g.id.as_str());
            let count = self.feeds.get(&g.id).map_or(0, Feed::len);
            button(
                row![
                    text(&g.name),
                    space::horizontal(),
                    text(count.to_string()).size(12)
                ]
                .spacing(8),
            )
            .width(Fill)
            .padding([10, 12])
            .style(if selected {
                button::primary
            } else {
                button::text
            })
            .on_press(Message::Select(g.id.clone()))
            .into()
        });
        container(column(items).spacing(4))
            .width(Length::Fixed(240.0))
            .height(Fill)
            .padding(8)
            .style(container::bordered_box)
            .into()
    }

    fn stream(&self) -> Element<'_, Message> {
        let Some(id) = &self.selected else {
            return container(text("选一个群")).center(Fill).into();
        };
        let rows = self
            .feeds
            .get(id)
            .into_iter()
            .flat_map(Feed::rows)
            .map(row_view);
        scrollable(column(rows).spacing(10).padding(16).width(Fill))
            .anchor_bottom()
            .height(Fill)
            .width(Fill)
            .into()
    }
}

fn rgb(c: [u8; 3]) -> Color {
    Color::from_rgb8(c[0], c[1], c[2])
}

fn row_view(r: &Row) -> Element<'_, Message> {
    match r {
        Row::Text { body } => text(body).into(),
        Row::Attachment { kind, name } => row![
            container(text(attachment_label(*kind)).size(12))
                .padding([2, 6])
                .style(container::rounded_box),
            text(name),
        ]
        .spacing(8)
        .into(),
        Row::Status(card) => status_view(card),
        Row::Delivery(card) => delivery_view(card),
        Row::Choice(card) => choice_view(card),
    }
}

fn choice_view(card: &ChoiceCard) -> Element<'_, Message> {
    let mut options = row![].spacing(8);
    for (value, label) in &card.options {
        let picked = card.choice.as_deref() == Some(value.as_str());
        let style = if picked { button::primary } else { button::secondary };
        options = options.push(button(text(label).size(13)).style(style));
    }
    container(column![text(&card.bot_name).size(12), text(&card.prompt), options].spacing(6))
        .padding([10, 14])
        .width(Fill)
        .style(container::rounded_box)
        .into()
}

fn card_style(p: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(rgb(p.text)),
        background: Some(rgb(p.fill).into()),
        border: Border::default().rounded(8),
        ..container::Style::default()
    }
}

fn status_view(card: &StatusCard) -> Element<'_, Message> {
    let mut line = row![
        text(&card.bot_name),
        space::horizontal(),
        text(card.status_line)
    ]
    .spacing(12)
    .align_y(iced::Center);
    if let Some(action) = card.action {
        line = line.push(
            button(text(action).size(13))
                .style(button::secondary)
                .on_press(Message::TakeOver(card.task_id.clone())),
        );
    }
    container(line)
        .padding([10, 14])
        .width(Fill)
        .style(card_style(card.palette))
        .into()
}

fn delivery_view(card: &DeliveryCard) -> Element<'_, Message> {
    let mut head = row![text(&card.summary), space::horizontal()].align_y(iced::Center);
    if let Some(badge) = card.badge {
        head = head.push(
            container(text(badge).size(12))
                .padding([2, 8])
                .style(container::primary),
        );
    }
    container(
        column![
            head,
            // Display only: plain text, not a link or button.
            text(&card.path).font(Font::MONOSPACE).size(13),
        ]
        .spacing(6),
    )
    .padding([10, 14])
    .width(Fill)
    .style(container::bordered_box)
    .into()
}
