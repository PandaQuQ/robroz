//! Minimal blocking host client for the spike: `GET /api/groups`,
//! `GET /api/unread` and `GET /api/groups/{id}/events`, every request with
//! `Authorization: Bearer <token>`. It decodes with the `robroz-contract`
//! types, so the wire format is the one the desktop client uses.
//!
//! ureq runs with rustls so an `https://` tunnel works on Android and iOS.
//! The token is never printed: `Debug` hides it and error texts only carry
//! the base URL.

use std::fmt;
use std::time::Duration;

use robroz_contract::{ErrorBody, Event, Events, Group, Groups, Unread, Unreads};
use robroz_ui::i18n::{host_text, Key};

#[derive(Clone)]
pub struct Conn {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conn")
            .field("base", &self.base)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .finish()
    }
}

/// What went wrong, already in the active language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError(pub String);

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `https://` is assumed when the user types a bare host. Only http and
/// https are accepted. `None` for anything else.
pub fn normalize_base(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let rest = with_scheme
        .strip_prefix("https://")
        .or_else(|| with_scheme.strip_prefix("http://"))?;
    let rest = rest.trim_end_matches('/');
    (!rest.is_empty() && !rest.starts_with('/') && !rest.contains(char::is_whitespace))
        .then(|| with_scheme.trim_end_matches('/').to_string())
}

impl Conn {
    pub fn new(base: &str, token: &str) -> Result<Self, ApiError> {
        if base.trim().is_empty() {
            return Err(ApiError(Key::HostNeedUrl.as_str().to_string()));
        }
        let base = normalize_base(base)
            .ok_or_else(|| ApiError(Key::HostUrlInvalid.fill(&[&base.trim()])))?;
        let token = Some(token.trim().to_string()).filter(|t| !t.is_empty());
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .timeout_connect(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { base, token, agent })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn get(&self, path: &str) -> Result<String, ApiError> {
        let url = format!("{}{path}", self.base);
        let mut req = self.agent.get(&url);
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        let mut response = req
            .call()
            .map_err(|e| ApiError(Key::ClientConnectFailed.fill(&[&self.base, &e])))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| ApiError(Key::ClientReadFailed.fill(&[&url, &e])))?;
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let msg = serde_json::from_str::<ErrorBody>(&body)
            .map(|e| host_text(&e.error).into_owned())
            .unwrap_or_else(|_| Key::ClientHttpStatus.fill(&[&"GET", &url, &status]));
        Err(ApiError(msg))
    }

    fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, ApiError> {
        serde_json::from_str(body).map_err(|e| ApiError(Key::ClientDecodeFailed.fill(&[&e])))
    }

    /// `GET /api/groups`, oldest first.
    pub fn groups(&self) -> Result<Vec<Group>, ApiError> {
        Ok(Self::decode::<Groups>(&self.get("/api/groups")?)?.groups)
    }

    /// `GET /api/unread`, most recently active first.
    pub fn unread(&self) -> Result<Vec<Unread>, ApiError> {
        Ok(Self::decode::<Unreads>(&self.get("/api/unread")?)?.groups)
    }

    /// `GET /api/groups/{id}/events`, oldest first, each with whether a
    /// message is the user's own.
    ///
    /// The contract's `Event::Message` has no author field yet, so the side
    /// is read from a local `"from": "bro"` field on the raw JSON, which only
    /// the spike's fixture host sends. A real host omits it, so every message
    /// reads as the user's own (right side). The contract is not touched.
    pub fn events(&self, group_id: &str) -> Result<Vec<(Event, bool)>, ApiError> {
        let path = format!("/api/groups/{group_id}/events");
        let body = self.get(&path)?;
        let events = Self::decode::<Events>(&body)?.events;
        let from: Vec<Option<String>> = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("events")?.as_array().map(|a| {
                    a.iter()
                        .map(|e| e.get("from").and_then(|f| f.as_str()).map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_default();
        Ok(events
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                let own = from.get(i).and_then(|f| f.as_deref()) != Some("bro");
                (e, own)
            })
            .collect())
    }
}

/// Sessions as the list shows them: most recently active first, with the
/// unread count. Groups without activity keep host order after the active
/// ones.
pub fn session_rows(groups: &[Group], unread: &[Unread]) -> Vec<(Group, u64)> {
    let mut rows: Vec<(Group, u64)> = Vec::new();
    for u in unread {
        if let Some(g) = groups.iter().find(|g| g.id == u.group_id) {
            rows.push((g.clone(), u.unread));
        }
    }
    for g in groups {
        if !rows.iter().any(|(r, _)| r.id == g.id) {
            rows.push((g.clone(), 0));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One-request fake host; returns the raw request it saw.
    fn fake_host(status: &str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn base_url_is_normalised() {
        assert_eq!(
            normalize_base(" example.com/ "),
            Some("https://example.com".into())
        );
        assert_eq!(
            normalize_base("http://10.0.2.2:8790"),
            Some("http://10.0.2.2:8790".into())
        );
        assert_eq!(normalize_base("ftp://x"), None);
        assert_eq!(normalize_base(""), None);
        assert_eq!(normalize_base("https://"), None);
    }

    #[test]
    fn sends_bearer_and_decodes_groups() {
        let (base, seen) = fake_host(
            "200 OK",
            r#"{"groups":[{"id":"g1","name":"Ada","tenantId":"t"}]}"#,
        );
        let conn = Conn::new(&base, "dummy-token-for-fixture-only").unwrap();
        let groups = conn.groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Ada");
        let request = seen.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /api/groups "), "{request}");
        assert!(
            request.contains("authorization: bearer dummy-token-for-fixture-only"),
            "{request}"
        );
    }

    #[test]
    fn rejected_token_reads_as_the_host_text() {
        let body: &'static str = Box::leak(
            format!(
                r#"{{"error":"{}"}}"#,
                robroz_contract::host_text::TOKEN_REJECTED
            )
            .into_boxed_str(),
        );
        let (base, seen) = fake_host("401 Unauthorized", body);
        let conn = Conn::new(&base, "wrong").unwrap();
        let err = conn.groups().unwrap_err();
        seen.join().unwrap();
        let _en = robroz_ui::i18n::scoped(robroz_ui::i18n::Locale::En);
        assert_eq!(err.0, host_text(robroz_contract::host_text::TOKEN_REJECTED));
    }

    #[test]
    fn message_side_comes_from_the_local_from_field_and_defaults_to_own() {
        let body = r#"{"events":[
            {"type":"message","id":"m1","kind":"text","body":"hi","createdAt":"2026-11-20T10:00:00.000Z"},
            {"type":"message","id":"m2","kind":"text","body":"yo","createdAt":"2026-11-20T10:00:01.000Z","from":"bro"}]}"#;
        let (base, seen) = fake_host("200 OK", body);
        let conn = Conn::new(&base, "t").unwrap();
        let events = conn.events("g1").unwrap();
        seen.join().unwrap();
        let own: Vec<bool> = events.iter().map(|(_, own)| *own).collect();
        assert_eq!(own, [true, false]);
    }

    #[test]
    fn token_stays_out_of_debug_output() {
        let conn = Conn::new("http://127.0.0.1:1", "dummy-token-for-fixture-only").unwrap();
        assert!(!format!("{conn:?}").contains("dummy-token"));
    }

    #[test]
    fn rows_put_active_sessions_first() {
        let g = |id: &str| Group {
            id: id.into(),
            name: id.into(),
            tenant_id: "t".into(),
        };
        let u = |id: &str, n: u64| Unread {
            group_id: id.into(),
            read_at: None,
            unread: n,
            last_activity_at: None,
        };
        let rows = session_rows(&[g("a"), g("b"), g("c")], &[u("c", 2), u("a", 0)]);
        let ids: Vec<_> = rows.iter().map(|(g, n)| (g.id.as_str(), *n)).collect();
        assert_eq!(ids, [("c", 2), ("a", 0), ("b", 0)]);
    }
}
