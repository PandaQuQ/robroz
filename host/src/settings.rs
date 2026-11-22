//! Shared SQLite `settings` table, used by search and model settings.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
}

pub(crate) fn env_get(name: &str) -> Option<String> {
    let raw = {
        #[cfg(test)]
        {
            test_env::get(name)
        }
        #[cfg(not(test))]
        {
            std::env::var(name).ok()
        }
    };
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub(crate) fn setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    let value: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

pub(crate) fn upsert_setting(
    conn: &Connection,
    key: &str,
    value: Option<&str>,
) -> rusqlite::Result<()> {
    match value {
        None => {
            conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        }
        Some(value) => {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn last4(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let n = value.chars().count();
    Some(value.chars().skip(n.saturating_sub(4)).collect())
}

pub(crate) fn as_opt_opt<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

#[cfg(test)]
pub(crate) mod test_env {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static ENV: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
    }

    pub fn get(name: &str) -> Option<String> {
        ENV.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|map| map.get(name).cloned())
        })
    }

    pub fn with(pairs: &[(&str, &str)], f: impl FnOnce()) {
        let _guard = set(pairs);
        f();
    }

    pub struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            ENV.with(|slot| *slot.borrow_mut() = None);
        }
    }

    pub fn set(pairs: &[(&str, &str)]) -> Guard {
        ENV.with(|slot| {
            *slot.borrow_mut() = Some(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
        });
        Guard
    }
}
