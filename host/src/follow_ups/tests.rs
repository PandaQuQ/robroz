use super::*;

#[test]
fn a_follow_up_section_keeps_the_person_words() {
    assert_eq!(section("再加一句"), "\n\nFollow-up:\n再加一句");
}

/// PR-C3: `startedAt` is per round. A follow-up that queues a done task
/// again clears it, so the next claim records the new round's start; the
/// card's `createdAt` (identity and order) stays.
#[test]
fn a_follow_up_requeue_clears_started_at_and_keeps_created_at() {
    use crate::event_log::tests::{db, T0};
    let conn = db();
    conn.execute(
        "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, started_at)
         VALUES ('t1', 'g1', 'b1', 'do', 'done', ?1, '2026-11-27T00:00:09.000Z', '2026-11-27T00:00:01.000Z')",
        [T0],
    )
    .unwrap();
    attach(&conn, "t1", "m1", "one more", "2026-11-27T00:00:10.000Z").unwrap();
    assert!(
        continue_after_done(&conn, "t1", Some("first reply"), "2026-11-27T00:00:11.000Z").unwrap()
    );
    let (state, created_at, started_at): (String, String, Option<String>) = conn
        .query_row(
            "SELECT state, created_at, started_at FROM tasks WHERE id = 't1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "queued");
    assert_eq!(created_at, T0);
    assert_eq!(started_at, None);
}
