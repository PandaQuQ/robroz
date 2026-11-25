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

use std::collections::{HashSet, VecDeque};
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};

use crate::{
    error_response, group_events, lock_db, parse_since, AppState, EventsQuery, TimedEvent,
};

pub(crate) const POLL: Duration = Duration::from_millis(250);
const KEEP_ALIVE: Duration = Duration::from_secs(15);

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
        (state, group_id, cursor, pending),
        |(state, group_id, mut cursor, mut pending)| async move {
            loop {
                if let Some(next) = pending.pop_front() {
                    let sse = to_sse(&next);
                    return Some((Ok::<_, Infallible>(sse), (state, group_id, cursor, pending)));
                }
                tokio::time::sleep(POLL).await;
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
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
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
