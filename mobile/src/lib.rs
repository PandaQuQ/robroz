//! robroz mobile client spike (Slint). Connect screen, session list, one
//! status card, Settings (language, About with the Slint attribution).
//!
//! Strings come from the `robroz-ui` i18n keys (no Slint copy of their own);
//! the host talk and the status card rules are in [`api`] and [`card`].

pub mod api;
pub mod card;
pub mod chat;

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use robroz_card::Palette;
use robroz_ui::i18n::{self, Key, Locale};

slint::include_modules!();

fn color(rgb: [u8; 3]) -> slint::Color {
    slint::Color::from_rgb_u8(rgb[0], rgb[1], rgb[2])
}

fn palette_colors(p: Palette) -> (slint::Color, slint::Color) {
    (color(p.fill), color(p.text))
}

fn apply_strings(ui: &MainWindow) {
    let tr = ui.global::<Tr>();
    tr.set_title_chats(Key::ConversationsTitle.as_str().into());
    tr.set_empty_chats(Key::ConversationsEmptyHint.as_str().into());
    tr.set_label_host(Key::MobileHostLabel.as_str().into());
    tr.set_label_token(Key::MobileTokenLabel.as_str().into());
    tr.set_connect(Key::MobileConnect.as_str().into());
    tr.set_back(Key::ViewBackToChats.as_str().into());
    tr.set_settings(Key::HostSettingsBotName.as_str().into());
    tr.set_language(Key::SettingsLanguage.as_str().into());
    tr.set_about(Key::MobileAbout.as_str().into());
    ui.set_lang_name(i18n::locale().other().native_name().into());
    ui.set_version_text(
        Key::MobileVersion
            .fill(&[&env!("CARGO_PKG_VERSION")])
            .into(),
    );
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
    if let Ok(h) = std::env::var("ROBROZ_HOST") {
        ui.set_host_url(h.into());
    }
    if let Ok(t) = std::env::var("ROBROZ_TOKEN") {
        ui.set_token(t.into());
    }
    apply_strings(&ui);

    ui.on_connect({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let ui = weak.unwrap();
            ui.set_error("".into());
            let conn = match api::Conn::new(&ui.get_host_url(), &ui.get_token()) {
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
                            state.lock().unwrap().conn = Some(conn);
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

    ui.on_toggle_language({
        let weak = ui.as_weak();
        let state = state.clone();
        move || {
            let ui = weak.unwrap();
            i18n::set_locale(i18n::locale().other());
            apply_strings(&ui);
            ui.set_error("".into());
            let st = state.lock().unwrap();
            ui.set_chat(ModelRc::new(VecModel::from(chat_rows(
                &st.chat,
                &st.chat_name,
            ))));
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
