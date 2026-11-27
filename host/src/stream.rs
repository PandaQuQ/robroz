//! `GET /api/groups/{id}/stream`: live events over Server-Sent Events.
//!
//! Each SSE message is one contract `Event` as JSON (`data:`), its `type` as the
//! SSE `event:` name, and the card's change time as `id:`. Reconnect with the
//! standard `Last-Event-ID` header (or `?since=`) to get only what changed after
//! it. A card that changes again is sent again with the same `data.id`, so the
//! client replaces it in place, exactly as with `GET …/events`. Nothing changed
//! means nothing is sent apart from keep-alive comments. Polling `…/events?since=`
//! stays as the fallback.
//!
//! The stream reads the same query as `…/events` on a short interval, so write
//! paths stay untouched.
//!
//! Long-lived streams are bounded ([`StreamConfig`]): the poll interval and
//! keep-alive come from the environment; a stream closes cleanly after
//! `ROBROZ_SSE_MAX_AGE_SECS` (the client reconnects with `Last-Event-ID` and
//! loses nothing, and its token and membership are checked again); and past
//! `ROBROZ_SSE_MAX_STREAMS` open streams a new one gets 503 with
//! `Retry-After: 5`. Responses carry `X-Accel-Buffering: no` so a reverse
//! proxy does not hold frames back.

use std::collections::{HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::extract::{Path as AxumPath, Query};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};

use crate::{
    error_response, group_events, lock_db, parse_since, AppState, EventsQuery, TimedEvent,
};

/// Default for `ROBROZ_SSE_POLL_MS`.
pub const POLL: Duration = Duration::from_millis(250);
/// Default for `ROBROZ_SSE_KEEPALIVE_SECS`.
pub const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// Default for `ROBROZ_SSE_MAX_AGE_SECS`.
pub const MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// Default for `ROBROZ_SSE_MAX_STREAMS`.
pub const MAX_STREAMS: usize = 64;
/// `Retry-After` on the 503 for one stream too many, in seconds.
pub const RETRY_AFTER_SECS: u64 = 5;
/// `code` of that 503.
pub const TOO_MANY_STREAMS: &str = "too_many_streams";

/// Limits of live streams. Every field falls back to its default when the
/// variable is missing, zero or not a positive integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    /// `ROBROZ_SSE_POLL_MS`: how often a stream checks for changes.
    pub poll: Duration,
    /// `ROBROZ_SSE_KEEPALIVE_SECS`: keep-alive comment interval.
    pub keep_alive: Duration,
    /// `ROBROZ_SSE_MAX_AGE_SECS`: a stream is closed cleanly after this.
    pub max_age: Duration,
    /// `ROBROZ_SSE_MAX_STREAMS`: open streams across all groups.
    pub max_streams: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            poll: POLL,
            keep_alive: KEEP_ALIVE,
            max_age: MAX_AGE,
            max_streams: MAX_STREAMS,
        }
    }
}

impl StreamConfig {
    pub fn from_env() -> Self {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let num = |name: &str| crate::env_num::positive(get(name).as_deref());
        let d = Self::default();
        Self {
            poll: num("ROBROZ_SSE_POLL_MS").map_or(d.poll, Duration::from_millis),
            keep_alive: num("ROBROZ_SSE_KEEPALIVE_SECS").map_or(d.keep_alive, Duration::from_secs),
            max_age: num("ROBROZ_SSE_MAX_AGE_SECS").map_or(d.max_age, Duration::from_secs),
            max_streams: num("ROBROZ_SSE_MAX_STREAMS")
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(d.max_streams),
        }
    }
}

/// One open stream counted in [`crate::AppState`]; released on drop, which
/// is when the client goes away or the stream ends.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>, max: usize) -> Option<Self> {
        // Counted first, so two racing requests cannot both take the last one.
        let slot = Self(open.clone());
        (open.fetch_add(1, Ordering::SeqCst) < max).then_some(slot)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn too_many_streams() -> Response {
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({
            "error": "too many open streams; retry later",
            "code": TOO_MANY_STREAMS,
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(RETRY_AFTER_SECS));
    response
}

/// Where a stream has got to. Rows written in the same millisecond as the
/// cursor are tracked by id so they are neither lost nor sent twice.
struct Cursor {
    at: Option<String>,
    sent_at_cursor: HashSet<String>,
}

impl Cursor {
    fn take_new(&mut self, timed: Vec<TimedEvent>) -> Vec<TimedEvent> {
        let fresh: Vec<TimedEvent> = timed
            .into_iter()
            .filter(|t| match &self.at {
                None => true,
                Some(at) => {
                    t.changed_at > *at
                        || (t.changed_at == *at && !self.sent_at_cursor.contains(&t.id))
                }
            })
            .collect();
        for t in &fresh {
            if self.at.as_ref().is_none_or(|at| t.changed_at > *at) {
                self.at = Some(t.changed_at.clone());
                self.sent_at_cursor.clear();
            }
        }
        if let Some(at) = &self.at {
            for t in fresh.iter().filter(|t| &t.changed_at == at) {
                self.sent_at_cursor.insert(t.id.clone());
            }
        }
        fresh
    }

    /// Lower bound for the next query: `>= at`, so same-millisecond rows show up.
    fn query_since(&self) -> Option<String> {
        self.at.as_ref().map(|at| {
            let t = chrono::DateTime::parse_from_rfc3339(at).expect("cursor is our own timestamp");
            (t - chrono::Duration::milliseconds(1))
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
    }
}

fn to_sse(t: &TimedEvent) -> SseEvent {
    let value = serde_json::to_value(&t.event).expect("contract events serialise");
    let name = value["type"].as_str().unwrap_or("event").to_string();
    SseEvent::default()
        .event(name)
        .id(t.changed_at.clone())
        .data(value.to_string())
}

pub(crate) async fn stream_events(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    let raw = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| query.get("since").cloned());
    let Ok(since) = parse_since(raw.as_deref()) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            robroz_contract::host_text::SINCE_INVALID,
        );
    };
    let config = state.stream;
    let Some(slot) = Slot::take(&state.open_streams, config.max_streams) else {
        return too_many_streams();
    };
    let deadline = tokio::time::Instant::now() + config.max_age;
    let mut cursor = Cursor {
        at: since,
        sent_at_cursor: HashSet::new(),
    };
    let first = {
        let conn = lock_db(&state);
        group_events(&conn, &group_id, cursor.query_since().as_deref())
    };
    let pending: VecDeque<TimedEvent> = match first {
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                robroz_contract::host_text::GROUP_NOT_FOUND,
            )
        }
        Err(err) => return crate::db_error(err),
        Ok(Some(timed)) => cursor.take_new(timed).into(),
    };

    let stream = futures_util::stream::unfold(
        (state, group_id, cursor, pending, slot),
        move |(state, group_id, mut cursor, mut pending, slot)| async move {
            loop {
                if let Some(next) = pending.pop_front() {
                    let sse = to_sse(&next);
                    return Some((
                        Ok::<_, Infallible>(sse),
                        (state, group_id, cursor, pending, slot),
                    ));
                }
                // Old enough: end cleanly between batches. The client
                // reconnects with its last id and misses nothing.
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    return None;
                }
                tokio::time::sleep(config.poll.min(deadline - now)).await;
                if tokio::time::Instant::now() >= deadline {
                    return None;
                }
                let found = {
                    let conn = lock_db(&state);
                    group_events(&conn, &group_id, cursor.query_since().as_deref())
                };
                match found {
                    Ok(Some(timed)) => pending.extend(cursor.take_new(timed)),
                    Ok(None) => return None,
                    Err(err) => {
                        eprintln!("stream {group_id}: {err}");
                        return None;
                    }
                }
            }
        },
    );
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(config.keep_alive))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_config_env_like_the_watchdog() {
        let none = StreamConfig::from_vars(|_| None);
        assert_eq!(none, StreamConfig::default());
        assert_eq!(none.poll, Duration::from_millis(250));
        assert_eq!(none.keep_alive, Duration::from_secs(15));
        assert_eq!(none.max_age, Duration::from_secs(3600));
        assert_eq!(none.max_streams, 64);
        let junk = StreamConfig::from_vars(|name| {
            Some(match name {
                "ROBROZ_SSE_POLL_MS" => "0".into(),
                "ROBROZ_SSE_KEEPALIVE_SECS" => "".into(),
                "ROBROZ_SSE_MAX_AGE_SECS" => "soon".into(),
                _ => "-1".into(),
            })
        });
        assert_eq!(junk, StreamConfig::default());
        let set = StreamConfig::from_vars(|name| {
            Some(match name {
                "ROBROZ_SSE_POLL_MS" => "500".into(),
                "ROBROZ_SSE_KEEPALIVE_SECS" => "30".into(),
                "ROBROZ_SSE_MAX_AGE_SECS" => "600".into(),
                _ => "8".into(),
            })
        });
        assert_eq!(
            set,
            StreamConfig {
                poll: Duration::from_millis(500),
                keep_alive: Duration::from_secs(30),
                max_age: Duration::from_secs(600),
                max_streams: 8,
            }
        );
    }

    #[test]
    fn a_slot_is_counted_until_dropped() {
        let open = Arc::new(AtomicUsize::new(0));
        let a = Slot::take(&open, 2).unwrap();
        let b = Slot::take(&open, 2).unwrap();
        assert!(Slot::take(&open, 2).is_none());
        assert_eq!(open.load(Ordering::SeqCst), 2, "a refused take is undone");
        drop(a);
        assert!(Slot::take(&open, 2).is_some());
        drop(b);
        assert_eq!(open.load(Ordering::SeqCst), 0);
    }
    use robroz_contract::{Event, MessageKind};

    fn msg(id: &str, at: &str) -> TimedEvent {
        TimedEvent {
            created_at: at.into(),
            changed_at: at.into(),
            id: id.into(),
            event: Event::Message {
                id: id.into(),
                kind: MessageKind::Text,
                body: String::new(),
                created_at: at.into(),
                author_name: None,
                author_user_id: None,
            },
        }
    }

    #[test]
    fn same_millisecond_rows_are_neither_lost_nor_repeated() {
        let t = "2026-11-20T02:00:00.000Z";
        let mut c = Cursor {
            at: None,
            sent_at_cursor: HashSet::new(),
        };
        assert_eq!(c.take_new(vec![msg("a", t)]).len(), 1);
        // A second row lands in the same millisecond after the first read.
        let again = c.take_new(vec![msg("a", t), msg("b", t)]);
        assert_eq!(
            again.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
        assert!(c.take_new(vec![msg("a", t), msg("b", t)]).is_empty());
        let later = c.take_new(vec![msg("a", t), msg("c", "2026-11-20T02:00:00.001Z")]);
        assert_eq!(later.len(), 1);
        assert_eq!(c.query_since().as_deref(), Some(t));
    }
}
