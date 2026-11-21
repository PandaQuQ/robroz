//! iced widgets for the new cards and the routines panel.
//!
//! Every colour, radius, border, padding and size comes from [`Skin`].
//! Paths and addresses are plain monospace text, never links. Widgets only
//! emit [`Action`]s; the app decides what to call.

use chrono::Local;
use iced::widget::text::{LineHeight, Wrapping};
use iced::widget::{button, canvas, column, container, hover, row, space, text, text_input, Space};
use iced::{
    font, Alignment, Background, Border, Color, Element, Fill, Font, Length, Padding, Pixels,
    Shadow as IcedShadow, Theme as IcedTheme, Vector,
};

use crate::cards::confirm::{self, ConfirmCard};
use crate::cards::interact::{self, ApprovalActions, CardUi, Look, OptionButton};
use crate::cards::memories::{self, Book};
use crate::cards::routines::{self, Routines};
use crate::cards::skills::{self, Book as SkillBook};
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
    /// 「引入一个 bro」: create a bro and open its DM.
    NewBro,
    /// Narrow screens: back from a chat to the list.
    BackToList,
    /// Composer text changed.
    ComposerEdit {
        text: String,
    },
    /// Enter or 发送 in the composer.
    ComposerSend,
    /// Bro DM header: click the name to rename the bro.
    RenameStart,
    RenameEdit {
        text: String,
    },
    /// Enter in the rename box.
    RenameSave,
    /// Esc or 取消.
    RenameCancel,
    /// 记忆 page: open or close the 记一条 form.
    MemoryAddOpen,
    MemoryAddClose,
    MemoryAddText {
        text: String,
    },
    MemoryAddScope {
        scope: String,
    },
    MemoryAddKind {
        kind: robroz_contract::MemoryKind,
    },
    MemoryAddSave,
    /// 改: edit this memory's text in place.
    MemoryEdit {
        id: String,
    },
    MemoryEditText {
        text: String,
    },
    MemoryEditSave,
    MemoryEditCancel,
    /// 忘掉 once asks; the second 忘掉 deletes.
    MemoryForgetAsk {
        id: String,
    },
    MemoryForgetConfirm,
    MemoryForgetCancel,
    /// 记忆 page, pending entry: 记下 (`keep`) or 不用, through its confirm card.
    MemoryDecide {
        id: String,
        keep: bool,
    },
    /// Skill confirm card: open or close the full instruction.
    ToggleInstruction {
        card: String,
    },
    /// 「还有 N 条待确认」: show every pending confirm card of this task.
    UnfoldConfirms {
        task: String,
    },
    /// 技能 page: open or close the 加一项 form.
    SkillAddOpen,
    SkillAddClose,
    SkillAddSlug {
        text: String,
    },
    SkillAddTitle {
        text: String,
    },
    SkillAddInstruction {
        text: String,
    },
    SkillAddScope {
        scope: String,
    },
    SkillAddSave,
    /// 改: edit this skill's title and instruction in place.
    SkillEdit {
        slug: String,
    },
    SkillEditTitle {
        text: String,
    },
    SkillEditInstruction {
        text: String,
    },
    SkillEditSave,
    SkillEditCancel,
    /// 删除 once asks; the second 删除 forgets the skill.
    SkillDeleteAsk {
        slug: String,
    },
    SkillDeleteConfirm,
    SkillDeleteCancel,
    /// 技能 page, pending entry: 保存 (`keep`) or 不用, through its confirm card.
    SkillDecide {
        slug: String,
        keep: bool,
    },
    /// Open or close a long instruction on the 技能 page.
    SkillToggleInstruction {
        slug: String,
    },
    /// Enable or disable a kept skill in this group.
    SkillToggleGroup {
        slug: String,
        group: String,
    },
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
        Card::Confirm(c) => confirm_card(t, c, ui, width),
    }
}

/// A bro's proposal: tag 「记忆」/「技能」, the full text with who can use
/// it, then 记下 (保存) / 不用 until answered; the answer goes to the footnote.
pub fn confirm_card<'a>(
    t: &Skin,
    card: &'a ConfirmCard,
    ui: &CardUi,
    width: f32,
) -> Element<'a, Action> {
    let (label, d) = card.status();
    let mut col = column![
        header(t, &card.id, &card.bot, ui, label, d),
        gap(t.metrics.head_gap)
    ];
    let tag_el = tag(t, card.tag(), t.palette.text2);
    match &card.title {
        // Skill: title + slug, instruction cut to three lines until opened.
        Some(title) => {
            let mut head = row![
                tag_el,
                text(title.as_str())
                    .size(t.type_.body)
                    .font(bold())
                    .color(color(t.palette.text)),
            ]
            .spacing(t.controls.button_gap)
            .align_y(Alignment::Center);
            if let Some(slug) = &card.slug {
                head = head.push(mono(t, slug));
            }
            col = col.push(head);
            if let Some(instruction) = &card.instruction {
                let inner = width - 2.0 * (t.controls.card_pad_h + t.stroke.card);
                let cols = (inner / (t.type_.body * t.type_.unit_width)).max(8.0) as usize;
                let cut = status::clamp_lines(instruction, cols, confirm::INSTRUCTION_LINES);
                let long = cut.ends_with('…');
                let shown = if ui.more_open || !long {
                    instruction.clone()
                } else {
                    cut
                };
                col = col.push(gap(t.metrics.head_gap)).push(
                    text(shown)
                        .size(t.type_.body)
                        .line_height(LineHeight::Absolute(Pixels(t.type_.body_line)))
                        .color(color(t.palette.text))
                        .width(Fill),
                );
                if long {
                    col = col.push(btn(
                        t,
                        if ui.more_open {
                            confirm::SHOW_LESS
                        } else {
                            confirm::SHOW_ALL
                        }
                        .into(),
                        Look::Secondary,
                        Leading::None,
                        Some(Action::ToggleInstruction {
                            card: card.id.clone(),
                        }),
                    ));
                }
            }
        }
        // Memory: the whole proposed text, then who can use it.
        None => {
            col = col.push(
                row![
                    tag_el,
                    container(body(t, card.text.as_deref().unwrap_or_default())).width(Fill),
                ]
                .spacing(t.controls.button_gap)
                .align_y(Alignment::Center),
            );
        }
    }
    if let Some(scope) = &card.scope_line {
        col = col.push(gap(t.metrics.head_gap)).push(
            text(scope.as_str())
                .size(t.type_.foot)
                .color(color(t.palette.text2)),
        );
    }
    if let Some(enabled) = confirm::confirm_actions(card, ui) {
        let id = &card.id;
        col = col.push(gap(t.metrics.body_gap)).push(buttons_row(
            t,
            vec![
                btn(
                    t,
                    card.keep_label().into(),
                    Look::Primary,
                    Leading::None,
                    enabled.then(|| Action::Decide {
                        card: id.clone(),
                        approve: true,
                    }),
                ),
                btn(
                    t,
                    confirm::DROP.into(),
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
    let foot = ui
        .note
        .as_ref()
        .map(|n| n.text())
        .or_else(|| card.footnote_in(&Local));
    if let Some(foot) = foot {
        col = col.push(gap(t.metrics.foot_gap)).push(footnote(t, foot));
    }
    frame(t, col.into())
}

/// 「还有 N 条待确认」 in place of a task's folded confirm cards.
pub fn confirm_more<'a>(t: &Skin, task: &str, n: usize) -> Element<'a, Action> {
    btn(
        t,
        confirm::more_label(n),
        Look::Secondary,
        Leading::None,
        Some(Action::UnfoldConfirms {
            task: task.to_string(),
        }),
    )
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

/// Full-width hairline (chat header / composer edges).
pub fn rule<'a, M: 'a>(t: &Skin) -> Element<'a, M> {
    divider(t, 0.0)
}

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

// ---------- 记忆 page ----------

/// A failed save, shown where it happened: accent text, never silent.
fn inline_note<'a, M: 'a>(t: &Skin, note: &str) -> Element<'a, M> {
    text(note.to_string())
        .size(t.type_.foot)
        .color(color(t.palette.accent))
        .into()
}

/// The 记忆 page: one panel like the routines list. Groups by who may use a
/// memory (「所有 bro」 first, then each bro); pending apart from kept.
pub fn memories_panel<'a>(t: &Skin, book: &'a Book, now: i64) -> Element<'a, Action> {
    use robroz_contract::MemoryKind;
    let m = t.metrics;
    let busy = book.busy;
    let mut head = row![text(memories::TITLE)
        .size(t.type_.list_title)
        .font(bold())
        .color(color(t.palette.text))]
    .spacing(t.controls.button_gap)
    .align_y(Alignment::Center);
    if !book.memories.is_empty() {
        head = head.push(
            text(book.memories.len().to_string())
                .size(t.type_.list_count)
                .color(color(t.palette.text3)),
        );
    }
    head = head.push(space::horizontal());
    if book.adding.is_none() {
        head = head.push(btn(
            t,
            memories::ADD.into(),
            Look::Secondary,
            Leading::None,
            (!busy).then_some(Action::MemoryAddOpen),
        ));
    }
    let mut col = column![
        container(head)
            .height(Length::Fixed(m.panel_head_height))
            .padding(Padding::from([0.0, m.row_pad_h]))
            .align_y(Alignment::Center),
        divider(t, 0.0),
    ];
    let pad = Padding::from([m.row_pad_h * 0.75, m.row_pad_h]);

    if let Some(note) = &book.note {
        col = col.push(
            container(
                text(note.as_str())
                    .size(t.type_.foot)
                    .color(color(t.palette.text2)),
            )
            .padding(Padding::from([8.0, m.row_pad_h])),
        );
    }

    if let Some(new) = &book.adding {
        let mut input = text_input(memories::NEW_PLACEHOLDER, &new.text)
            .size(t.type_.body)
            .padding(Padding::from([8.0, 10.0]));
        if !busy {
            input = input
                .on_input(|text| Action::MemoryAddText { text })
                .on_submit(Action::MemoryAddSave);
        }
        let mut scopes = row![].spacing(t.controls.button_gap);
        for (scope, label) in book.scope_choices() {
            let look = if new.scope.as_deref() == Some(scope.as_str()) {
                Look::Selected
            } else {
                Look::Secondary
            };
            scopes = scopes.push(btn(
                t,
                label,
                look,
                Leading::None,
                Some(Action::MemoryAddScope { scope }),
            ));
        }
        let mut kinds = row![].spacing(t.controls.button_gap);
        for kind in MemoryKind::ALL {
            let look = if kind == new.kind {
                Look::Selected
            } else {
                Look::Secondary
            };
            kinds = kinds.push(btn(
                t,
                memories::kind_label(kind).into(),
                look,
                Leading::None,
                Some(Action::MemoryAddKind { kind }),
            ));
        }
        let can_save = book.can_add();
        let mut form = column![input, scopes.wrap()].spacing(m.head_gap);
        // One grey row, always there: 「选一下谁能用这条」 until a scope is
        // picked, then 「只给 Bob 用」. Keeps 保存 from moving.
        if let Some(hint) = book.add_scope_hint() {
            form = form.push(text(hint).size(t.type_.foot).color(color(t.palette.text3)));
        }
        if let Some(note) = &book.add_note {
            form = form.push(inline_note(t, note));
        }
        col = col.push(
            container(
                column![
                    form,
                    kinds,
                    row![
                        btn(
                            t,
                            memories::SAVE.into(),
                            Look::Primary,
                            Leading::None,
                            can_save.then_some(Action::MemoryAddSave)
                        ),
                        btn(
                            t,
                            memories::CANCEL.into(),
                            Look::Secondary,
                            Leading::None,
                            (!busy).then_some(Action::MemoryAddClose)
                        ),
                    ]
                    .spacing(t.controls.button_gap),
                ]
                .spacing(m.head_gap),
            )
            .padding(pad),
        );
        col = col.push(divider(t, 0.0));
    }

    if book.loaded && book.is_empty() {
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
                    text(memories::EMPTY_TITLE)
                        .size(t.type_.empty_title)
                        .font(bold())
                        .color(color(t.palette.text2)),
                    gap(m.dot_gap),
                    text(memories::EMPTY_HINT)
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
    }

    let entry = |mem: &'a robroz_contract::Memory, pending: bool| -> Element<'a, Action> {
        let mut top = row![tag(t, memories::kind_label(mem.kind), t.palette.text2)]
            .spacing(m.row_time_gap)
            .align_y(Alignment::Center);
        if pending {
            top = top.push(tag(t, memories::PENDING_TAG, t.palette.accent));
        }
        top = top.push(space::horizontal());
        if book.just_saved.as_deref() == Some(mem.id.as_str()) {
            top = top.push(
                text(memories::SAVED)
                    .size(t.type_.row_time)
                    .color(color(t.palette.text2)),
            );
        }
        top = top.push(
            text(memories::updated_text(&mem.updated_at, now))
                .size(t.type_.row_time)
                .color(color(t.palette.text3)),
        );
        let editing = book.editing.as_ref().filter(|(id, _)| *id == mem.id);
        let asking = book.confirm_forget.as_deref() == Some(mem.id.as_str());
        let body: Element<'a, Action> = match editing {
            Some((_, draft)) => {
                let mut input = text_input("", draft)
                    .size(t.type_.body)
                    .padding(Padding::from([6.0, 8.0]));
                if !busy {
                    input = input
                        .on_input(|text| Action::MemoryEditText { text })
                        .on_submit(Action::MemoryEditSave);
                }
                input.into()
            }
            None => text(mem.text.as_str())
                .size(t.type_.body)
                .line_height(LineHeight::Absolute(Pixels(t.type_.body_line)))
                .color(color(t.palette.text))
                .width(Fill)
                .into(),
        };
        let actions: Element<'a, Action> = if pending {
            // 记下 / 不用 answer the entry's confirm card (robroz#66); an entry
            // without one (older host) stays display only. No 改/忘掉 here.
            match &mem.confirm_id {
                Some(_) => row![
                    btn(
                        t,
                        confirm::KEEP.into(),
                        Look::Primary,
                        Leading::None,
                        (!busy).then(|| Action::MemoryDecide {
                            id: mem.id.clone(),
                            keep: true,
                        })
                    ),
                    btn(
                        t,
                        confirm::DROP.into(),
                        if busy {
                            Look::Disabled
                        } else {
                            Look::Secondary
                        },
                        Leading::None,
                        (!busy).then(|| Action::MemoryDecide {
                            id: mem.id.clone(),
                            keep: false,
                        })
                    ),
                ]
                .spacing(t.controls.button_gap)
                .into(),
                None => Space::new().height(Length::Fixed(0.0)).into(),
            }
        } else if editing.is_some() {
            row![
                btn(
                    t,
                    memories::SAVE.into(),
                    Look::Primary,
                    Leading::None,
                    book.can_save_edit().then_some(Action::MemoryEditSave)
                ),
                btn(
                    t,
                    memories::CANCEL.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::MemoryEditCancel)
                ),
            ]
            .spacing(t.controls.button_gap)
            .into()
        } else if asking {
            row![
                text(memories::FORGET_ASK)
                    .size(t.type_.foot)
                    .color(color(t.palette.text2)),
                space::horizontal(),
                btn(
                    t,
                    memories::FORGET.into(),
                    Look::Primary,
                    Leading::None,
                    (!busy).then_some(Action::MemoryForgetConfirm)
                ),
                btn(
                    t,
                    memories::CANCEL.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::MemoryForgetCancel)
                ),
            ]
            .spacing(t.controls.button_gap)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                btn(
                    t,
                    memories::EDIT.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::MemoryEdit { id: mem.id.clone() })
                ),
                btn(
                    t,
                    memories::FORGET.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::MemoryForgetAsk { id: mem.id.clone() })
                ),
            ]
            .spacing(t.controls.button_gap)
            .into()
        };
        let mut entry_col = column![top, body, actions].spacing(m.dot_gap + 2.0);
        if let Some(note) = book.note_for(&mem.id) {
            entry_col = entry_col.push(inline_note(t, note));
        }
        container(entry_col).padding(pad).width(Fill).into()
    };

    for (gi, group) in book.groups().into_iter().enumerate() {
        if gi > 0 {
            col = col.push(divider(t, 0.0));
        }
        col = col.push(
            container(
                text(group.title.clone())
                    .size(t.type_.row_name)
                    .font(bold())
                    .color(color(t.palette.text)),
            )
            .padding(Padding::from([m.row_pad_h * 0.75, m.row_pad_h])),
        );
        let sub = |label: &'static str, n: usize| -> Element<'a, Action> {
            container(
                text(format!("{label} {n}"))
                    .size(t.type_.foot)
                    .color(color(t.palette.text3)),
            )
            .padding(Padding::from([0.0, m.row_pad_h]))
            .into()
        };
        // 「待确认 N」 then 「已记下 N」; the kept head shows even when the
        // group has nothing pending, so every scope group reads the same.
        for (label, n) in group.heads() {
            if label == memories::KEPT_HEAD {
                if !group.pending.is_empty() {
                    col = col.push(divider(t, m.row_pad_h));
                    col = col.push(gap(m.dot_gap));
                }
                col = col.push(sub(label, n));
                break;
            }
            col = col.push(sub(label, n));
            for (i, mem) in group.pending.iter().enumerate() {
                if i > 0 {
                    col = col.push(divider(t, m.row_pad_h));
                }
                col = col.push(entry(mem, true));
            }
        }
        for (i, mem) in group.kept.iter().enumerate() {
            if i > 0 {
                col = col.push(divider(t, m.row_pad_h));
            }
            col = col.push(entry(mem, false));
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

// ---------- 技能 page ----------

/// The 技能 page: same grouping as the 记忆 page, plus a 3-line
/// instruction, edit of title and instruction, a confirm before 删除,
/// and a per-group enable toggle on kept entries.
pub fn skills_panel<'a>(
    t: &Skin,
    book: &'a SkillBook,
    now: i64,
    width: f32,
) -> Element<'a, Action> {
    let m = t.metrics;
    let busy = book.busy;
    let mut head = row![text(skills::TITLE)
        .size(t.type_.list_title)
        .font(bold())
        .color(color(t.palette.text))]
    .spacing(t.controls.button_gap)
    .align_y(Alignment::Center);
    if !book.skills.is_empty() {
        head = head.push(
            text(book.skills.len().to_string())
                .size(t.type_.list_count)
                .color(color(t.palette.text3)),
        );
    }
    head = head.push(space::horizontal());
    if book.adding.is_none() {
        head = head.push(btn(
            t,
            skills::ADD.into(),
            Look::Secondary,
            Leading::None,
            (!busy).then_some(Action::SkillAddOpen),
        ));
    }
    let mut col = column![
        container(head)
            .height(Length::Fixed(m.panel_head_height))
            .padding(Padding::from([0.0, m.row_pad_h]))
            .align_y(Alignment::Center),
        divider(t, 0.0),
    ];
    let pad = Padding::from([m.row_pad_h * 0.75, m.row_pad_h]);

    if let Some(note) = &book.note {
        col = col.push(
            container(
                text(note.as_str())
                    .size(t.type_.foot)
                    .color(color(t.palette.text2)),
            )
            .padding(Padding::from([8.0, m.row_pad_h])),
        );
    }

    if let Some(new) = &book.adding {
        let mut slug = text_input(skills::SLUG_PLACEHOLDER, &new.slug)
            .size(t.type_.body)
            .padding(Padding::from([8.0, 10.0]));
        let mut title = text_input(skills::TITLE_PLACEHOLDER, &new.title)
            .size(t.type_.body)
            .padding(Padding::from([8.0, 10.0]));
        let mut instruction = text_input(skills::INSTRUCTION_PLACEHOLDER, &new.instruction)
            .size(t.type_.body)
            .padding(Padding::from([8.0, 10.0]));
        if !busy {
            slug = slug.on_input(|text| Action::SkillAddSlug { text });
            title = title.on_input(|text| Action::SkillAddTitle { text });
            instruction = instruction
                .on_input(|text| Action::SkillAddInstruction { text })
                .on_submit(Action::SkillAddSave);
        }
        let mut scopes = row![].spacing(t.controls.button_gap);
        for (scope, label) in book.scope_choices() {
            let look = if new.scope.as_deref() == Some(scope.as_str()) {
                Look::Selected
            } else {
                Look::Secondary
            };
            scopes = scopes.push(btn(
                t,
                label,
                look,
                Leading::None,
                Some(Action::SkillAddScope { scope }),
            ));
        }
        let can_save = book.can_add();
        let mut form = column![slug, title, instruction, scopes.wrap()].spacing(m.head_gap);
        if let Some(hint) = book.add_scope_hint() {
            form = form.push(text(hint).size(t.type_.foot).color(color(t.palette.text3)));
        }
        if let Some(note) = &book.add_note {
            form = form.push(inline_note(t, note));
        }
        col = col.push(
            container(
                column![
                    form,
                    row![
                        btn(
                            t,
                            skills::SAVE.into(),
                            Look::Primary,
                            Leading::None,
                            can_save.then_some(Action::SkillAddSave)
                        ),
                        btn(
                            t,
                            skills::CANCEL.into(),
                            Look::Secondary,
                            Leading::None,
                            (!busy).then_some(Action::SkillAddClose)
                        ),
                    ]
                    .spacing(t.controls.button_gap),
                ]
                .spacing(m.head_gap),
            )
            .padding(pad),
        );
        col = col.push(divider(t, 0.0));
    }

    if book.loaded && book.is_empty() {
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
                    text(skills::EMPTY_TITLE)
                        .size(t.type_.empty_title)
                        .font(bold())
                        .color(color(t.palette.text2)),
                    gap(m.dot_gap),
                    text(skills::EMPTY_HINT)
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
    }

    let inner = (width - 2.0 * (t.controls.card_pad_h + t.stroke.card)).max(80.0);
    let cols = (inner / (t.type_.body * t.type_.unit_width)).max(8.0) as usize;

    let entry = |skill: &'a robroz_contract::Skill, pending: bool| -> Element<'a, Action> {
        let mut top = row![
            text(skill.title.as_str())
                .size(t.type_.body)
                .font(bold())
                .color(color(t.palette.text)),
            mono(t, &skill.slug),
        ]
        .spacing(m.row_time_gap)
        .align_y(Alignment::Center);
        if pending {
            top = top.push(tag(t, skills::PENDING_TAG, t.palette.accent));
        }
        top = top.push(space::horizontal());
        if book.just_saved.as_deref() == Some(skill.slug.as_str()) {
            top = top.push(
                text(skills::SAVED)
                    .size(t.type_.row_time)
                    .color(color(t.palette.text2)),
            );
        }
        top = top.push(
            text(skills::updated_text(&skill.updated_at, now))
                .size(t.type_.row_time)
                .color(color(t.palette.text3)),
        );
        let editing = book.editing.as_ref().filter(|e| e.slug == skill.slug);
        let asking = book.confirm_delete.as_deref() == Some(skill.slug.as_str());
        let body: Element<'a, Action> = match editing {
            Some(edit) => {
                let mut title = text_input(skills::TITLE_PLACEHOLDER, &edit.title)
                    .size(t.type_.body)
                    .padding(Padding::from([6.0, 8.0]));
                let mut instruction =
                    text_input(skills::INSTRUCTION_PLACEHOLDER, &edit.instruction)
                        .size(t.type_.body)
                        .padding(Padding::from([6.0, 8.0]));
                if !busy {
                    title = title.on_input(|text| Action::SkillEditTitle { text });
                    instruction = instruction
                        .on_input(|text| Action::SkillEditInstruction { text })
                        .on_submit(Action::SkillEditSave);
                }
                column![title, instruction].spacing(m.dot_gap).into()
            }
            None => {
                let cut = status::clamp_lines(&skill.instruction, cols, skills::INSTRUCTION_LINES);
                let long = cut.ends_with('…');
                let open = book.open.contains(&skill.slug);
                let shown = if open || !long {
                    skill.instruction.clone()
                } else {
                    cut
                };
                let mut body = column![text(shown)
                    .size(t.type_.body)
                    .line_height(LineHeight::Absolute(Pixels(t.type_.body_line)))
                    .color(color(t.palette.text))
                    .width(Fill)]
                .spacing(m.dot_gap);
                if long {
                    body = body.push(btn(
                        t,
                        if open {
                            confirm::SHOW_LESS
                        } else {
                            confirm::SHOW_ALL
                        }
                        .into(),
                        Look::Secondary,
                        Leading::None,
                        Some(Action::SkillToggleInstruction {
                            slug: skill.slug.clone(),
                        }),
                    ));
                }
                body.into()
            }
        };
        let actions: Element<'a, Action> = if pending {
            match &skill.confirm_id {
                Some(_) => row![
                    btn(
                        t,
                        confirm::SAVE.into(),
                        Look::Primary,
                        Leading::None,
                        (!busy).then(|| Action::SkillDecide {
                            slug: skill.slug.clone(),
                            keep: true,
                        })
                    ),
                    btn(
                        t,
                        confirm::DROP.into(),
                        if busy {
                            Look::Disabled
                        } else {
                            Look::Secondary
                        },
                        Leading::None,
                        (!busy).then(|| Action::SkillDecide {
                            slug: skill.slug.clone(),
                            keep: false,
                        })
                    ),
                ]
                .spacing(t.controls.button_gap)
                .into(),
                None => Space::new().height(Length::Fixed(0.0)).into(),
            }
        } else if editing.is_some() {
            row![
                btn(
                    t,
                    skills::SAVE.into(),
                    Look::Primary,
                    Leading::None,
                    book.can_save_edit().then_some(Action::SkillEditSave)
                ),
                btn(
                    t,
                    skills::CANCEL.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::SkillEditCancel)
                ),
            ]
            .spacing(t.controls.button_gap)
            .into()
        } else if asking {
            row![
                text(skills::DELETE_ASK)
                    .size(t.type_.foot)
                    .color(color(t.palette.text2)),
                space::horizontal(),
                btn(
                    t,
                    skills::DELETE.into(),
                    Look::Primary,
                    Leading::None,
                    (!busy).then_some(Action::SkillDeleteConfirm)
                ),
                btn(
                    t,
                    skills::CANCEL.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::SkillDeleteCancel)
                ),
            ]
            .spacing(t.controls.button_gap)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                btn(
                    t,
                    skills::EDIT.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::SkillEdit {
                        slug: skill.slug.clone()
                    })
                ),
                btn(
                    t,
                    skills::DELETE.into(),
                    Look::Secondary,
                    Leading::None,
                    (!busy).then_some(Action::SkillDeleteAsk {
                        slug: skill.slug.clone()
                    })
                ),
            ]
            .spacing(t.controls.button_gap)
            .into()
        };
        let mut entry_col = column![top, body, actions].spacing(m.dot_gap + 2.0);
        if !pending && editing.is_none() && !asking && !book.chats.is_empty() {
            let mut toggles = row![text(skills::GROUP_ENABLE)
                .size(t.type_.foot)
                .color(color(t.palette.text3))]
            .spacing(t.controls.button_gap)
            .align_y(Alignment::Center);
            for (group, name) in &book.chats {
                let state = book.enable_state(group, &skill.slug);
                let known = state.known();
                let (track_on, track_off, knob) = if known.is_some() {
                    (
                        color(t.palette.accent),
                        color(t.palette.toggle_off),
                        color(t.palette.knob),
                    )
                } else {
                    let grey = color(t.palette.text3);
                    (grey, grey, color(t.palette.surface))
                };
                let toggle = hit_area(
                    t,
                    canvas(Toggle {
                        on: known.unwrap_or(false),
                        track_on,
                        track_off,
                        knob,
                        knob_size: t.controls.knob,
                    })
                    .width(t.controls.switch_w)
                    .height(t.controls.switch_h)
                    .into(),
                    Some(t.controls.switch_w),
                    t.controls.switch_h,
                    known.and_then(|_| {
                        (!busy).then(|| Action::SkillToggleGroup {
                            slug: skill.slug.clone(),
                            group: group.clone(),
                        })
                    }),
                );
                let mut cell = row![
                    text(name.as_str())
                        .size(t.type_.foot)
                        .color(color(t.palette.text2)),
                    toggle,
                ]
                .spacing(m.dot_gap)
                .align_y(Alignment::Center);
                if known.is_none() {
                    cell = cell.push(
                        text(skills::LOADING)
                            .size(t.type_.foot)
                            .color(color(t.palette.text3)),
                    );
                }
                toggles = toggles.push(cell);
            }
            entry_col = entry_col.push(toggles.wrap());
        }
        if let Some(note) = book.note_for(&skill.slug) {
            entry_col = entry_col.push(inline_note(t, note));
        }
        container(entry_col).padding(pad).width(Fill).into()
    };

    for (gi, group) in book.groups().into_iter().enumerate() {
        if gi > 0 {
            col = col.push(divider(t, 0.0));
        }
        col = col.push(
            container(
                text(group.title.clone())
                    .size(t.type_.row_name)
                    .font(bold())
                    .color(color(t.palette.text)),
            )
            .padding(Padding::from([m.row_pad_h * 0.75, m.row_pad_h])),
        );
        let sub = |label: &'static str, n: usize| -> Element<'a, Action> {
            container(
                text(format!("{label} {n}"))
                    .size(t.type_.foot)
                    .color(color(t.palette.text3)),
            )
            .padding(Padding::from([0.0, m.row_pad_h]))
            .into()
        };
        for (label, n) in group.heads() {
            if label == skills::KEPT_HEAD {
                if !group.pending.is_empty() {
                    col = col.push(divider(t, m.row_pad_h));
                    col = col.push(gap(m.dot_gap));
                }
                col = col.push(sub(label, n));
                break;
            }
            col = col.push(sub(label, n));
            for (i, skill) in group.pending.iter().enumerate() {
                if i > 0 {
                    col = col.push(divider(t, m.row_pad_h));
                }
                col = col.push(entry(skill, true));
            }
        }
        for (i, skill) in group.kept.iter().enumerate() {
            if i > 0 {
                col = col.push(divider(t, m.row_pad_h));
            }
            col = col.push(entry(skill, false));
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
    let size = avatar_size(t);
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

fn avatar_size(t: &Skin) -> f32 {
    t.controls.touch_target.max(t.controls.button_h) + 4.0
}

/// Unread badge width estimate, reserved beside the preview.
const BADGE_W: f32 = 30.0;
/// Time column width estimate (`10月7日`, `昨天`, `23:59`).
const TIME_W: f32 = 52.0;
/// Accent bar on the open chat's row (drawn in the row's left padding).
const SELECTED_BAR: f32 = 3.0;
const ROW_PAD_H: f32 = 8.0;
const ROW_GAP: f32 = 10.0;

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

/// A conversation row: the open chat sits on `surface` with a border, so
/// it stands out on the sidebar in both themes; hover uses `control_hover`.
pub fn conversation_row_style(
    t: Skin,
    selected: bool,
) -> impl Fn(&IcedTheme, button::Status) -> button::Style {
    move |_, status| {
        let (bg, border) = if selected {
            (
                Some(Background::Color(color(t.palette.surface))),
                Border::default()
                    .width(t.stroke.card)
                    .color(color(t.palette.border2))
                    .rounded(t.radii.row),
            )
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            (
                Some(Background::Color(color(t.palette.control_hover))),
                Border::default().rounded(t.radii.row),
            )
        } else {
            (None, Border::default().rounded(t.radii.row))
        };
        button::Style {
            background: bg,
            text_color: color(t.palette.text),
            border,
            ..button::Style::default()
        }
    }
}

/// Width units that fit in `px` at font size `size`.
fn units_in(t: &Skin, px: f32, size: f32) -> usize {
    (px / (size * t.type_.unit_width)).max(4.0) as usize
}

fn conversation_row<'a>(
    t: &Skin,
    c: &'a Conversation,
    selected: bool,
    now: i64,
    width: f32,
) -> Element<'a, Action> {
    let time = c
        .last_at
        .as_deref()
        .map(|at| conversations::list_time(at, now, &Local))
        .unwrap_or_default();
    let badge = conversations::badge(c.unread);
    // Text column: row minus padding, avatar and gap.
    let body_w = width - 2.0 * ROW_PAD_H - ROW_GAP - avatar_size(t);
    let name = conversations::fit(&c.name, units_in(t, body_w - TIME_W, t.type_.row_name));
    let preview_w = body_w - if badge.is_some() { BADGE_W } else { 0.0 };
    let preview = conversations::fit(
        c.preview.as_deref().unwrap_or_default(),
        units_in(t, preview_w, t.type_.foot),
    );
    let top = row![
        container(
            text(name)
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
    if let Some(b) = badge {
        bottom = bottom.push(unread_badge(t, b));
    }
    let body = column![top, bottom].spacing(2).width(Fill).clip(true);
    let content: Element<'a, Action> = container(
        row![avatar(t, &c.name), body]
            .spacing(ROW_GAP)
            .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding(Padding::from([6.0, ROW_PAD_H]))
    .into();
    // The open chat gets an accent bar drawn inside the left padding, so
    // it takes no layout space and every row's avatar lines up flush.
    let inner: Element<'a, Action> = if selected {
        let accent = t.palette.accent;
        let bar = container(Space::new())
            .width(Length::Fixed(SELECTED_BAR))
            .height(Length::Fixed(avatar_size(t) - 8.0))
            .style(move |_| container::Style {
                background: Some(color(accent).into()),
                border: Border::default().rounded(SELECTED_BAR / 2.0),
                ..container::Style::default()
            });
        iced::widget::stack![
            content,
            container(bar)
                .height(Fill)
                .align_y(Alignment::Center)
                .padding(Padding {
                    left: 2.0,
                    ..Padding::ZERO
                }),
        ]
        .into()
    } else {
        content
    };
    button(inner)
        .width(Fill)
        .padding(0)
        .style(conversation_row_style(*t, selected))
        .on_press(Action::OpenChat {
            group: c.group_id.clone(),
        })
        .into()
}

/// The IM list: heading with 「引入一个 bro」, then every group and DM,
/// newest first. `width` is the list's own width (for fitting previews).
/// Empty: 「还没有 bro，引入第一个」 and the same button.
pub fn conversation_list<'a>(
    t: &Skin,
    list: &'a Conversations,
    selected: Option<&str>,
    now: i64,
    width: f32,
) -> Element<'a, Action> {
    let h = t.controls.touch_target.max(t.controls.button_h);
    let new_bro = || {
        button(
            text(conversations::NEW_BRO)
                .size(t.type_.foot)
                .height(Fill)
                .align_y(Alignment::Center),
        )
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
            width,
        ));
    }
    col.into()
}

// ---------- Chat page ----------

/// Rename state for a bro DM's header; `None` in groups.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeaderRename<'a> {
    /// `Some(draft)` while editing.
    pub draft: Option<&'a str>,
    pub saving: bool,
    /// Last rename error.
    pub note: Option<&'a str>,
}

/// Widget id of the rename box, focused when editing starts.
pub const RENAME_INPUT: &str = "robroz-rename-bro";

/// Top of the chat page: the group or bro name (and `私聊` for a DM).
/// `back` adds 「‹」 for narrow screens, where the chat covers the list.
/// With `rename` (bro DMs) the name is a button that turns into an input:
/// Enter or 保存 saves (保存 is off while the name is empty or unchanged),
/// Esc or 取消 cancels.
pub fn chat_header<'a>(
    t: &Skin,
    name: &str,
    dm: bool,
    back: bool,
    rename: Option<HeaderRename<'_>>,
) -> Element<'a, Action> {
    let h = t.controls.touch_target.max(t.controls.button_h);
    let mut bar = row![].spacing(8).align_y(Alignment::Center);
    if back {
        bar = bar.push(
            button(
                text("‹ 聊天")
                    .size(t.type_.row_name)
                    .height(Fill)
                    .align_y(Alignment::Center),
            )
            .height(Length::Fixed(h))
            .padding(Padding::from([0.0, 8.0]))
            .style(nav_button_style(*t, false))
            .on_press(Action::BackToList),
        );
    }
    let title = || {
        text(name.to_string())
            .size(t.type_.list_title)
            .font(bold())
            .wrapping(Wrapping::None)
            .color(color(t.palette.text))
    };
    match rename {
        Some(HeaderRename {
            draft: Some(draft),
            saving,
            ..
        }) => {
            let mut input = text_input(conversations::RENAME_PLACEHOLDER, draft)
                .id(RENAME_INPUT)
                .size(t.type_.row_name)
                .padding(Padding::from([6.0, 8.0]));
            if !saving {
                input = input
                    .on_input(|text| Action::RenameEdit { text })
                    .on_submit(Action::RenameSave);
            }
            let changed = !draft.trim().is_empty() && draft.trim() != name.trim();
            bar = bar
                .push(container(input).width(Fill))
                .push(btn(
                    t,
                    "保存".into(),
                    Look::Primary,
                    Leading::None,
                    (!saving && changed).then_some(Action::RenameSave),
                ))
                .push(btn(
                    t,
                    "取消".into(),
                    Look::Secondary,
                    Leading::None,
                    (!saving).then_some(Action::RenameCancel),
                ));
        }
        Some(_) => {
            bar = bar.push(
                button(
                    row![
                        title(),
                        text(conversations::RENAME_HINT)
                            .size(t.type_.tag)
                            .color(color(t.palette.text3)),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding(Padding::from([4.0, 6.0]))
                .style(nav_button_style(*t, false))
                .on_press(Action::RenameStart),
            );
        }
        None => bar = bar.push(title()),
    }
    let editing = matches!(rename, Some(HeaderRename { draft: Some(_), .. }));
    if dm && !editing {
        bar = bar.push(tag(t, "私聊", t.palette.text2));
    }
    let mut col = column![container(bar)
        .width(Fill)
        .height(Length::Fixed(h + 12.0))
        .align_y(Alignment::Center)]
    .spacing(0);
    if let Some(note) = rename.and_then(|r| r.note) {
        col = col.push(container(footnote(t, note.to_string())).padding(Padding {
            bottom: 6.0,
            ..Padding::ZERO
        }));
    }
    container(col)
        .width(Fill)
        .padding(Padding::from([0.0, t.metrics.stream_padding]))
        .into()
}

/// Bottom of the chat page: one-line input and 发送; Enter sends.
/// `note` is the last send error, shown above the input.
pub fn composer<'a>(
    t: &Skin,
    draft: &str,
    hint: &str,
    sending: bool,
    note: Option<&str>,
) -> Element<'a, Action> {
    let h = t.controls.touch_target.max(t.controls.button_h);
    let can_send = !sending && !draft.trim().is_empty();
    let mut input = text_input(hint, draft)
        .size(t.type_.body)
        .padding(Padding::from([8.0, 10.0]));
    if !sending {
        input = input
            .on_input(|text| Action::ComposerEdit { text })
            .on_submit(Action::ComposerSend);
    }
    let send = btn(
        t,
        conversations::SEND.into(),
        Look::Primary,
        Leading::None,
        can_send.then_some(Action::ComposerSend),
    );
    let mut col = column![].spacing(4);
    if let Some(n) = note {
        col = col.push(footnote(t, n.to_string()));
    }
    col = col.push(
        row![
            container(input)
                .width(Fill)
                .height(Length::Fixed(h))
                .align_y(Alignment::Center),
            send
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );
    container(col)
        .width(Fill)
        .padding(Padding::from([8.0, t.metrics.stream_padding]))
        .into()
}
