//! Client preferences kept on this computer: the UI language, and an optional
//! host token.
//!
//! `ROBROZ_LANG=en|zh` wins for a run and is never written back. Otherwise
//! the language picked with the language setting on the Settings page is
//! saved as one line (`en` or `zh`) in `lang` under the client's config folder:
//! `ROBROZ_CONFIG_DIR` if set, else `$XDG_CONFIG_HOME/robroz`,
//! `~/.config/robroz` (Linux), `~/Library/Application Support/robroz`
//! (macOS) or `%APPDATA%\robroz` (Windows). No file, or an unreadable one,
//! means English.
//!
//! The host token follows the same folder. `ROBROZ_TOKEN` wins for this run
//! and is never written back. Otherwise the client reads `token` next to
//! `lang`. A missing or unreadable file means no token. Writes use mode
//! `0600` on Unix. The value is never logged.

use std::path::PathBuf;

use robroz_ui::i18n::Locale;

const FILE: &str = "lang";
const TOKEN_FILE: &str = "token";

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

fn trim_token(raw: &str) -> Option<String> {
    let t = raw.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The saved host token in `dir`, if any. A trailing newline is stripped.
pub fn load_token_in(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    trim_token(&text)
}

/// Write `token` in `dir`, creating the folder. On Unix the file is mode
/// `0600`, so only this user can read it.
#[cfg(test)]
pub fn save_token_in(dir: &std::path::Path, token: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(TOKEN_FILE);
    let data = format!("{}\n", token.trim());
    std::fs::write(&path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Resolve the token: `env` (already the raw `ROBROZ_TOKEN` value, if any)
/// wins when it is non-empty after trim; otherwise the `token` file in `dir`.
pub fn token_from(env: Option<&str>, dir: Option<&std::path::Path>) -> Option<String> {
    env.and_then(trim_token)
        .or_else(|| dir.and_then(load_token_in))
}

/// Token for this run: `ROBROZ_TOKEN`, then the saved `token` file.
pub fn token_for_run() -> Option<String> {
    token_from(
        std::env::var("ROBROZ_TOKEN").ok().as_deref(),
        config_dir().as_deref(),
    )
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
    fn env_token_wins_over_the_saved_file() {
        let dir = temp_dir("token-env");
        save_token_in(&dir, "file-token-0123456789").unwrap();
        assert_eq!(
            token_from(None, Some(&dir)).as_deref(),
            Some("file-token-0123456789")
        );
        assert_eq!(
            token_from(Some("  "), Some(&dir)).as_deref(),
            Some("file-token-0123456789"),
            "whitespace-only env falls through to the file"
        );
        assert_eq!(
            token_from(Some(" env-token-0123456789 "), Some(&dir)).as_deref(),
            Some("env-token-0123456789")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn token_file_trims_and_ignores_blank() {
        let dir = temp_dir("token-blank");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(TOKEN_FILE), "  \n").unwrap();
        assert_eq!(load_token_in(&dir), None);
        std::fs::write(dir.join(TOKEN_FILE), "  sekrit-token-ok  \n").unwrap();
        assert_eq!(load_token_in(&dir).as_deref(), Some("sekrit-token-ok"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_token_sets_mode_600_on_unix() {
        let dir = temp_dir("token-mode");
        save_token_in(&dir, "sekrit-token-01234567").unwrap();
        let path = dir.join(TOKEN_FILE);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "sekrit-token-01234567\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{mode:o}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_token_file_reads_none() {
        let dir = temp_dir("token-none");
        assert_eq!(load_token_in(&dir), None);
    }
}
