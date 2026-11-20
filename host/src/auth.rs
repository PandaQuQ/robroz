//! Listener guard and API token.
//!
//! - Default listener is loopback (`127.0.0.1:8790`); local use needs no token.
//! - Binding anything that is not loopback refuses to start unless a token is set.
//! - The token comes only from `ROBROZ_TOKEN` or a file named by
//!   `ROBROZ_TOKEN_FILE`. It is never printed: [`ApiToken`]'s `Debug` is redacted
//!   and no error message includes it.
//! - When a token is set, every `/api` request needs `Authorization: Bearer <token>`,
//!   on loopback too.

use std::net::SocketAddr;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};

/// Shortest token accepted, so a non-loopback host is not guarded by `123`.
pub const TOKEN_MIN_LEN: usize = 16;

#[derive(Clone, PartialEq, Eq)]
pub struct ApiToken(String);

impl std::fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiToken(***)")
    }
}

impl ApiToken {
    /// Trims surrounding whitespace (token files end with a newline). Empty means unset.
    pub fn parse(raw: &str) -> Result<Option<Self>, String> {
        let token = raw.trim();
        if token.is_empty() {
            return Ok(None);
        }
        if token.chars().count() < TOKEN_MIN_LEN {
            return Err(format!("令牌太短，至少 {TOKEN_MIN_LEN} 个字符"));
        }
        if token.chars().any(char::is_whitespace) {
            return Err("令牌里不能有空白字符".into());
        }
        Ok(Some(Self(token.to_string())))
    }

    /// Constant-time compare, so response timing does not leak the token.
    fn matches(&self, given: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), given.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

/// `ROBROZ_TOKEN` wins; otherwise read `ROBROZ_TOKEN_FILE`. Setting both is an error.
pub fn token_from(env_token: Option<String>, token_file: Option<String>) -> Result<Option<ApiToken>, String> {
    match (env_token.filter(|v| !v.trim().is_empty()), token_file.filter(|v| !v.trim().is_empty())) {
        (Some(_), Some(_)) => Err("ROBROZ_TOKEN 和 ROBROZ_TOKEN_FILE 只能设一个".into()),
        (Some(raw), None) => ApiToken::parse(&raw).map_err(|e| format!("ROBROZ_TOKEN: {e}")),
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(&path).map_err(|e| format!("读不了 ROBROZ_TOKEN_FILE {path}: {e}"))?;
            ApiToken::parse(&raw).map_err(|e| format!("ROBROZ_TOKEN_FILE: {e}"))
        }
        (None, None) => Ok(None),
    }
}

pub fn token_from_env() -> Result<Option<ApiToken>, String> {
    token_from(std::env::var("ROBROZ_TOKEN").ok(), std::env::var("ROBROZ_TOKEN_FILE").ok())
}

/// Startup guard: a non-loopback listener without a token is refused.
pub fn check_listener(addr: SocketAddr, token: Option<&ApiToken>) -> Result<(), String> {
    if addr.ip().is_loopback() || token.is_some() {
        Ok(())
    } else {
        Err(format!(
            "拒绝启动：{addr} 不是本机回环地址，必须先设置 ROBROZ_TOKEN 或 ROBROZ_TOKEN_FILE"
        ))
    }
}

/// Wraps `router` so every request carries the bearer token. `None` leaves it open.
pub fn require_token(router: Router, token: Option<ApiToken>) -> Router {
    match token {
        None => router,
        Some(token) => router.layer(middleware::from_fn_with_state(token, check_bearer)),
    }
}

async fn check_bearer(State(token): State<ApiToken>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if given.is_some_and(|g| token.matches(g)) {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(serde_json::json!({ "error": "缺少或错误的令牌" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef-good";

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_starts_without_token() {
        assert!(check_listener(addr("127.0.0.1:8790"), None).is_ok());
        assert!(check_listener(addr("[::1]:8790"), None).is_ok());
    }

    #[test]
    fn non_loopback_refuses_without_token() {
        for a in ["0.0.0.0:8790", "192.168.2.10:8790", "[::]:8790"] {
            let err = check_listener(addr(a), None).unwrap_err();
            assert!(err.contains("ROBROZ_TOKEN"), "{err}");
        }
        let token = ApiToken::parse(GOOD).unwrap().unwrap();
        assert!(check_listener(addr("0.0.0.0:8790"), Some(&token)).is_ok());
    }

    #[test]
    fn token_parsing() {
        assert_eq!(ApiToken::parse("").unwrap(), None);
        assert_eq!(ApiToken::parse("  \n").unwrap(), None);
        assert!(ApiToken::parse("short").is_err());
        assert!(ApiToken::parse("0123456789 abcdefgh").is_err());
        assert_eq!(ApiToken::parse(&format!("{GOOD}\n")).unwrap(), ApiToken::parse(GOOD).unwrap());
    }

    #[test]
    fn token_never_shows_up_in_debug_or_errors() {
        let token = ApiToken::parse(GOOD).unwrap().unwrap();
        assert!(!format!("{token:?}").contains(GOOD));
        assert!(!format!("{:?}", Some(&token)).contains(GOOD));
        let err = token_from(Some(GOOD.into()), Some("/x".into())).unwrap_err();
        assert!(!err.contains(GOOD));
        let err = token_from(Some("tiny-secret".into()), None).unwrap_err();
        assert!(!err.contains("tiny-secret"), "{err}");
    }

    #[test]
    fn token_sources() {
        assert_eq!(token_from(None, None).unwrap(), None);
        assert_eq!(token_from(Some(" ".into()), None).unwrap(), None);
        let dir = std::env::temp_dir().join(format!("robroz-token-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("token");
        std::fs::write(&file, format!("{GOOD}\n")).unwrap();
        let from_file = token_from(None, Some(file.display().to_string())).unwrap();
        assert_eq!(from_file, ApiToken::parse(GOOD).unwrap());
        assert!(token_from(None, Some(dir.join("missing").display().to_string())).is_err());
        assert!(token_from(Some(GOOD.into()), Some(file.display().to_string())).is_err());
    }

    #[test]
    fn compare_is_exact() {
        let token = ApiToken::parse(GOOD).unwrap().unwrap();
        assert!(token.matches(GOOD));
        assert!(!token.matches(&GOOD[1..]));
        assert!(!token.matches(&format!("{GOOD}x")));
        assert!(!token.matches(""));
    }
}
