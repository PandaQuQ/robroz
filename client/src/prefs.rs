//! Client preferences kept on this computer: the UI language.
//!
//! `ROBROZ_LANG=en|zh` wins for a run and is never written back. Otherwise
//! the language picked with the language setting on the Settings page is
//! saved as one line (`en` or `zh`) in `lang` under the client's config folder:
//! `ROBROZ_CONFIG_DIR` if set, else `$XDG_CONFIG_HOME/robroz`,
//! `~/.config/robroz` (Linux), `~/Library/Application Support/robroz`
//! (macOS) or `%APPDATA%\robroz` (Windows). No file, or an unreadable one,
//! means English.

use std::path::PathBuf;

use robroz_ui::i18n::Locale;

const FILE: &str = "lang";

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
    fn junk_file_reads_none() {
        let dir = temp_dir("junk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), "klingon").unwrap();
        assert_eq!(load_in(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
