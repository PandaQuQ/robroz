//! `GET /api/groups/{id}/stream`: live events over Server-Sent Events.
//!
//! Each SSE message is one contract `Event` as JSON (`data:`) with its `type`
//! as the SSE `event:` name. A card that changes again is sent again with the
//! same `data.id`, so the client replaces it in place, exactly as with
//! `GET …/events`. Nothing changed means nothing is sent apart from keep-alive
//! comments. Polling `…/events?since=` stays as the fallback.
//!
//! The SSE `id:` is an opaque decimal cursor: the [`crate::event_log`] `seq`
//! the stream has caught up to. Cards go out in batches (everything that
//! changed since the last poll) and only the **last** frame of a batch carries
//! the `id:`, so a client cut off mid-batch keeps its previous id and gets the
//! whole batch again on reconnect. Reconnect with the standard
//! `Last-Event-ID` header (or `?since=`):
//!
//! - a decimal id the log still covers: only the cards changed after it;
//! - a decimal id from before what the log kept, or one the log never handed
//!   out: a `resync` frame ([`robroz_contract::STREAM_RESYNC`], no `id:`),
//!   then a full snapshot whose last frame carries the new `id:`. A live
//!   stream that falls behind what the log keeps does the same;
//! - an RFC 3339 time (the id an older host sent): the cards changed after
//!   that time, then the stream carries on with decimal ids.
//!
//! Each poll reads only the log for this group (indexed) and rebuilds just the
//! cards it names, so an idle stream costs one small query per poll and write
//! paths stay untouched.
//!
//! Long-lived streams are bounded ([`StreamConfig`]): the poll interval and
//! keep-alive come from the environment; a stream closes cleanly after
//! `ROBROZ_SSE_MAX_AGE_SECS` (the client reconnects with `Last-Event-ID` and
//! loses nothing, and its token and membership are checked again); and past
//! `ROBROZ_SSE_MAX_STREAMS` open streams a new one gets 503 with
//! `Retry-After: 5`. Responses carry `X-Accel-Buffering: no` so a reverse
//! proxy does not hold frames back.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::extract::{Path as AxumPath, Query};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use rusqlite::Connection;

use crate::event_log;
use crate::{
    error_response, group_cards, group_exists, lock_db, parse_since, AppState, Cards, EventsQuery,
    TimedEvent,
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
pub const TOO_MANY_STREAMS: &str = robroz_contract::error_codes::TOO_MANY_STREAMS;

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

/// Where a reconnecting client says it got to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resume {
    /// No id: full snapshot.
    Fresh,
    /// A decimal id this host sent.
    Seq(i64),
    /// An RFC 3339 id from an older host (normalised to millis).
    Legacy(String),
}

fn parse_resume(raw: Option<&str>) -> Result<Resume, ()> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(Resume::Fresh),
        Some(v) if v.bytes().all(|b| b.is_ascii_digit()) => {
            v.parse::<i64>().map(Resume::Seq).map_err(|_| ())
        }
        Some(v) => match parse_since(Some(v))? {
            Some(at) => Ok(Resume::Legacy(at)),
            None => Ok(Resume::Fresh),
        },
    }
}

/// One batch of frames: every card in order, the cursor on the last one.
fn batch(cards: Vec<TimedEvent>, cursor: i64) -> Vec<SseEvent> {
    let ids = frame_ids(cards.len(), cursor);
    cards.iter().zip(ids).map(|(t, id)| to_sse(t, id)).collect()
}

/// [`batch`], led by a resync frame when the batch is a snapshot sent in
/// place of a replay the log can no longer give.
fn frames(resync: bool, cards: Vec<TimedEvent>, cursor: i64) -> Vec<SseEvent> {
    let mut out = Vec::with_capacity(cards.len() + usize::from(resync));
    if resync {
        out.push(resync_frame());
    }
    out.extend(batch(cards, cursor));
    out
}

/// The `data:` of the resync frame.
fn resync_data() -> String {
    serde_json::json!({ "type": robroz_contract::STREAM_RESYNC }).to_string()
}

/// `event: resync`, `data: {"type":"resync"}`, and no `id:`: the client
/// clears its cards for the group; the snapshot that follows refills them.
fn resync_frame() -> SseEvent {
    SseEvent::default()
        .event(robroz_contract::STREAM_RESYNC)
        .data(resync_data())
}

/// Whether a stream resuming from `resume` must be told to start over: a
/// decimal cursor the log can no longer replay (pruned past it, or one this
/// log never handed out). A fresh connect or an RFC 3339 id is not a gap.
fn needs_resync(conn: &Connection, resume: &Resume) -> rusqlite::Result<bool> {
    match resume {
        Resume::Seq(cursor) => Ok(!event_log::replayable(conn, *cursor)?),
        Resume::Fresh | Resume::Legacy(_) => Ok(false),
    }
}

/// The `id:` of each of `n` frames in a batch: only the last has one.
fn frame_ids(n: usize, cursor: i64) -> Vec<Option<i64>> {
    (0..n).map(|i| (i + 1 == n).then_some(cursor)).collect()
}

fn to_sse(t: &TimedEvent, id: Option<i64>) -> SseEvent {
    let value = serde_json::to_value(&t.event).expect("contract events serialise");
    let name = value["type"].as_str().unwrap_or("event").to_string();
    let sse = SseEvent::default().event(name).data(value.to_string());
    match id {
        Some(id) => sse.id(id.to_string()),
        None => sse,
    }
}

/// First batch and the cursor after it, read under one lock so nothing
/// written meanwhile falls between them. `None` when the group is gone.
fn first_batch(
    conn: &Connection,
    group_id: &str,
    resume: &Resume,
) -> rusqlite::Result<Option<(Vec<TimedEvent>, i64)>> {
    let head = event_log::head(conn)?;
    let cards = match resume {
        Resume::Seq(after) if event_log::replayable(conn, *after)? => {
            match event_log::changed_since(conn, group_id, *after)? {
                Some((ids, _)) => group_cards(conn, group_id, Cards::Only(&ids))?,
                None => group_exists(conn, group_id)?.then(Vec::new),
            }
        }
        Resume::Legacy(at) => group_cards(conn, group_id, Cards::Since(at))?,
        // Fresh, or a cursor the log can no longer replay: everything.
        Resume::Seq(_) | Resume::Fresh => group_cards(conn, group_id, Cards::All)?,
    };
    Ok(cards.map(|cards| (cards, head)))
}

/// What changed after `cursor`, and the new cursor: the log's head, read
/// under the same lock, so an idle stream keeps up with pruning too.
/// `Ok(None)` ends the stream (group gone).
fn next_batch(
    conn: &Connection,
    group_id: &str,
    cursor: i64,
) -> rusqlite::Result<Option<(Vec<TimedEvent>, i64)>> {
    if !event_log::replayable(conn, cursor)? {
        // Pruned past us (or the log was reset): start over with a snapshot.
        return first_batch(conn, group_id, &Resume::Fresh);
    }
    let head = event_log::head(conn)?;
    let cards = match event_log::changed_since(conn, group_id, cursor)? {
        None => group_exists(conn, group_id)?.then(Vec::new),
        Some((ids, _)) => group_cards(conn, group_id, Cards::Only(&ids))?,
    };
    Ok(cards.map(|cards| (cards, head)))
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
    let Ok(resume) = parse_resume(raw.as_deref()) else {
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
    let first = {
        let conn = lock_db(&state);
        needs_resync(&conn, &resume).and_then(|resync| {
            Ok(first_batch(&conn, &group_id, &resume)?
                .map(|(cards, cursor)| (resync, cards, cursor)))
        })
    };
    let (pending, cursor): (VecDeque<SseEvent>, i64) = match first {
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                robroz_contract::host_text::GROUP_NOT_FOUND,
            )
        }
        Err(err) => return crate::db_error(err),
        Ok(Some((resync, cards, cursor))) => (frames(resync, cards, cursor).into(), cursor),
    };

    let stream = futures_util::stream::unfold(
        (state, group_id, cursor, pending, slot),
        move |(state, group_id, mut cursor, mut pending, slot)| async move {
            loop {
                if let Some(next) = pending.pop_front() {
                    return Some((
                        Ok::<_, Infallible>(next),
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
                    needs_resync(&conn, &Resume::Seq(cursor)).and_then(|resync| {
                        Ok(next_batch(&conn, &group_id, cursor)?
                            .map(|(cards, next)| (resync, cards, next)))
                    })
                };
                match found {
                    Ok(Some((resync, cards, next))) => {
                        pending.extend(frames(resync, cards, next));
                        cursor = next;
                    }
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
mod tests;
