//! Client preferences kept on this computer: the UI language and the
//! appearance (system, light or dark).
//!
//! `ROBROZ_LANG=en|zh` wins for a run and is never written back. Otherwise
//! the language picked with the language setting on the Settings page is
//! saved as one line (`en` or `zh`) in `lang` under the client's config folder:
//! `ROBROZ_CONFIG_DIR` if set, else `$XDG_CONFIG_HOME/robroz`,
//! `~/.config/robroz` (Linux), `~/Library/Application Support/robroz`
//! (macOS) or `%APPDATA%\robroz` (Windows). No file, or an unreadable one,
//! means English.
//!
//! The appearance picked in Settings is saved the same way as one line
//! (`system`, `light` or `dark`) in `theme` next to `lang`. `ROBROZ_THEME=
//! light|dark` wins, but only in fake and gallery runs (which also ignore
//! the saved choice, so review screenshots are reproducible). No file, or an
//! unreadable one, means follow the system.

use std::path::PathBuf;

use robroz_ui::i18n::Locale;
use robroz_ui::Mode;

const FILE: &str = "lang";
const THEME_FILE: &str = "theme";

/// What the user chose for the colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    /// Follow the operating system, live.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    /// The Settings row order.
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];

    /// The word saved in the `theme` file.
    pub fn code(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        }
    }

    /// `system`, `light` or `dark`, ignoring case and surrounding space.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::ALL
            .into_iter()
            .find(|p| p.code().eq_ignore_ascii_case(text))
    }

    /// The scheme to draw with, given the operating system's.
    pub fn resolve(self, system: Mode) -> Mode {
        match self {
            ThemePref::System => system,
            ThemePref::Light => Mode::Light,
            ThemePref::Dark => Mode::Dark,
        }
    }
}

/// The config folder, or `None` when no home or app-data folder is known.
pub fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env("ROBROZ_CONFIG_DIR") {
        return Some(dir);
    }
    if cfg!(windows) {
        return env("APPDATA").map(|d| d.join("robroz"));
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|h| h.join("Library/Application Support/robroz"));
    }
    env("XDG_CONFIG_HOME")
        .map(|d| d.join("robroz"))
        .or_else(|| env("HOME").map(|h| h.join(".config/robroz")))
}

/// `ROBROZ_LANG`, when set to a known language.
pub fn env_locale() -> Option<Locale> {
    std::env::var("ROBROZ_LANG")
        .ok()
        .as_deref()
        .and_then(Locale::parse)
}

/// The saved language in `dir`, if any.
pub fn load_in(dir: &std::path::Path) -> Option<Locale> {
    let text = std::fs::read_to_string(dir.join(FILE)).ok()?;
    Locale::parse(&text)
}

/// Save `locale` in `dir`, creating the folder. Errors are returned, not
/// shown: a failed save only means the next start uses the default.
pub fn save_in(dir: &std::path::Path, locale: Locale) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(FILE), format!("{}\n", locale.code()))
}

/// Language for this run: `ROBROZ_LANG`, then the saved choice (unless
/// `saved` is false, as in fake and gallery runs), then English.
pub fn initial_locale(saved: bool) -> Locale {
    env_locale()
        .or_else(|| saved.then(config_dir).flatten().and_then(|d| load_in(&d)))
        .unwrap_or_default()
}

/// `ROBROZ_THEME` as a choice: only `light` and `dark` are accepted.
fn env_theme_value(value: Option<&str>) -> Option<ThemePref> {
    value
        .and_then(ThemePref::parse)
        .filter(|p| *p != ThemePref::System)
}

/// The saved appearance in `dir`, if any.
pub fn load_theme_in(dir: &std::path::Path) -> Option<ThemePref> {
    let text = std::fs::read_to_string(dir.join(THEME_FILE)).ok()?;
    ThemePref::parse(&text)
}

/// Save `pref` in `dir`, creating the folder.
pub fn save_theme_in(dir: &std::path::Path, pref: ThemePref) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(THEME_FILE), format!("{}\n", pref.code()))
}

/// The choice given the raw pieces, so the order is testable: in a `fake`
/// run `env` wins and nothing saved is read; a real run ignores `env` and
/// uses `saved`; both fall back to following the system. A fake run with
/// no `ROBROZ_THEME` is light, so screenshots do not depend on the desktop.
fn pick_theme(fake: bool, env: Option<&str>, saved: Option<ThemePref>) -> ThemePref {
    if fake {
        env_theme_value(env).unwrap_or(ThemePref::Light)
    } else {
        saved.unwrap_or_default()
    }
}

/// Appearance for this run, see [`pick_theme`].
pub fn initial_theme(fake: bool) -> ThemePref {
    let env = std::env::var("ROBROZ_THEME").ok();
    let saved = (!fake)
        .then(config_dir)
        .flatten()
        .and_then(|d| load_theme_in(&d));
    pick_theme(fake, env.as_deref(), saved)
}

/// Save the appearance picked on the Settings page. Ignored when no config folder is known.
pub fn save_theme(pref: ThemePref) {
    if let Some(dir) = config_dir() {
        if let Err(err) = save_theme_in(&dir, pref) {
            eprintln!("robroz: could not save appearance: {err}");
        }
    }
}

/// Save the language picked on the Settings page. Ignored when no config folder is known.
pub fn save(locale: Locale) {
    if let Some(dir) = config_dir() {
        if let Err(err) = save_in(&dir, locale) {
            eprintln!("robroz: could not save language: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("robroz-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn nothing_saved_reads_none() {
        let dir = temp_dir("none");
        assert_eq!(load_in(&dir), None);
    }

    #[test]
    fn saved_choice_round_trips() {
        let dir = temp_dir("round");
        save_in(&dir, Locale::Zh).unwrap();
        assert_eq!(load_in(&dir), Some(Locale::Zh));
        save_in(&dir, Locale::En).unwrap();
        assert_eq!(load_in(&dir), Some(Locale::En));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn language_row_choice_persists_in_the_config_lang_file() {
        // Shared by the Settings (Model) row and the disconnected pane:
        // Message::SetLocale calls prefs::save, which writes `lang`.
        let dir = temp_dir("language-row");
        save_in(&dir, Locale::Zh).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), "zh\n");
        assert_eq!(load_in(&dir), Some(Locale::Zh));
        save_in(&dir, Locale::En).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), "en\n");
        assert_eq!(load_in(&dir), Some(Locale::En));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn junk_file_reads_none() {
        let dir = temp_dir("junk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "klingon").unwrap();
        assert_eq!(load_in(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_words_parse_loosely_and_reject_junk() {
        assert_eq!(ThemePref::parse("system"), Some(ThemePref::System));
        assert_eq!(ThemePref::parse(" Dark\n"), Some(ThemePref::Dark));
        assert_eq!(ThemePref::parse("LIGHT"), Some(ThemePref::Light));
        assert_eq!(ThemePref::parse("sepia"), None);
        assert_eq!(ThemePref::parse(""), None);
        for p in ThemePref::ALL {
            assert_eq!(ThemePref::parse(p.code()), Some(p));
        }
        assert_eq!(ThemePref::default(), ThemePref::System);
    }

    #[test]
    fn theme_resolves_against_the_system_only_when_following_it() {
        assert_eq!(ThemePref::System.resolve(Mode::Dark), Mode::Dark);
        assert_eq!(ThemePref::System.resolve(Mode::Light), Mode::Light);
        assert_eq!(ThemePref::Light.resolve(Mode::Dark), Mode::Light);
        assert_eq!(ThemePref::Dark.resolve(Mode::Light), Mode::Dark);
    }

    #[test]
    fn theme_choice_round_trips_in_the_theme_file_beside_lang() {
        let dir = temp_dir("theme-round");
        assert_eq!(load_theme_in(&dir), None);
        for p in ThemePref::ALL {
            save_theme_in(&dir, p).unwrap();
            assert_eq!(load_theme_in(&dir), Some(p));
        }
        save_theme_in(&dir, ThemePref::Dark).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(THEME_FILE)).unwrap(),
            "dark\n"
        );
        // Language and appearance live in separate files.
        save_in(&dir, Locale::Zh).unwrap();
        assert_eq!(load_theme_in(&dir), Some(ThemePref::Dark));
        assert_eq!(load_in(&dir), Some(Locale::Zh));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn junk_theme_file_reads_none() {
        let dir = temp_dir("theme-junk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(THEME_FILE), "sepia").unwrap();
        assert_eq!(load_theme_in(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_run_uses_saved_choice_and_ignores_the_env() {
        assert_eq!(pick_theme(false, None, None), ThemePref::System);
        assert_eq!(
            pick_theme(false, None, Some(ThemePref::Dark)),
            ThemePref::Dark
        );
        assert_eq!(
            pick_theme(false, Some("dark"), Some(ThemePref::Light)),
            ThemePref::Light
        );
        assert_eq!(pick_theme(false, Some("dark"), None), ThemePref::System);
    }

    #[test]
    fn fake_run_env_overrides_the_saved_choice() {
        assert_eq!(
            pick_theme(true, Some("dark"), Some(ThemePref::Light)),
            ThemePref::Dark
        );
        assert_eq!(
            pick_theme(true, Some("light"), Some(ThemePref::Dark)),
            ThemePref::Light
        );
        // Saved choice is not read in a fake run; no env means light.
        assert_eq!(
            pick_theme(true, None, Some(ThemePref::Dark)),
            ThemePref::Light
        );
        // `system` is not an env value; it falls back like junk does.
        assert_eq!(pick_theme(true, Some("system"), None), ThemePref::Light);
        assert_eq!(pick_theme(true, Some("sepia"), None), ThemePref::Light);
    }
}
