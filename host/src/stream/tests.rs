use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::event_log::tests::{db, seed_every_card, T0};

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

fn ids(cards: &[TimedEvent]) -> BTreeSet<String> {
    cards.iter().map(|t| t.id.clone()).collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

#[test]
fn resume_ids_are_decimal_or_an_older_hosts_time() {
    assert_eq!(parse_resume(None), Ok(Resume::Fresh));
    assert_eq!(parse_resume(Some(" ")), Ok(Resume::Fresh));
    assert_eq!(parse_resume(Some("42")), Ok(Resume::Seq(42)));
    assert_eq!(
        parse_resume(Some("2026-11-27T08:00:00+08:00")),
        Ok(Resume::Legacy("2026-11-27T00:00:00.000Z".into()))
    );
    assert_eq!(parse_resume(Some("not-a-time")), Err(()));
    assert_eq!(parse_resume(Some("-1")), Err(()));
    assert_eq!(parse_resume(Some("99999999999999999999999")), Err(()));
}

/// Only the last frame of a batch carries the cursor.
#[test]
fn only_the_last_frame_of_a_batch_carries_the_id() {
    assert_eq!(frame_ids(3, 9), vec![None, None, Some(9)]);
    assert_eq!(frame_ids(1, 9), vec![Some(9)]);
    assert!(frame_ids(0, 9).is_empty(), "nothing changed, nothing sent");
}

#[test]
fn fresh_connect_is_a_full_snapshot_and_the_cursor_is_the_head() {
    let conn = db();
    seed_every_card(&conn);
    let (cards, cursor) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    assert_eq!(
        ids(&cards),
        set(&["m1", "t1", "d1", "c1", "a1", "p1", "p2"])
    );
    assert_eq!(cursor, event_log::head(&conn).unwrap());
    assert!(first_batch(&conn, "missing", &Resume::Fresh)
        .unwrap()
        .is_none());
}

#[test]
fn reconnect_replays_only_what_changed_after_the_id() {
    let conn = db();
    seed_every_card(&conn);
    let (_, cursor) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m2', 'g1', 'text', 'x', ?1)",
        ["2026-11-27T00:00:05.000Z"],
    )
    .unwrap();
    conn.execute("UPDATE tasks SET state = 'running' WHERE id = 't1'", [])
        .unwrap();
    let (cards, next) = first_batch(&conn, "g1", &Resume::Seq(cursor))
        .unwrap()
        .unwrap();
    assert_eq!(ids(&cards), set(&["m2", "t1"]));
    assert!(next > cursor);
    // Caught up: nothing more.
    let (cards, same) = next_batch(&conn, "g1", next).unwrap().unwrap();
    assert!(cards.is_empty());
    assert_eq!(same, next);
}

/// The bug the log fixes: a timestamp cursor skips a card whose
/// `updated_at` is older than the cursor (clock step back, `newer_than`
/// on another row running ahead). The seq cursor does not.
#[test]
fn a_card_whose_updated_at_is_older_than_the_cursor_is_still_delivered() {
    let conn = db();
    seed_every_card(&conn);
    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('late', 'g1', 'text', 'x', ?1)",
        ["2026-11-27T09:00:00.000Z"],
    )
    .unwrap();
    let (_, cursor) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    let time_cursor = "2026-11-27T09:00:00.000Z";
    // Written after the cursor, stamped before it.
    conn.execute(
        "UPDATE tasks SET state = 'running', updated_at = ?1 WHERE id = 't1'",
        ["2026-11-27T08:59:59.999Z"],
    )
    .unwrap();
    let old = crate::group_events(&conn, "g1", Some(time_cursor))
        .unwrap()
        .unwrap();
    assert!(
        !ids(&old).contains("t1"),
        "the timestamp cursor misses it (why the log exists)"
    );
    let (cards, _) = next_batch(&conn, "g1", cursor).unwrap().unwrap();
    assert_eq!(ids(&cards), set(&["t1"]));
}

/// A client cut off mid-batch still holds the previous batch's id (only
/// the last frame carries one), so a reconnect sends the whole batch again.
#[test]
fn a_drop_mid_batch_resends_the_whole_batch() {
    let conn = db();
    seed_every_card(&conn);
    let (_, before) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    conn.execute_batch(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m2', 'g1', 'text', 'a', '2026-11-27T00:00:01.000Z');
         INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m3', 'g1', 'text', 'b', '2026-11-27T00:00:02.000Z');
         INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m4', 'g1', 'text', 'c', '2026-11-27T00:00:03.000Z');",
    )
    .unwrap();
    let (cards, after) = next_batch(&conn, "g1", before).unwrap().unwrap();
    assert_eq!(cards.len(), 3);
    let frames = batch(cards, after);
    assert_eq!(frames.len(), 3);
    // The client read `m2` and `m3` (no id on either) and dropped: it
    // reconnects with `before`.
    let (again, _) = first_batch(&conn, "g1", &Resume::Seq(before))
        .unwrap()
        .unwrap();
    assert_eq!(ids(&again), set(&["m2", "m3", "m4"]));
}

#[test]
fn an_older_hosts_time_id_replays_by_time_then_switches_to_seq() {
    let conn = db();
    seed_every_card(&conn);
    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m2', 'g1', 'text', 'x', ?1)",
        ["2026-11-27T00:00:05.000Z"],
    )
    .unwrap();
    let (cards, cursor) = first_batch(&conn, "g1", &Resume::Legacy(T0.into()))
        .unwrap()
        .unwrap();
    assert_eq!(ids(&cards), set(&["m2"]));
    assert_eq!(cursor, event_log::head(&conn).unwrap());
}

#[test]
fn an_id_the_log_no_longer_covers_gets_a_full_snapshot() {
    let conn = db();
    seed_every_card(&conn);
    let (_, old) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m2', 'g1', 'text', 'x', ?1)",
        [T0],
    )
    .unwrap();
    let later = chrono::Utc::now() + chrono::Duration::hours(1);
    event_log::maintain(&conn, later, Duration::from_secs(1)).unwrap();
    let all = set(&["m1", "m2", "t1", "d1", "c1", "a1", "p1", "p2"]);
    let (cards, cursor) = first_batch(&conn, "g1", &Resume::Seq(old))
        .unwrap()
        .unwrap();
    assert_eq!(ids(&cards), all, "below the floor");
    // A live stream that falls behind the floor starts over the same way.
    let (cards, _) = next_batch(&conn, "g1", old).unwrap().unwrap();
    assert_eq!(ids(&cards), all);
    // An id from the future (the database was replaced) too.
    let (cards, _) = first_batch(&conn, "g1", &Resume::Seq(cursor + 100))
        .unwrap()
        .unwrap();
    assert_eq!(ids(&cards), all);
}

#[test]
fn a_hidden_turn_or_another_groups_change_sends_nothing() {
    let conn = db();
    seed_every_card(&conn);
    let (_, cursor) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    conn.execute(
        "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, turn_hidden)
         VALUES ('hidden', 'g1', 'b1', 'p', 'running', ?1, ?1, 1)",
        [T0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('dm-m', 'dm1', 'text', 'x', ?1)",
        [T0],
    )
    .unwrap();
    let (cards, next) = next_batch(&conn, "g1", cursor).unwrap().unwrap();
    assert!(cards.is_empty(), "{:?}", ids(&cards));
    assert_eq!(next, event_log::head(&conn).unwrap(), "cursor still moves");
}

#[test]
fn a_deleted_group_ends_the_stream() {
    let conn = db();
    seed_every_card(&conn);
    let (_, cursor) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    crate::groups::delete_rows(&conn, "g1", "g").unwrap();
    assert!(next_batch(&conn, "g1", cursor).unwrap().is_none());
}

/// Lock-hold time per poll, old (rebuild the whole group, filter by time)
/// against new (read the log, rebuild only changed cards), for a group with
/// 5000 messages and 10 open streams. Prints the numbers; run with
/// `cargo test -p robroz-host --release -- --ignored --nocapture poll_cost`.
#[test]
#[ignore = "timing; run by hand"]
fn poll_cost_with_5k_messages_and_10_streams() {
    const STREAMS: usize = 10;
    const POLLS: usize = 20;
    let conn = db();
    seed_every_card(&conn);
    let tx = conn.unchecked_transaction().unwrap();
    for i in 0..5000 {
        tx.execute(
            "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES (?1, 'g1', 'text', ?2, ?3)",
            rusqlite::params![
                format!("bulk-{i:05}"),
                format!("message number {i} with some text in it"),
                format!("2026-11-26T{:02}:{:02}:{:02}.000Z", i / 3600 % 24, i / 60 % 60, i % 60)
            ],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let since = "2026-11-27T01:00:00.000Z";
    let cursor = event_log::head(&conn).unwrap();

    let time = |f: &mut dyn FnMut()| {
        let start = Instant::now();
        for _ in 0..STREAMS * POLLS {
            f();
        }
        start.elapsed() / (STREAMS * POLLS) as u32
    };
    // Idle: nothing changed since the cursor.
    let old_idle = time(&mut || {
        crate::group_events(&conn, "g1", Some(since)).unwrap();
    });
    let new_idle = time(&mut || {
        next_batch(&conn, "g1", cursor).unwrap();
    });
    // Busy: one status card changed since the cursor (a progress line).
    conn.execute(
        "UPDATE tasks SET state = 'running', updated_at = '2026-11-27T02:00:00.000Z' WHERE id = 't1'",
        [],
    )
    .unwrap();
    let old_busy = time(&mut || {
        crate::group_events(&conn, "g1", Some(since)).unwrap();
    });
    let new_busy = time(&mut || {
        next_batch(&conn, "g1", cursor).unwrap();
    });
    let per_sec = |d: Duration| d.as_secs_f64() * 1000.0 * STREAMS as f64 * 4.0;
    println!("poll cost per stream, 5000 messages:");
    println!(
        "  idle: old {old_idle:?}, new {new_idle:?}; lock held per second with {STREAMS} streams at 250ms: old {:.1} ms, new {:.2} ms",
        per_sec(old_idle),
        per_sec(new_idle)
    );
    println!(
        "  one card changed: old {old_busy:?}, new {new_busy:?}; per second: old {:.1} ms, new {:.2} ms",
        per_sec(old_busy),
        per_sec(new_busy)
    );
    assert!(new_idle < old_idle && new_busy < old_busy);
}

/// PR-C3: a cursor the log can no longer replay is told to start over
/// before the snapshot; any other resume is not.
#[test]
fn only_a_cursor_the_log_cannot_replay_needs_a_resync() {
    let conn = db();
    seed_every_card(&conn);
    let (_, old) = first_batch(&conn, "g1", &Resume::Fresh).unwrap().unwrap();
    assert!(!needs_resync(&conn, &Resume::Fresh).unwrap());
    assert!(!needs_resync(&conn, &Resume::Legacy(T0.into())).unwrap());
    assert!(
        !needs_resync(&conn, &Resume::Seq(old)).unwrap(),
        "replayable"
    );
    assert!(
        !needs_resync(&conn, &Resume::Seq(0)).unwrap(),
        "nothing pruned yet"
    );
    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m2', 'g1', 'text', 'x', ?1)",
        [T0],
    )
    .unwrap();
    let later = chrono::Utc::now() + chrono::Duration::hours(1);
    event_log::maintain(&conn, later, Duration::from_secs(1)).unwrap();
    let floor = event_log::pruned_through(&conn).unwrap();
    assert!(floor > old);
    assert!(
        needs_resync(&conn, &Resume::Seq(old)).unwrap(),
        "below the floor"
    );
    assert!(
        !needs_resync(&conn, &Resume::Seq(floor)).unwrap(),
        "at the floor"
    );
    let head = event_log::head(&conn).unwrap();
    assert!(
        needs_resync(&conn, &Resume::Seq(head + 100)).unwrap(),
        "an id this log never handed out"
    );
}

#[test]
fn the_resync_frame_data_is_just_its_type() {
    let data: serde_json::Value = serde_json::from_str(&resync_data()).unwrap();
    assert_eq!(data, serde_json::json!({ "type": "resync" }));
    assert_eq!(robroz_contract::STREAM_RESYNC, "resync");
}

/// PR-C3: a status card carries `startedAt` only once the column is set.
#[test]
fn status_cards_carry_started_at_only_when_set() {
    let conn = db();
    seed_every_card(&conn);
    let status = |conn: &Connection| -> serde_json::Value {
        let (cards, _) = first_batch(conn, "g1", &Resume::Fresh).unwrap().unwrap();
        let t = cards.into_iter().find(|t| t.id == "t1").unwrap();
        serde_json::to_value(&t.event).unwrap()
    };
    assert!(status(&conn).get("startedAt").is_none(), "queued: absent");
    conn.execute(
        "UPDATE tasks SET state = 'running', started_at = '2026-11-27T00:00:01.000Z' WHERE id = 't1'",
        [],
    )
    .unwrap();
    assert_eq!(status(&conn)["startedAt"], "2026-11-27T00:00:01.000Z");
}
