//! openbots desktop client, phase-1 shell.
//!
//! Left: groups. Right: the selected group's message stream. Events come from
//! an [`EventSource`]; this build only wires the scripted [`FakeSource`].

mod model;
mod source;

use std::collections::HashMap;
use std::time::Duration;

use hive_card::Palette;
use hive_contract::Group;
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Border, Color, Element, Fill, Font, Length, Subscription, Theme};

use model::{attachment_label, DeliveryCard, Feed, Row, StatusCard};
use source::{EventSource, FakeSource};

const FAKE_TICK: Duration = Duration::from_millis(900);

pub fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("openbots")
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size((1040.0, 680.0))
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Select(String),
    Tick,
    TakeOver(String),
}

struct App {
    source: Box<dyn EventSource>,
    groups: Vec<Group>,
    feeds: HashMap<String, Feed>,
    selected: Option<String>,
}

impl App {
    fn new() -> Self {
        Self::with_source(Box::new(FakeSource::new()))
    }

    fn with_source(source: Box<dyn EventSource>) -> Self {
        let groups = source.groups();
        let selected = groups.first().map(|g| g.id.clone());
        let mut app = Self {
            source,
            groups,
            feeds: HashMap::new(),
            selected,
        };
        app.pull();
        app
    }

    fn pull(&mut self) {
        for ge in self.source.poll() {
            self.feeds.entry(ge.group_id).or_default().apply(&ge.event);
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Select(id) => self.selected = Some(id),
            Message::Tick => self.pull(),
            // Node 6 opens the private chat. For now this only logs.
            Message::TakeOver(task_id) => eprintln!("去私聊接管: task {task_id} (not wired yet)"),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.source.has_more() {
            iced::time::every(FAKE_TICK).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn theme(&self) -> Theme {
        Theme::Light
    }

    fn view(&self) -> Element<'_, Message> {
        row![self.group_list(), self.stream()].height(Fill).into()
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
    }
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
