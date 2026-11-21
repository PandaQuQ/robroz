//! One HTTP connection setup shared by the event source, the card API and
//! the live stream: base URL, timeouts and the optional host token.
//!
//! Token: `ROBROZ_TOKEN`, sent as `Authorization: Bearer …` on every request
//! when set (robroz#22). It is never printed: `Debug` hides it and error
//! texts only carry the base URL.

use std::fmt;
use std::time::Duration;

pub const DEFAULT_HOST: &str = "http://127.0.0.1:8790";

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

fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(timeout)
        .timeout_connect(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into()
}

impl Conn {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim().trim_end_matches('/').to_string(),
            token: None,
            agent: agent(Some(Duration::from_secs(5))),
        }
    }

    /// `ROBROZ_HOST` (default [`DEFAULT_HOST`]) and `ROBROZ_TOKEN`.
    pub fn from_env() -> Self {
        let base = std::env::var("ROBROZ_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
        Self::new(base).with_token(std::env::var("ROBROZ_TOKEN").ok())
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        self
    }

    /// Same host and token, no overall timeout: for the long-lived stream.
    pub fn streaming(&self) -> Self {
        Self {
            agent: agent(None),
            ..self.clone()
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn auth<B>(&self, req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        match &self.token {
            Some(t) => req.header("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    pub fn get(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.auth(self.agent.get(&self.url(path)))
    }

    pub fn post(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.auth(self.agent.post(&self.url(path)))
    }

    pub fn patch(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.auth(self.agent.patch(&self.url(path)))
    }

    pub fn put(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.auth(self.agent.put(&self.url(path)))
    }

    pub fn delete(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.auth(self.agent.delete(&self.url(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_base_and_hides_token() {
        let c = Conn::new(" http://h:1/ ").with_token(Some(" sekrit-0123456789 ".into()));
        assert_eq!(c.base(), "http://h:1");
        assert_eq!(c.url("/api/unread"), "http://h:1/api/unread");
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("sekrit"), "{dbg}");
        assert!(Conn::new("x").with_token(Some("  ".into())).token.is_none());
    }
}
