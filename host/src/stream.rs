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
//!   out: a full snapshot, as on a fresh connect;
//! - an RFC 3339 time (the id an older host sent): the cards changed after
//!   that time, then the stream carries on with decimal ids.
//!
//! Each poll reads only the log for this group (indexed) and rebuilds just the
//! cards it names, so an idle stream costs one small query per poll and write
//! paths stay untouched.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::Duration;

use crate::extract::{Path as AxumPath, Query};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use rusqlite::Connection;

use crate::event_log;
use crate::{
    error_response, group_cards, group_exists, lock_db, parse_since, AppState, Cards, EventsQuery,
    TimedEvent,
};

pub(crate) const POLL: Duration = Duration::from_millis(250);
const KEEP_ALIVE: Duration = Duration::from_secs(15);

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
    let first = {
        let conn = lock_db(&state);
        first_batch(&conn, &group_id, &resume)
    };
    let (pending, cursor): (VecDeque<SseEvent>, i64) = match first {
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                robroz_contract::host_text::GROUP_NOT_FOUND,
            )
        }
        Err(err) => return crate::db_error(err),
        Ok(Some((cards, cursor))) => (batch(cards, cursor).into(), cursor),
    };

    let stream = futures_util::stream::unfold(
        (state, group_id, cursor, pending),
        |(state, group_id, mut cursor, mut pending)| async move {
            loop {
                if let Some(next) = pending.pop_front() {
                    return Some((
                        Ok::<_, Infallible>(next),
                        (state, group_id, cursor, pending),
                    ));
                }
                tokio::time::sleep(POLL).await;
                let found = {
                    let conn = lock_db(&state);
                    next_batch(&conn, &group_id, cursor)
                };
                match found {
                    Ok(Some((cards, next))) => {
                        pending.extend(batch(cards, next));
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
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response()
}

#[cfg(test)]
mod tests;
