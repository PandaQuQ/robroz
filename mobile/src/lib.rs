//! robroz mobile client spike (Slint). Connect screen, session list, one
//! status card, Settings (language, About with the Slint attribution).
//!
//! Strings come from the `robroz-ui` i18n keys (no Slint copy of their own);
//! the host talk and the status card rules are in [`api`] and [`card`].
//! Colours come from `robroz_tokens` via [`robroz_ui::theme::Skin`].

pub mod api;
pub mod card;
pub mod chat;

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use robroz_card::Palette as CardPalette;
use robroz_tokens::{ColorSet, DARK, LIGHT};
use robroz_ui::i18n::{self, Key, Locale};
use robroz_ui::theme::Mode;

slint::include_modules!();

fn color(rgb: [u8; 3]) -> slint::Color {
    slint::Color::from_rgb_u8(rgb[0], rgb[1], rgb[2])
}

fn palette_colors(p: CardPalette) -> (slint::Color, slint::Color) {
    (color(p.fill), color(p.text))
}

/// The token set for a mode. Used directly so bubble and chrome colours
/// stay on `robroz_tokens` (the desktop `Skin` still overwrites a few
/// fields with designer literals that have not landed in tokens.json).
fn tokens_for(mode: Mode) -> ColorSet {
    match mode {
        Mode::Dark => DARK,
        Mode::Light => LIGHT,
    }
}

/// Selected Language / Appearance segment: same rule as the desktop
/// client. Dark uses the border and text tokens; light uses
/// `control_checked` and text.
fn segment_selected(mode: Mode) -> ([u8; 3], [u8; 3]) {
    let p = tokens_for(mode);
    match mode {
        Mode::Dark => (p.border, p.text),
        Mode::Light => (p.control_checked, p.text),
    }
}

fn apply_theme(ui: &MainWindow, mode: Mode) {
    let p = tokens_for(mode);
    let brand = ui.global::<Brand>();
    brand.set_paper(color(p.window));
    brand.set_ink(color(p.text));
    brand.set_muted(color(p.text2));
    brand.set_rule(color(p.divider));
    brand.set_accent(color(p.accent));
    brand.set_on_accent(color(p.on_accent));
    brand.set_surface(color(p.surface));
    // Bubbles use the token palette (ink on light, inverted on dark for
    // own; surface + border for a bro). No hex literals in the UI.
    brand.set_own_bg(color(p.text));
    brand.set_own_text(color(p.window));
    brand.set_bro_bg(color(p.surface));
    brand.set_bro_border(color(p.border));
    let (fill, text) = segment_selected(mode);
    brand.set_segment_fill(color(fill));
    brand.set_segment_text(color(text));
    ui.set_theme_index(match mode {
        Mode::Light => 0,
        Mode::Dark => 1,
    });
}

fn apply_strings(ui: &MainWindow) {
    let tr = ui.global::<Tr>();
    tr.set_title_chats(Key::ConversationsTitle.as_str().into());
    tr.set_empty_chats(Key::ConversationsEmptyHint.as_str().into());
    tr.set_label_host(Key::ConnHostLabel.as_str().into());
    tr.set_label_token(Key::ConnTokenLabel.as_str().into());
    tr.set_connect(Key::ConnConnect.as_str().into());
    tr.set_connecting(Key::ConnConnecting.as_str().into());
    tr.set_show(Key::ConnShow.as_str().into());
    tr.set_hide(Key::ConnHide.as_str().into());
    tr.set_back(Key::ViewBackToChats.as_str().into());
    tr.set_settings(Key::HostSettingsBotName.as_str().into());
    tr.set_language(Key::SettingsLanguage.as_str().into());
    tr.set_locale_en(Key::LocaleNameEn.as_str().into());
    tr.set_locale_zh(Key::LocaleNameZh.as_str().into());
    tr.set_appearance(Key::SettingsAppearance.as_str().into());
    tr.set_theme_light(Key::ClientThemeLight.as_str().into());
    tr.set_theme_dark(Key::ClientThemeDark.as_str().into());
    tr.set_about(Key::MobileAbout.as_str().into());
    ui.set_locale_index(match i18n::locale() {
        Locale::En => 0,
        Locale::Zh => 1,
    });
    ui.set_version_text(
        Key::MobileVersion
            .fill(&[&env!("CARGO_PKG_VERSION")])
            .into(),
    );
}

fn apply_unencrypted_hint(ui: &MainWindow) {
    let hint = if api::is_insecure_remote(&ui.get_host_url()) {
        Key::ConnUnencryptedHint.as_str().to_string()
    } else {
        String::new()
    };
    ui.set_unencrypted_hint(hint.into());
}

fn apply_token_field(ui: &MainWindow, st: &State) {
    let masked = !st.token_dirty && st.token_saved.as_deref().is_some_and(|s| !s.is_empty());
    ui.set_token_masked(masked);
    if masked {
        ui.set_show_token(false);
        ui.set_token(api::TOKEN_MASK.into());
    } else if !st.token_dirty {
        ui.set_token("".into());
    }
}

fn chat_rows(items: &[chat::ChatItem], fallback_name: &str) -> Vec<ChatRow> {
    let bro = chat::bro_name(items, fallback_name);
    items
        .iter()
        .map(|item| match item {
            chat::ChatItem::Text { body, own } => ChatRow {
                kind: if *own { 0 } else { 1 },
                name: if *own { "".into() } else { bro.into() },
                text: body.as_str().into(),
                ..ChatRow::default()
            },
            chat::ChatItem::Status(c) => {
                let (fill, text_color) = palette_colors(c.palette);
                let shown = |s: &Option<String>| -> SharedString {
                    s.as_deref()
                        .map(|s| card::display(s).into_owned())
                        .unwrap_or_default()
                        .into()
                };
                ChatRow {
                    kind: 2,
                    name: c.bot_name.as_str().into(),
                    status: card::display(&c.status_line).into_owned().into(),
                    step: shown(&c.step_progress),
                    progress: shown(&c.progress),
                    fill,
                    text_color,
                    ..ChatRow::default()
                }
            }
        })
        .collect()
}

#[derive(Default)]
struct State {
    conn: Option<api::Conn>,
    chat: Vec<chat::ChatItem>,
    /// The open conversation's name, for the bubbles of a bro without a card.
    chat_name: String,
    /// Where Back from Settings returns to.
    before_settings: i32,
    theme: Mode,
    /// The last saved (or env) token. Never shown in the field.
    token_saved: Option<String>,
    token_dirty: bool,
}

/// Desktop-only shot hook: pre-fill screens from a fixture host when
/// `ROBROZ_SHOT_SCREEN` is set (`connect`, `chats`, `chat-running`,
/// `chat-failed`, `settings`, `connect-401`).
fn prepare_shot(ui: &MainWindow, state: &Arc<Mutex<State>>, shot: &str) {
    let shot = shot.trim();
    if shot == "connect" || shot == "connect-hint" {
        return;
    }
    let host = ui.get_host_url().to_string();
    let token = {
        let st = state.lock().unwrap();
        if shot == "connect-401" {
            "wrong-token".to_string()
        } else {
            api::token_for_connect(st.token_saved.as_deref(), &ui.get_token(), st.token_dirty)
        }
    };
    let conn = match api::Conn::new(&host, &token) {
        Ok(c) => c,
        Err(e) => {
            ui.set_error(e.0.into());
            return;
        }
    };
    let mut names: Vec<(String, String)> = Vec::new();
    match conn.groups() {
        Ok(groups) => {
            let unread = conn.unread().unwrap_or_default();
            let rows = api::session_rows(&groups, &unread);
            names = rows
                .iter()
                .map(|(g, _)| (g.id.clone(), g.name.clone()))
                .collect();
            ui.set_sessions(ModelRc::new(VecModel::from(
                rows.into_iter()
                    .map(|(g, n)| SessionRow {
                        id: g.id.into(),
                        name: g.name.into(),
                        unread: n.min(99) as i32,
                    })
                    .collect::<Vec<_>>(),
            )));
            let mut st = state.lock().unwrap();
            st.conn = Some(conn.clone());
            if !token.is_empty() && shot != "connect-401" {
                st.token_saved = Some(token);
                st.token_dirty = false;
                apply_token_field(ui, &st);
            }
        }
        Err(e) => {
            ui.set_error(e.0.into());
            if shot == "connect-401" {
                return;
            }
        }
    }
    let (group_id, screen) = match shot {
        "chats" => return ui.set_screen(1),
        "settings" => {
            state.lock().unwrap().before_settings = 1;
            return ui.set_screen(3);
        }
        "chat-running" => ("g1", 2),
        "chat-failed" => ("g2", 2),
        _ => return,
    };
    if let Some(conn) = state.lock().unwrap().conn.clone() {
        match conn.events(group_id) {
            Ok(events) => {
                let name = names
                    .into_iter()
                    .find(|(id, _)| id == group_id)
                    .map(|(_, n)| n)
                    .unwrap_or_else(|| group_id.to_string());
                let items = chat::build(&events);
                ui.set_chat_title(name.clone().into());
                ui.set_chat_note("".into());
                ui.set_chat(ModelRc::new(VecModel::from(chat_rows(&items, &name))));
                let mut st = state.lock().unwrap();
                st.chat = items;
                st.chat_name = name;
                ui.set_screen(screen);
            }
            Err(e) => ui.set_chat_note(e.0.into()),
        }
    }
}

/// Build the window and run until it closes.
pub fn run() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let state = Arc::new(Mutex::new(State::default()));

    // Desktop runs only; on a phone the environment is empty.
    if let Some(l) = std::env::var("ROBROZ_LANG")
        .ok()
        .and_then(|v| Locale::parse(&v))
    {
        i18n::set_locale(l);
    }
    let theme = Mode::from_env_value(std::env::var("ROBROZ_THEME").ok().as_deref());
    state.lock().unwrap().theme = theme;
    if let Ok(h) = std::env::var("ROBROZ_HOST") {
        ui.set_host_url(h.into());
    }
    if let Ok(t) = std::env::var("ROBROZ_TOKEN") {
        let trimmed = t.trim().to_string();
        if !trimmed.is_empty() {
            state.lock().unwrap().token_saved = Some(trimmed);
        }
    }
    if let Ok(v) = std::env::var("ROBROZ_SHOT_SAFE") {
        if v != "0" {
            // Typical iPhone inset, so desktop shots can show the safe-area
            // padding (the desktop backend reports 0).
            ui.set_extra_safe_top(47.0);
            ui.set_extra_safe_bottom(34.0);
        }
    }
    apply_theme(&ui, theme);
    apply_strings(&ui);
    apply_unencrypted_hint(&ui);
    apply_token_field(&ui, &state.lock().unwrap());

    if let Ok(shot) = std::env::var("ROBROZ_SHOT_SCREEN") {
        prepare_shot(&ui, &state, &shot);
    }

    ui.on_host_changed({
        let weak = ui.as_weak();
        move || {
            apply_unencrypted_hint(&weak.unwrap());
        }
    });

    ui.on_token_changed({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let ui = weak.unwrap();
            let mut st = state.lock().unwrap();
            let (draft, dirty) = api::edit_token(st.token_dirty, &ui.get_token());
            st.token_dirty = dirty;
            if dirty {
                ui.set_token_masked(false);
                if ui.get_token() != draft.as_str() && !draft.is_empty() {
                    ui.set_token(draft.into());
                }
            } else {
                apply_token_field(&ui, &st);
            }
        }
    });

    ui.on_toggle_show_token({
        let weak = ui.as_weak();
        move || {
            let ui = weak.unwrap();
            if ui.get_token_masked() {
                return;
            }
            ui.set_show_token(!ui.get_show_token());
        }
    });

    ui.on_connect({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let ui = weak.unwrap();
            if ui.get_busy() {
                return;
            }
            ui.set_error("".into());
            apply_unencrypted_hint(&ui);
            let (host, token) = {
                let st = state.lock().unwrap();
                (
                    ui.get_host_url().to_string(),
                    api::token_for_connect(
                        st.token_saved.as_deref(),
                        &ui.get_token(),
                        st.token_dirty,
                    ),
                )
            };
            let conn = match api::Conn::new(&host, &token) {
                Ok(c) => c,
                Err(e) => {
                    ui.set_error(e.0.into());
                    return;
                }
            };
            ui.set_busy(true);
            let weak = ui.as_weak();
            let state = state.clone();
            std::thread::spawn(move || {
                let result = conn.groups().and_then(|groups| {
                    // The unread counts only decorate the list.
                    let unread = conn.unread().unwrap_or_default();
                    Ok(api::session_rows(&groups, &unread))
                });
                let _ = slint::invoke_from_event_loop(move || {
                    let ui = weak.unwrap();
                    ui.set_busy(false);
                    match result {
                        Ok(rows) => {
                            let rows: Vec<SessionRow> = rows
                                .into_iter()
                                .map(|(g, n)| SessionRow {
                                    id: g.id.into(),
                                    name: g.name.into(),
                                    unread: n.min(99) as i32,
                                })
                                .collect();
                            ui.set_sessions(ModelRc::new(VecModel::from(rows)));
                            {
                                let mut st = state.lock().unwrap();
                                st.conn = Some(conn);
                                if !token.is_empty() {
                                    st.token_saved = Some(token);
                                    st.token_dirty = false;
                                    apply_token_field(&ui, &st);
                                }
                            }
                            ui.set_screen(1);
                        }
                        Err(e) => ui.set_error(e.0.into()),
                    }
                });
            });
        }
    });

    ui.on_open_session({
        let weak = ui.as_weak();
        let state = state.clone();
        move |id, name| {
            let Some(conn) = state.lock().unwrap().conn.clone() else {
                return;
            };
            let ui = weak.unwrap();
            ui.set_chat(ModelRc::new(VecModel::from(Vec::<ChatRow>::new())));
            ui.set_chat_title(name.clone());
            ui.set_chat_note("…".into());
            ui.set_screen(2);
            let weak = ui.as_weak();
            let state = state.clone();
            let id = id.to_string();
            let name = name.to_string();
            std::thread::spawn(move || {
                let result = conn.events(&id);
                let _ = slint::invoke_from_event_loop(move || {
                    let ui = weak.unwrap();
                    match result {
                        Ok(events) => {
                            let items = chat::build(&events);
                            ui.set_chat_note(if items.is_empty() {
                                Key::ConversationsEmptyHint.as_str().into()
                            } else {
                                "".into()
                            });
                            ui.set_chat(ModelRc::new(VecModel::from(chat_rows(&items, &name))));
                            let mut st = state.lock().unwrap();
                            st.chat = items;
                            st.chat_name = name;
                        }
                        Err(e) => ui.set_chat_note(e.0.into()),
                    }
                });
            });
        }
    });

    ui.on_back({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let ui = weak.unwrap();
            let to = if ui.get_screen() == 3 {
                state.lock().unwrap().before_settings
            } else {
                1
            };
            ui.set_screen(to);
        }
    });

    ui.on_open_settings({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let ui = weak.unwrap();
            state.lock().unwrap().before_settings = ui.get_screen();
            ui.set_screen(3);
        }
    });

    ui.on_set_locale({
        let weak = ui.as_weak();
        let state = state.clone();
        move |index| {
            let ui = weak.unwrap();
            let locale = if index == 1 { Locale::Zh } else { Locale::En };
            i18n::set_locale(locale);
            apply_strings(&ui);
            apply_unencrypted_hint(&ui);
            ui.set_error("".into());
            let st = state.lock().unwrap();
            ui.set_chat(ModelRc::new(VecModel::from(chat_rows(
                &st.chat,
                &st.chat_name,
            ))));
        }
    });

    ui.on_set_theme({
        let weak = ui.as_weak();
        let state = state.clone();
        move |index| {
            let ui = weak.unwrap();
            let mode = if index == 1 { Mode::Dark } else { Mode::Light };
            state.lock().unwrap().theme = mode;
            apply_theme(&ui, mode);
        }
    });

    ui.run()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).expect("init android backend");
    run().expect("run");
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    #[test]
    fn segment_selected_matches_desktop_tokens() {
        assert_eq!(
            segment_selected(Mode::Light),
            (LIGHT.control_checked, LIGHT.text)
        );
        // Same values the desktop client uses (DARK.border / DARK.text).
        assert_eq!(segment_selected(Mode::Dark), (DARK.border, DARK.text));
    }
}
