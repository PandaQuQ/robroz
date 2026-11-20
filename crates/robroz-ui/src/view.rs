//! iced widgets for the new cards and the routines panel.
//!
//! Every colour, radius, border, padding and size comes from [`Skin`].
//! Paths and addresses are plain monospace text, never links. Widgets only
//! emit [`Action`]s; the app decides what to call.

use chrono::Local;
use iced::widget::text::{LineHeight, Wrapping};
use iced::widget::{button, canvas, column, container, hover, row, space, text, Space};
use iced::{
    font, Alignment, Background, Border, Color, Element, Fill, Font, Length, Padding, Pixels,
    Shadow as IcedShadow, Theme as IcedTheme, Vector,
};

use crate::cards::interact::{self, ApprovalActions, CardUi, Look, OptionButton};
use crate::cards::routines::{self, Routines};
use crate::cards::status::{self, Dot};
use crate::cards::{ApprovalCard, Card, ChoiceCard, DraftCard};
use crate::conversations::{self, Conversation, Conversations};
use crate::theme::{Rgb, Skin};
use crate::widgets::{Check, CheckBox, Rings, Toggle};

/// What a click on a new card or the routines panel asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Single choice: send this option.
    Choose {
        card: String,
        option: String,
    },
    /// Multi choice: flip a pick locally.
    TogglePick {
        card: String,
        option: String,
    },
    /// Multi choice: send the picks.
    ConfirmChoice {
        card: String,
    },
    Decide {
        card: String,
        approve: bool,
    },
    SendDraft {
        card: String,
    },
    /// Open the bot's DM with the draft; no editor in the group.
    EditDraftInDm {
        card: String,
    },
    /// Stuck card: go to the DM to take over.
    TakeOver {
        card: String,
    },
    TogglePause {
        id: String,
    },
    /// Narrow screens: show or hide the full bot name on a card.
    ToggleBotName {
        card: String,
    },
    /// Conversation list: open this group or DM.
    OpenChat {
        group: String,
    },
    /// 「＋ 新 bro」: create a bro and open its DM.
    NewBro,
}

/// Monospace advance as a fraction of font size (layout estimate only).
const MONO_ADVANCE: f32 = 0.6;

pub fn color(c: Rgb) -> Color {
    Color::from_rgb8(c[0], c[1], c[2])
}

fn faded(c: Rgb, alpha: f32) -> Color {
    let mut c = color(c);
    c.a = alpha;
    c
}

pub fn bold() -> Font {
    Font {
        weight: font::Weight::Bold,
        ..Font::DEFAULT
    }
}

fn shadow(t: &Skin) -> IcedShadow {
    match t.shadows.card {
        Some(s) => IcedShadow {
            color: faded(s.color, s.alpha),
            offset: Vector::new(s.offset.0, s.offset.1),
            blur_radius: s.blur,
        },
        None => IcedShadow::default(),
    }
}

fn gap<'a, M: 'a>(h: f32) -> Element<'a, M> {
    Space::new().height(Length::Fixed(h)).into()
}

// ---------- Surfaces used by the app shell too ----------

/// Page background (message stream, routines page).
pub fn page_style(t: Skin) -> impl Fn(&IcedTheme) -> container::Style {
    move |_| container::Style {
        background: Some(color(t.palette.window).into()),
        text_color: Some(color(t.palette.text)),
        ..container::Style::default()
    }
}

/// Sidebar: surface with a right divider look.
pub fn sidebar_style(t: Skin) -> impl Fn(&IcedTheme) -> container::Style {
    move |_| container::Style {
        background: Some(color(t.palette.sidebar).into()),
        text_color: Some(color(t.palette.text)),
        border: Border::default()
            .width(t.stroke.divider)
            .color(color(t.palette.divider)),
        ..container::Style::default()
    }
}

/// Sidebar entry.
pub fn nav_button_style(
    t: Skin,
    selected: bool,
) -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    move |_, status| {
        let bg = if selected {
            Some(Background::Color(color(t.palette.control_checked)))
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(Background::Color(color(t.palette.control_hover)))
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: color(t.palette.text),
            border: Border::default().rounded(t.radii.row),
            ..button::Style::default()
        }
    }
}

/// New-card frame: `surface`, 1px `border`, radius from the skin.
pub fn card_style(t: Skin) -> impl Fn(&IcedTheme) -> container::Style {
    move |_| container::Style {
        background: Some(color(t.palette.surface).into()),
        text_color: Some(color(t.palette.text)),
        border: Border::default()
            .width(t.stroke.card)
            .color(color(t.palette.border))
            .rounded(t.radii.card),
        shadow: shadow(&t),
        ..container::Style::default()
    }
}

/// Frozen status card: fill and text from `robroz_card::palette()`
/// unchanged; the skin adds only the dark-mode outline.
pub fn status_card_style(
    t: Skin,
    fill: Rgb,
    text_rgb: Rgb,
) -> impl Fn(&IcedTheme) -> container::Style {
    move |_| {
        let border = match t.palette.status_outline {
            Some(c) => Border::default()
                .width(t.stroke.status_outline)
                .color(color(c)),
            None => Border::default(),
        };
        container::Style {
            text_color: Some(color(text_rgb)),
            background: Some(color(fill).into()),
            border: border.rounded(t.radii.status_card),
            ..container::Style::default()
        }
    }
}

pub fn status_card_padding(t: &Skin) -> Padding {
    Padding::from([t.metrics.status_pad_v, t.metrics.status_pad_h])
}

pub fn card_padding(t: &Skin) -> Padding {
    Padding::from([t.controls.card_pad_v, t.controls.card_pad_h])
}

// ---------- Pieces ----------

fn dot<'a, M: 'a>(t: &Skin, d: Dot) -> Element<'a, M> {
    let c = color(t.dot(d));
    let size = t.metrics.dot;
    container(Space::new().width(size).height(size))
        .style(move |_| container::Style {
            background: Some(c.into()),
            border: Border::default().rounded(size / 2.0),
            ..container::Style::default()
        })
        .into()
}

fn header<'a>(
    t: &Skin,
    card: &str,
    bot: &'a str,
    ui: &CardUi,
    label: &'static str,
    d: Dot,
) -> Element<'a, Action> {
    let name = |s: String| {
        text(s)
            .size(t.type_.bot)
            .font(bold())
            .color(color(t.palette.text))
    };
    // Narrow screens: names over 8 characters are cut; tapping toggles the
    // full name. Wide screens always show it whole.
    let bot_name: Element<'a, Action> = match status::short_bot_name(bot).filter(|_| t.narrow) {
        Some(short) => {
            let shown = if ui.name_expanded {
                bot.to_string()
            } else {
                short
            };
            let h = t.metrics.head_height;
            hit_area(
                t,
                name(shown).into(),
                None,
                h,
                Some(Action::ToggleBotName {
                    card: card.to_string(),
                }),
            )
        }
        None => name(bot.to_string()).into(),
    };
    row![
        bot_name,
        space::horizontal(),
        row![
            dot(t, d),
            text(label)
                .size(t.type_.status)
                .color(color(t.palette.text2)),
        ]
        .spacing(t.metrics.dot_gap)
        .align_y(Alignment::Center),
    ]
    .height(Length::Fixed(t.metrics.head_height.max(if t.narrow {
        t.controls.touch_target
    } else {
        0.0
    })))
    .align_y(Alignment::Center)
    .into()
}

fn body<'a, M: 'a>(t: &Skin, s: &'a str) -> Element<'a, M> {
    text(s)
        .size(t.type_.body)
        .line_height(LineHeight::Absolute(Pixels(t.type_.body_line)))
        .color(color(t.palette.text))
        .into()
}

fn footnote<'a, M: 'a>(t: &Skin, s: String) -> Element<'a, M> {
    text(s)
        .size(t.type_.foot)
        .line_height(LineHeight::Absolute(Pixels(t.type_.foot_line)))
        .color(color(t.palette.text3))
        .into()
}

fn tag<'a, M: 'a>(t: &Skin, label: &'static str, fg: Rgb) -> Element<'a, M> {
    let border = t.palette.border2;
    let (bw, r) = (t.stroke.tag, t.radii.tag);
    container(text(label).size(t.type_.tag).color(color(fg)))
        .padding(Padding::from([0.0, t.metrics.tag_pad_h]))
        .height(Length::Fixed(t.metrics.tag_height))
        .align_y(Alignment::Center)
        .style(move |_| container::Style {
            border: Border::default().width(bw).color(color(border)).rounded(r),
            ..container::Style::default()
        })
        .into()
}

/// Relative path or address chip. Plain text: not a link, not a button.
/// Path shown in a card: monospace, never a link. On narrow screens it is
/// shortened from the middle to fit `width` (the card's content width).
fn path_chip<'a, M: 'a>(t: &Skin, s: &'a str, width: f32) -> Element<'a, M> {
    let s: String = if t.narrow {
        let room = width - 2.0 * t.metrics.path_pad_h;
        let max = (room / (t.type_.mono * MONO_ADVANCE)).floor().max(8.0) as usize;
        status::middle_ellipsis(s, max)
    } else {
        s.to_string()
    };
    let bg = t.palette.sunken;
    let r = t.radii.path_chip;
    container(
        text(s)
            .font(Font::MONOSPACE)
            .size(t.type_.mono)
            .wrapping(Wrapping::None)
            .color(color(t.palette.text2)),
    )
    .padding(Padding::from([0.0, t.metrics.path_pad_h]))
    .height(Length::Fixed(t.metrics.path_height))
    .align_y(Alignment::Center)
    .style(move |_| container::Style {
        background: Some(color(bg).into()),
        border: Border::default().rounded(r),
        ..container::Style::default()
    })
    .into()
}

/// A path for display in the given width: shortened from the middle on
/// narrow screens, unchanged otherwise. For paths outside the new cards
/// (the delivery card in the shell).
pub fn display_path(t: &Skin, path: &str, width: f32) -> String {
    if t.narrow {
        let max = (width / (t.type_.mono * MONO_ADVANCE)).floor().max(8.0) as usize;
        status::middle_ellipsis(path, max)
    } else {
        path.to_string()
    }
}

fn mono<'a, M: 'a>(t: &Skin, s: &'a str) -> Element<'a, M> {
    text(s)
        .font(Font::MONOSPACE)
        .size(t.type_.mono)
        .color(color(t.palette.text2))
        .into()
}

#[derive(Clone, Copy)]
enum Leading {
    None,
    Check,
    Box(bool),
}

/// Wrap a visual in a transparent button whose hit area is at least
/// `touch_target` on both axes, centred on the visual.
fn hit_area<'a>(
    t: &Skin,
    visual: Element<'a, Action>,
    visual_w: Option<f32>,
    visual_h: f32,
    on_press: Option<Action>,
) -> Element<'a, Action> {
    let pad_v = ((t.controls.touch_target - visual_h) / 2.0).max(0.0);
    let pad_h = visual_w.map_or(0.0, |w| ((t.controls.touch_target - w) / 2.0).max(0.0));
    button(visual)
        .padding(Padding::from([pad_v, pad_h]))
        .on_press_maybe(on_press)
        .style(|_, _| button::Style {
            background: None,
            ..button::Style::default()
        })
        .into()
}

/// The one button shape (SPEC §3): 32px visual inside a hit area of at
/// least `touch_target`.
/// `enabled` false fades to the disabled alpha unless the look is
/// `Selected` (an answered pick stays readable).
fn btn<'a>(
    t: &Skin,
    label: String,
    look: Look,
    leading: Leading,
    on_press: Option<Action>,
) -> Element<'a, Action> {
    let t = *t;
    let enabled = on_press.is_some();
    let strong = matches!(look, Look::Primary | Look::Selected);
    let faded_out = look == Look::Disabled || (!enabled && look == Look::Primary);
    let alpha = if faded_out {
        t.palette.disabled_alpha
    } else {
        1.0
    };
    let fg = match look {
        Look::Primary => t.palette.on_accent,
        _ => t.palette.text,
    };
    let fg_color = faded(fg, alpha);
    let visual = move |hovered: bool| -> Element<'a, Action> {
        let mut content = row![]
            .spacing(t.metrics.check_gap)
            .align_y(Alignment::Center);
        match leading {
            Leading::None => {}
            Leading::Check => {
                content = content.push(
                    canvas(Check {
                        color: fg_color,
                        stroke: t.metrics.check_stroke,
                    })
                    .width(t.metrics.check_w)
                    .height(t.metrics.check_h),
                );
            }
            Leading::Box(checked) => {
                content = content.spacing(t.metrics.checkbox_gap).push(
                    canvas(CheckBox {
                        checked,
                        fill: faded(t.palette.text, alpha),
                        mark: color(t.palette.surface),
                        border: faded(t.palette.border2, alpha),
                        border_width: t.stroke.checkbox,
                        radius: t.radii.checkbox,
                        mark_stroke: t.metrics.check_stroke,
                    })
                    .width(t.controls.checkbox)
                    .height(t.controls.checkbox),
                );
            }
        }
        content = content.push(
            text(label.clone())
                .size(t.type_.button)
                .font(if strong { bold() } else { Font::DEFAULT })
                .color(fg_color),
        );
        let c = t.palette;
        let (bg, border) = match look {
            Look::Primary => {
                let fill = if hovered { c.accent_hover } else { c.accent };
                (Some(faded(fill, alpha)), Border::default())
            }
            Look::Selected => (
                Some(color(c.control_checked)),
                Border::default()
                    .width(t.stroke.selected)
                    .color(color(c.sel_border)),
            ),
            Look::Secondary | Look::Disabled => (
                hovered.then(|| color(c.control_hover)),
                Border::default()
                    .width(t.stroke.control)
                    .color(faded(c.border2, alpha)),
            ),
        };
        container(content)
            .height(Length::Fixed(t.controls.button_h))
            .padding(Padding::from([0.0, t.controls.button_px]))
            .align_y(Alignment::Center)
            .style(move |_| container::Style {
                background: bg.map(Background::Color),
                border: border.rounded(t.radii.button),
                shadow: shadow(&t),
                ..container::Style::default()
            })
            .into()
    };
    let shown: Element<'a, Action> = if enabled {
        hover(visual(false), visual(true))
    } else {
        visual(false)
    };
    hit_area(&t, shown, None, t.controls.button_h, on_press)
}

fn buttons_row<'a>(t: &Skin, items: Vec<Element<'a, Action>>) -> Element<'a, Action> {
    row(items)
        .spacing(t.controls.button_gap)
        .wrap()
        // Keep the visual row gap at `button_gap`; hit-area padding already
        // counts toward it.
        .vertical_spacing(
            (t.controls.button_gap - (t.controls.touch_target - t.controls.button_h).max(0.0))
                .max(0.0),
        )
        .into()
}

fn frame<'a>(t: &Skin, content: Element<'a, Action>) -> Element<'a, Action> {
    container(content)
        .padding(card_padding(t))
        .width(Fill)
        .style(card_style(*t))
        .into()
}

// ---------- Cards ----------

/// What the connected backend can do, so the view never draws a button
/// whose endpoint does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    /// Draft 发送 / 修改. Drafts are phase 2: only the fake source has them.
    pub draft_actions: bool,
}

impl Caps {
    /// Real host (phase 1): no draft buttons.
    pub const HOST: Caps = Caps {
        draft_actions: false,
    };
    /// `--fake` / `--gallery`: every card fully clickable.
    pub const FAKE: Caps = Caps {
        draft_actions: true,
    };
}

/// One new card. `width` is the stream's content width (for clamping the
/// draft preview); `ui` is that card's local state.
pub fn card<'a>(
    t: &Skin,
    card: &'a Card,
    ui: &CardUi,
    caps: Caps,
    width: f32,
) -> Element<'a, Action> {
    match card {
        Card::Choice(c) => choice(t, c, ui),
        Card::Approval(c) => approval(t, c, ui, width),
        Card::Draft(c) => draft(t, c, ui, caps, width),
    }
}

pub fn choice<'a>(t: &Skin, card: &'a ChoiceCard, ui: &CardUi) -> Element<'a, Action> {
    let (label, d) = status::choice_status(&card.state);
    let buttons = interact::choice_buttons(card, ui);
    let confirm = interact::confirm_button(card, ui);
    debug_assert!(interact::primary_count(&buttons, confirm.as_ref()) <= 1);

    let items = buttons
        .into_iter()
        .map(|b: OptionButton| {
            let leading = match (b.checkbox, b.look) {
                (Some(checked), _) => Leading::Box(checked),
                (None, Look::Selected) => Leading::Check,
                _ => Leading::None,
            };
            let action = b.clickable.then(|| {
                if card.multi {
                    Action::TogglePick {
                        card: card.id.clone(),
                        option: b.id.clone(),
                    }
                } else {
                    Action::Choose {
                        card: card.id.clone(),
                        option: b.id.clone(),
                    }
                }
            });
            btn(t, b.label, b.look, leading, action)
        })
        .collect();

    let mut col = column![
        header(t, &card.id, &card.bot, ui, label, d),
        gap(t.metrics.head_gap),
        body(t, &card.prompt),
        gap(t.metrics.body_gap),
        buttons_row(t, items),
    ];
    if let Some((confirm_label, enabled)) = confirm {
        col = col.push(gap(t.metrics.body_gap)).push(
            row![
                text(status::MULTI_HINT)
                    .size(t.type_.foot)
                    .color(color(t.palette.text3)),
                space::horizontal(),
                btn(
                    t,
                    confirm_label,
                    Look::Primary,
                    Leading::None,
                    enabled.then(|| Action::ConfirmChoice {
                        card: card.id.clone()
                    }),
                ),
            ]
            .align_y(Alignment::Center),
        );
    }
    let foot = ui
        .note
        .as_ref()
        .map(|n| n.text())
        .or_else(|| status::choice_footnote(card, &Local));
    if let Some(foot) = foot {
        col = col.push(gap(t.metrics.foot_gap)).push(footnote(t, foot));
    }
    frame(t, col.into())
}

pub fn approval<'a>(
    t: &Skin,
    card: &'a ApprovalCard,
    ui: &CardUi,
    width: f32,
) -> Element<'a, Action> {
    let inner = width - 2.0 * (t.controls.card_pad_h + t.stroke.card);
    let (label, d) = status::approval_status(&card.state);
    let mut col = column![
        header(t, &card.id, &card.bot, ui, label, d),
        gap(t.metrics.head_gap)
    ];
    let actions = interact::approval_actions(card, ui);
    if actions == ApprovalActions::TakeOver {
        // Stuck: one line and one secondary button. No approve/deny, no
        // code input, no terminal.
        col = col
            .push(body(t, status::NEEDS_OWNER_LINE))
            .push(gap(t.metrics.body_gap))
            .push(btn(
                t,
                status::TAKE_OVER.into(),
                Look::Secondary,
                Leading::None,
                Some(Action::TakeOver {
                    card: card.id.clone(),
                }),
            ));
    } else {
        col = col.push(
            row![
                tag(t, status::action_kind_label(card.kind), t.palette.text2),
                container(body(t, &card.summary)).width(Fill),
            ]
            .spacing(t.controls.button_gap)
            .align_y(Alignment::Center),
        );
        if let Some(path) = &card.path {
            col = col
                .push(gap(t.metrics.head_gap))
                .push(path_chip(t, path, inner));
        }
        if let ApprovalActions::Decide { enabled } = actions {
            let id = &card.id;
            col = col.push(gap(t.metrics.body_gap)).push(buttons_row(
                t,
                vec![
                    btn(
                        t,
                        "批准".into(),
                        Look::Primary,
                        Leading::None,
                        enabled.then(|| Action::Decide {
                            card: id.clone(),
                            approve: true,
                        }),
                    ),
                    btn(
                        t,
                        "拒绝".into(),
                        if enabled {
                            Look::Secondary
                        } else {
                            Look::Disabled
                        },
                        Leading::None,
                        enabled.then(|| Action::Decide {
                            card: id.clone(),
                            approve: false,
                        }),
                    ),
                ],
            ));
        }
    }
    let foot = ui
        .note
        .as_ref()
        .map(|n| n.text())
        .or_else(|| status::approval_footnote(&card.state, &Local));
    if let Some(foot) = foot {
        col = col.push(gap(t.metrics.foot_gap)).push(footnote(t, foot));
    }
    frame(t, col.into())
}

pub fn draft<'a>(
    t: &Skin,
    card: &'a DraftCard,
    ui: &CardUi,
    caps: Caps,
    width: f32,
) -> Element<'a, Action> {
    let (label, d) = status::draft_status(&card.state);
    let lines = status::draft_preview_lines(&card.state);
    let inner = width - 2.0 * (t.controls.card_pad_h + t.metrics.preview_pad + t.stroke.card);
    let cols = (inner / (t.type_.preview * t.type_.unit_width)).max(8.0) as usize;
    let preview_text = status::clamp_lines(&card.body, cols, lines);

    let mut preview = column![].spacing(4);
    if let Some(subject) = &card.subject {
        preview = preview.push(
            text(subject.as_str())
                .size(t.type_.draft_title)
                .font(bold())
                .color(color(t.palette.text)),
        );
    }
    let mut body_text = text(preview_text)
        .size(t.type_.preview)
        .line_height(LineHeight::Absolute(Pixels(t.type_.preview_line)))
        .color(color(t.palette.text2));
    if lines == 1 {
        body_text = body_text.wrapping(Wrapping::None);
    }
    preview = preview.push(body_text);
    let (bg, r) = (t.palette.sunken, t.radii.preview);
    let preview_box = container(preview)
        .padding(t.metrics.preview_pad)
        .width(Fill)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(color(bg).into()),
            border: Border::default().rounded(r),
            ..container::Style::default()
        });

    let mut col = column![
        header(t, &card.id, &card.bot, ui, label, d),
        gap(t.metrics.head_gap),
        row![
            tag(t, status::channel_label(card.channel), t.palette.text2),
            text("发给")
                .size(t.type_.foot)
                .color(color(t.palette.text2)),
            mono(t, &card.to),
        ]
        .spacing(t.metrics.dot_gap)
        .align_y(Alignment::Center)
        .wrap(),
        gap(t.metrics.head_gap),
        preview_box,
    ];
    if let Some(enabled) = interact::draft_actions(card, ui, caps.draft_actions) {
        let id = &card.id;
        col = col.push(gap(t.metrics.body_gap)).push(buttons_row(
            t,
            vec![
                btn(
                    t,
                    "发送".into(),
                    Look::Primary,
                    Leading::None,
                    enabled.then(|| Action::SendDraft { card: id.clone() }),
                ),
                btn(
                    t,
                    "修改".into(),
                    Look::Secondary,
                    Leading::None,
                    Some(Action::EditDraftInDm { card: id.clone() }),
                ),
            ],
        ));
    }
    let foot = ui
        .note
        .as_ref()
        .map(|n| n.text())
        .or_else(|| status::draft_footnote(&card.state, &Local));
    if let Some(foot) = foot {
        col = col.push(gap(t.metrics.foot_gap)).push(footnote(t, foot));
    }
    frame(t, col.into())
}

// ---------- Routines ----------

fn divider<'a, M: 'a>(t: &Skin, inset: f32) -> Element<'a, M> {
    let c = t.palette.divider;
    container(
        container(Space::new().width(Fill).height(t.stroke.divider)).style(move |_| {
            container::Style {
                background: Some(color(c).into()),
                ..container::Style::default()
            }
        }),
    )
    .padding(Padding::from([0.0, inset]))
    .width(Fill)
    .into()
}

/// The routines panel (SPEC §7). `now` is Unix seconds; times show in the
/// computer's local zone.
pub fn routines_panel<'a>(t: &Skin, routines: &'a Routines, now: i64) -> Element<'a, Action> {
    let m = t.metrics;
    let mut head = row![text(routines::TITLE)
        .size(t.type_.list_title)
        .font(bold())
        .color(color(t.palette.text))]
    .spacing(t.controls.button_gap)
    .align_y(Alignment::Center);
    if !routines.rows.is_empty() {
        head = head.push(
            text(routines.rows.len().to_string())
                .size(t.type_.list_count)
                .color(color(t.palette.text3)),
        );
    }
    let mut col = column![
        container(head)
            .height(Length::Fixed(m.panel_head_height))
            .padding(Padding::from([0.0, m.row_pad_h]))
            .align_y(Alignment::Center),
        divider(t, 0.0),
    ];

    if routines.rows.is_empty() {
        let rings = Rings {
            color: color(t.palette.text3),
            radius: m.empty_ring_r,
            stroke: m.empty_ring_stroke,
        };
        let size = rings.size();
        col = col.push(
            container(
                column![
                    canvas(rings).width(size.width).height(size.height),
                    gap(m.head_gap),
                    text(routines::EMPTY_TITLE)
                        .size(t.type_.empty_title)
                        .font(bold())
                        .color(color(t.palette.text2)),
                    gap(m.dot_gap),
                    text(routines::EMPTY_HINT)
                        .size(t.type_.empty_hint)
                        .color(color(t.palette.text3)),
                ]
                .align_x(Alignment::Center),
            )
            .width(Fill)
            .height(Length::Fixed(m.empty_height))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
        );
    } else {
        for (i, r) in routines.rows.iter().enumerate() {
            if i > 0 {
                col = col.push(divider(t, m.row_pad_h));
            }
            let name_color = if r.paused {
                t.palette.text3
            } else {
                t.palette.text
            };
            let mut right = row![].spacing(m.row_time_gap).align_y(Alignment::Center);
            if routines.failed.contains(&r.id) {
                right = right.push(
                    text(status::PAUSE_FAILED_NOTE)
                        .size(t.type_.foot)
                        .color(color(t.palette.text3)),
                );
            }
            right = right.push(match r.next_run_text(now) {
                Some(s) => {
                    Element::from(text(s).size(t.type_.row_time).color(color(t.palette.text2)))
                }
                None => tag(t, routines::PAUSED_TAG, t.palette.text3),
            });
            let toggle = hit_area(
                t,
                canvas(Toggle {
                    on: !r.paused,
                    track_on: color(t.palette.accent),
                    track_off: color(t.palette.toggle_off),
                    knob: color(t.palette.knob),
                    knob_size: t.controls.knob,
                })
                .width(t.controls.switch_w)
                .height(t.controls.switch_h)
                .into(),
                Some(t.controls.switch_w),
                t.controls.switch_h,
                Some(Action::TogglePause { id: r.id.clone() }),
            );
            right = right.push(toggle);
            col = col.push(
                row![
                    text(r.name.as_str())
                        .size(t.type_.row_name)
                        .color(color(name_color))
                        .wrapping(Wrapping::None)
                        .width(Fill),
                    right,
                ]
                .spacing(m.row_time_gap)
                .height(Length::Fixed(t.controls.row_h))
                .padding(Padding::from([0.0, m.row_pad_h]))
                .align_y(Alignment::Center),
            );
        }
    }

    let tt = *t;
    container(col)
        .width(Fill)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(color(tt.palette.surface).into()),
            text_color: Some(color(tt.palette.text)),
            border: Border::default()
                .width(tt.stroke.card)
                .color(color(tt.palette.border))
                .rounded(tt.radii.panel),
            shadow: shadow(&tt),
            ..container::Style::default()
        })
        .into()
}

// ---------- Conversation list ----------

/// Avatar: accent ring with the name's first letter (UI: logo ring + initial).
fn avatar<'a, M: 'a>(t: &Skin, name: &str) -> Element<'a, M> {
    let size = t.controls.touch_target.max(t.controls.button_h) + 4.0;
    let ring = t.palette.accent;
    container(
        text(conversations::initial(name))
            .size(t.type_.row_name)
            .font(bold())
            .color(color(ring)),
    )
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_| container::Style {
        border: Border::default()
            .width(2.0)
            .color(color(ring))
            .rounded(size / 2.0),
        ..container::Style::default()
    })
    .into()
}

/// Unread badge: accent pill with the count (no separate red token yet).
fn unread_badge<'a, M: 'a>(t: &Skin, label: String) -> Element<'a, M> {
    let (bg, fg) = (t.palette.accent, t.palette.on_accent);
    container(text(label).size(t.type_.tag).font(bold()).color(color(fg)))
        .padding(Padding::from([1.0, 6.0]))
        .style(move |_| container::Style {
            background: Some(color(bg).into()),
            border: Border::default().rounded(9.0),
            ..container::Style::default()
        })
        .into()
}

fn conversation_row<'a>(
    t: &Skin,
    c: &'a Conversation,
    selected: bool,
    now: i64,
) -> Element<'a, Action> {
    let time = c
        .last_at
        .as_deref()
        .map(|at| conversations::list_time(at, now, &Local))
        .unwrap_or_default();
    // Name and preview clip; time and badge keep their size.
    let top = row![
        container(
            text(c.name.as_str())
                .size(t.type_.row_name)
                .font(bold())
                .wrapping(Wrapping::None)
                .color(color(t.palette.text)),
        )
        .width(Fill)
        .clip(true),
        text(time)
            .size(t.type_.row_time)
            .wrapping(Wrapping::None)
            .color(color(t.palette.text3)),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let preview = c.preview.clone().unwrap_or_default();
    let mut bottom = row![container(
        text(preview)
            .size(t.type_.foot)
            .wrapping(Wrapping::None)
            .color(color(t.palette.text2))
    )
    .width(Fill)
    .clip(true)]
    .spacing(6)
    .align_y(Alignment::Center);
    if let Some(b) = conversations::badge(c.unread) {
        bottom = bottom.push(unread_badge(t, b));
    }
    let body = column![top, bottom].spacing(2).width(Fill).clip(true);
    button(
        row![avatar(t, &c.name), body]
            .spacing(10)
            .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([6.0, 8.0]))
    .style(nav_button_style(*t, selected))
    .on_press(Action::OpenChat {
        group: c.group_id.clone(),
    })
    .into()
}

/// The IM list: heading with 「＋ 新 bro」, then every group and DM, newest
/// first. Empty: 「还没有 bro，引入第一个」 and the same button.
pub fn conversation_list<'a>(
    t: &Skin,
    list: &'a Conversations,
    selected: Option<&str>,
    now: i64,
) -> Element<'a, Action> {
    let h = t.controls.touch_target.max(t.controls.button_h);
    let new_bro = || {
        button(text(conversations::NEW_BRO).size(t.type_.foot))
            .height(Length::Fixed(h))
            .padding(Padding::from([0.0, 10.0]))
            .style(nav_button_style(*t, false))
            .on_press(Action::NewBro)
    };
    let head = row![
        text(conversations::TITLE)
            .size(t.type_.list_title)
            .font(bold())
            .color(color(t.palette.text)),
        space::horizontal(),
        new_bro(),
    ]
    .align_y(Alignment::Center);
    let mut col = column![head].spacing(2);
    if list.is_empty() {
        col = col.push(gap(12.0)).push(
            text(conversations::EMPTY_HINT)
                .size(t.type_.empty_hint)
                .color(color(t.palette.text2)),
        );
        return col.into();
    }
    for c in list.rows() {
        col = col.push(conversation_row(
            t,
            c,
            selected == Some(c.group_id.as_str()),
            now,
        ));
    }
    col.into()
}
