//! Host address and token persistence, plus the public settings API
//! the connection form calls.
//!
//! The token never leaves this module as plaintext: `load` only returns a
//! [`TokenState`], and [`Conn::from_env`] is how the client applies the
//! stored values.
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use super::{normalize_host, Conn, DEFAULT_HOST};
use crate::prefs;

const HOST_FILE: &str = "host";
const TOKEN_FILE: &str = "token";
const KEYRING_SERVICE: &str = "robroz";
const KEYRING_USER: &str = "host-token";

/// Where a host address came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Saved,
    Default,
}

/// Whether a token is configured, without exposing its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    None,
    Saved,
    Env,
}

/// What `save` should do with the stored token.
pub enum TokenEdit {
    Keep,
    Set(String),
    Clear,
}

impl fmt::Debug for TokenEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keep => f.write_str("Keep"),
            Self::Clear => f.write_str("Clear"),
            Self::Set(_) => f.write_str("Set(***)"),
        }
    }
}

/// Settings the connection form can show. The token value stays inside [`Conn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnConfig {
    pub host: String,
    pub host_source: Source,
    pub token: TokenState,
}

/// Why `save` refused or failed. Never contains the token.
#[derive(Debug)]
pub enum SaveError {
    /// At least one field is set by an environment variable and was not written.
    EnvOverride {
        host: bool,
        token: bool,
    },
    Io(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvOverride { host, token } => match (*host, *token) {
                (true, true) => f.write_str("Set by ROBROZ_HOST and ROBROZ_TOKEN"),
                (true, false) => f.write_str("Set by ROBROZ_HOST"),
                (false, true) => f.write_str("Set by ROBROZ_TOKEN"),
                (false, false) => f.write_str("overridden by an environment variable"),
            },
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<io::Error> for SaveError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Typed failure for the first connect and later retries, so the UI can tell
/// a rejected token from a host that cannot be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnError {
    Unauthorized,
    Unreachable(String),
}

impl ConnError {
    /// Map a status-and-body from the host into a typed error.
    #[allow(dead_code)] // public for the form; the first Connect uses `as_conn_error`.
    pub fn from_status(status: u16, body: &str) -> Option<Self> {
        if status == 401 || crate::host_state::is_token_rejected(body) {
            Some(Self::Unauthorized)
        } else {
            None
        }
    }

    /// Map a text error from the existing request and retry path.
    pub fn classify(err: &str) -> Self {
        if crate::host_state::is_token_rejected(err) {
            return Self::Unauthorized;
        }
        let reason = crate::host_state::connect_reason(err)
            .filter(|key| *key != robroz_ui::i18n::Key::ClientTokenRejected)
            .map(|key| key.to_string())
            .unwrap_or_default();
        Self::Unreachable(reason)
    }
}

impl fmt::Display for ConnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "{}", robroz_ui::i18n::Key::ClientTokenRejected),
            Self::Unreachable(reason) if reason.is_empty() => {
                write!(f, "{}", robroz_ui::i18n::Key::ClientCantReachHost)
            }
            Self::Unreachable(reason) => {
                write!(f, "{} {reason}", robroz_ui::i18n::Key::ClientCantReachHost)
            }
        }
    }
}

/// Load the current host and whether a token is set.
///
/// `ROBROZ_HOST` and `ROBROZ_TOKEN` override any saved values.
pub fn load() -> ConnConfig {
    load_from(
        env_host(),
        env_token_present(),
        prefs::config_dir().as_deref(),
        &OsSecrets,
    )
}

/// Persist the host and optionally the token.
///
/// Never overwrites a field that an environment variable already sets:
/// returns [`SaveError::EnvOverride`] and writes nothing. After a successful
/// save, build a new [`Conn`] with [`Conn::from_env`] so the next request
/// uses the new values.
pub fn save(host: &str, token: TokenEdit) -> Result<(), SaveError> {
    let dir = prefs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config folder"))?;
    save_from(
        host,
        token,
        env_host(),
        env_token_present(),
        &dir,
        &OsSecrets,
    )
}

fn env_host() -> Option<String> {
    normalize_env(std::env::var("ROBROZ_HOST").ok())
}

fn env_token_present() -> bool {
    normalize_env(std::env::var("ROBROZ_TOKEN").ok()).is_some()
}

fn env_token_value() -> Option<String> {
    normalize_env(std::env::var("ROBROZ_TOKEN").ok())
}

fn normalize_env(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// Read the secret token used on requests. The UI never sees this.
pub(super) fn token_for_run() -> Option<String> {
    if let Some(token) = env_token_value() {
        return Some(token);
    }
    load_secret(&OsSecrets, prefs::config_dir().as_deref())
}

trait Secrets {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, token: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

struct OsSecrets;

impl Secrets for OsSecrets {
    fn get(&self) -> Result<Option<String>, String> {
        match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            Ok(entry) => match entry.get_password() {
                Ok(value) => {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(trimmed.to_string()))
                    }
                }
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(err) => Err(err.to_string()),
            },
            Err(err) => Err(err.to_string()),
        }
    }

    fn set(&self, token: &str) -> Result<(), String> {
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
        entry.set_password(token).map_err(|e| e.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        let entry =
            keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

fn load_from(
    env_host: Option<String>,
    env_token: bool,
    dir: Option<&Path>,
    secrets: &dyn Secrets,
) -> ConnConfig {
    let (host, host_source) = if let Some(host) = env_host {
        (normalize_host(&host), Source::Env)
    } else if let Some(saved) = dir.and_then(load_host) {
        (saved, Source::Saved)
    } else {
        (DEFAULT_HOST.to_string(), Source::Default)
    };
    let token = if env_token {
        TokenState::Env
    } else if load_secret(secrets, dir).is_some() {
        TokenState::Saved
    } else {
        TokenState::None
    };
    ConnConfig {
        host,
        host_source,
        token,
    }
}

fn save_from(
    host: &str,
    token: TokenEdit,
    env_host: Option<String>,
    env_token: bool,
    dir: &Path,
    secrets: &dyn Secrets,
) -> Result<(), SaveError> {
    let host = normalize_host(host);
    let host_blocked = if let Some(env) = env_host.as_deref() {
        normalize_host(env) != host
    } else {
        false
    };
    let token_blocked = env_token && !matches!(token, TokenEdit::Keep);
    if host_blocked || token_blocked {
        return Err(SaveError::EnvOverride {
            host: host_blocked,
            token: token_blocked,
        });
    }

    if env_host.is_none() {
        write_host(dir, &host)?;
    }
    match token {
        TokenEdit::Keep => {}
        TokenEdit::Set(value) => {
            let value = value.trim();
            if value.is_empty() {
                clear_secret(dir, secrets)?;
            } else {
                store_secret(dir, secrets, value)?;
            }
        }
        TokenEdit::Clear => clear_secret(dir, secrets)?,
    }
    Ok(())
}

fn load_host(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(HOST_FILE)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(normalize_host(trimmed))
    }
}

fn write_host(dir: &Path, host: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(HOST_FILE);
    let tmp = dir.join("host.tmp");
    fs::write(&tmp, format!("{host}\n"))?;
    fs::rename(&tmp, &path)
}

fn load_secret(secrets: &dyn Secrets, dir: Option<&Path>) -> Option<String> {
    if let Ok(Some(token)) = secrets.get() {
        return Some(token);
    }
    dir.and_then(load_token_file)
}

fn load_token_file(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn store_secret(dir: &Path, secrets: &dyn Secrets, token: &str) -> io::Result<()> {
    // Only trust the keychain once the value reads back. Otherwise keep the
    // 0600 file, so a broken or volatile store never loses the token.
    let stored =
        secrets.set(token).is_ok() && matches!(secrets.get(), Ok(Some(ref v)) if v == token);
    if stored {
        let _ = fs::remove_file(dir.join(TOKEN_FILE));
        Ok(())
    } else {
        write_token_file(dir, token)
    }
}

fn clear_secret(dir: &Path, secrets: &dyn Secrets) -> io::Result<()> {
    let keychain_ok = secrets.delete().is_ok();
    let file = dir.join(TOKEN_FILE);
    match fs::remove_file(&file) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            if !keychain_ok {
                return Err(err);
            }
        }
    }
    if keychain_ok || !file.exists() {
        Ok(())
    } else {
        Err(io::Error::other("could not clear the stored token"))
    }
}

fn write_token_file(dir: &Path, token: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(TOKEN_FILE);
    let tmp = dir.join("token.tmp");
    fs::write(&tmp, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, &path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Apply the current settings (env, then saved, then defaults) to a new connection.
impl Conn {
    /// Build a connection from [`load`] plus the stored token, which stays inside `Conn`.
    pub fn from_env() -> Self {
        let cfg = load();
        Self::new(cfg.host).with_token(token_for_run())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> PathBuf {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("robroz-conn-cfg-{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct MemorySecrets {
        value: RefCell<Option<String>>,
        fail: bool,
        /// `set` reports success but nothing is kept (a volatile or broken store).
        forgetful: bool,
    }

    impl MemorySecrets {
        fn empty() -> Self {
            Self {
                value: RefCell::new(None),
                fail: false,
                forgetful: false,
            }
        }

        fn failing() -> Self {
            Self {
                value: RefCell::new(None),
                fail: true,
                forgetful: false,
            }
        }

        fn forgetful() -> Self {
            Self {
                value: RefCell::new(None),
                fail: false,
                forgetful: true,
            }
        }
    }

    impl Secrets for MemorySecrets {
        fn get(&self) -> Result<Option<String>, String> {
            if self.fail {
                return Err("no keychain".into());
            }
            Ok(self.value.borrow().clone())
        }

        fn set(&self, token: &str) -> Result<(), String> {
            if self.fail {
                return Err("no keychain".into());
            }
            if !self.forgetful {
                *self.value.borrow_mut() = Some(token.to_string());
            }
            Ok(())
        }

        fn delete(&self) -> Result<(), String> {
            if self.fail {
                return Err("no keychain".into());
            }
            *self.value.borrow_mut() = None;
            Ok(())
        }
    }

    fn mode(path: &Path) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            0o600
        }
    }

    #[test]
    fn load_prefers_env_over_saved_and_default() {
        let dir = scratch();
        write_host(&dir, "http://saved.example:1").unwrap();
        let secrets = MemorySecrets::empty();
        secrets.set("saved-token").unwrap();

        let from_files = load_from(None, false, Some(&dir), &secrets);
        assert_eq!(from_files.host, "http://saved.example:1");
        assert_eq!(from_files.host_source, Source::Saved);
        assert_eq!(from_files.token, TokenState::Saved);

        let from_env = load_from(
            Some(" http://env.example:9/ ".into()),
            true,
            Some(&dir),
            &secrets,
        );
        assert_eq!(from_env.host, "http://env.example:9");
        assert_eq!(from_env.host_source, Source::Env);
        assert_eq!(from_env.token, TokenState::Env);

        let def = load_from(None, false, None, &MemorySecrets::empty());
        assert_eq!(def.host, DEFAULT_HOST);
        assert_eq!(def.host_source, Source::Default);
        assert_eq!(def.token, TokenState::None);
    }

    #[test]
    fn save_refuses_to_overwrite_an_env_override() {
        let dir = scratch();
        write_host(&dir, "http://saved.example:1").unwrap();
        let secrets = MemorySecrets::empty();
        secrets.set("saved-token").unwrap();

        let err = save_from(
            "http://other.example",
            TokenEdit::Set("new-token".into()),
            Some("http://env.example".into()),
            true,
            &dir,
            &secrets,
        )
        .unwrap_err();
        match err {
            SaveError::EnvOverride { host, token } => {
                assert!(host);
                assert!(token);
            }
            other => panic!("expected EnvOverride, got {other:?}"),
        }
        assert_eq!(load_host(&dir).as_deref(), Some("http://saved.example:1"));
        assert_eq!(secrets.get().unwrap().as_deref(), Some("saved-token"));
    }

    #[test]
    fn save_keeps_an_env_token_and_can_write_the_host() {
        let dir = scratch();
        let secrets = MemorySecrets::empty();
        save_from(
            "http://new.example:2/",
            TokenEdit::Keep,
            None,
            true,
            &dir,
            &secrets,
        )
        .unwrap();
        assert_eq!(load_host(&dir).as_deref(), Some("http://new.example:2"));
        assert!(secrets.get().unwrap().is_none());
        assert!(!dir.join(TOKEN_FILE).exists());
    }

    #[test]
    fn keyring_failure_falls_back_to_a_0600_file() {
        let dir = scratch();
        let secrets = MemorySecrets::failing();
        save_from(
            DEFAULT_HOST,
            TokenEdit::Set("  file-token  ".into()),
            None,
            false,
            &dir,
            &secrets,
        )
        .unwrap();

        let path = dir.join(TOKEN_FILE);
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "file-token");
        assert_eq!(mode(&path), 0o600);

        let loaded = load_from(None, false, Some(&dir), &secrets);
        assert_eq!(loaded.token, TokenState::Saved);
        assert_eq!(
            load_secret(&secrets, Some(&dir)).as_deref(),
            Some("file-token")
        );
    }

    #[test]
    fn unverified_keychain_write_keeps_the_0600_file() {
        let dir = scratch();
        let secrets = MemorySecrets::forgetful();
        save_from(
            DEFAULT_HOST,
            TokenEdit::Set("must-survive".into()),
            None,
            false,
            &dir,
            &secrets,
        )
        .unwrap();
        let path = dir.join(TOKEN_FILE);
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "must-survive");
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            load_secret(&secrets, Some(&dir)).as_deref(),
            Some("must-survive")
        );
    }

    #[test]
    fn keep_leaves_the_token_and_clear_removes_it() {
        let dir = scratch();
        let secrets = MemorySecrets::empty();
        save_from(
            "http://one.example",
            TokenEdit::Set("keep-me".into()),
            None,
            false,
            &dir,
            &secrets,
        )
        .unwrap();
        // A successful keychain write must drop a stale file so it cannot
        // come back if the keychain later fails.
        assert!(!dir.join(TOKEN_FILE).exists());

        save_from(
            "http://two.example",
            TokenEdit::Keep,
            None,
            false,
            &dir,
            &secrets,
        )
        .unwrap();
        assert_eq!(secrets.get().unwrap().as_deref(), Some("keep-me"));
        assert_eq!(load_host(&dir).as_deref(), Some("http://two.example"));

        save_from(
            "http://two.example",
            TokenEdit::Clear,
            None,
            false,
            &dir,
            &secrets,
        )
        .unwrap();
        assert!(secrets.get().unwrap().is_none());
        assert!(!dir.join(TOKEN_FILE).exists());
        let loaded = load_from(None, false, Some(&dir), &secrets);
        assert_eq!(loaded.token, TokenState::None);
    }

    #[test]
    fn clear_also_removes_the_fallback_file() {
        let dir = scratch();
        write_token_file(&dir, "file-token").unwrap();
        let secrets = MemorySecrets::failing();
        save_from(DEFAULT_HOST, TokenEdit::Clear, None, false, &dir, &secrets).unwrap();
        assert!(!dir.join(TOKEN_FILE).exists());
        assert_eq!(
            load_from(None, false, Some(&dir), &secrets).token,
            TokenState::None
        );
    }

    #[test]
    fn token_edit_debug_hides_the_value() {
        assert_eq!(format!("{:?}", TokenEdit::Set("secret".into())), "Set(***)");
        assert_eq!(format!("{:?}", TokenEdit::Keep), "Keep");
        assert_eq!(format!("{:?}", TokenEdit::Clear), "Clear");
    }

    #[test]
    fn conn_error_separates_unauthorized_from_unreachable() {
        assert_eq!(
            ConnError::from_status(401, "缺少或错误的令牌"),
            Some(ConnError::Unauthorized)
        );
        assert_eq!(ConnError::from_status(404, "gone"), None);
        assert_eq!(
            ConnError::classify("GET http://127.0.0.1:1/api/groups returned 401: missing"),
            ConnError::Unauthorized
        );
        assert_eq!(
            ConnError::classify("io: Connection refused (os error 111)"),
            ConnError::Unreachable(robroz_ui::i18n::Key::ClientConnectionRefused.to_string())
        );
        let shown = ConnError::Unauthorized.to_string();
        assert_eq!(shown, "The host rejected the token. Check ROBROZ_TOKEN.");
        let reach = ConnError::Unreachable("Connection refused".into()).to_string();
        assert!(
            reach.starts_with("Can't reach host."),
            "expected the approved can't-reach prefix, got {reach}"
        );
    }
}
