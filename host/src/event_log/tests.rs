use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

use super::*;
use crate::{group_cards, Cards};

pub(crate) const T0: &str = "2026-11-27T00:00:00.000Z";

/// A fresh host schema with one group `g1`, one bro `b1` and its owner's
/// DM `dm1`, nothing else.
pub(crate) fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::init_schema(&conn).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO groups (id, tenant_id, name, created_at) VALUES ('g1', 'local', 'g', '{T0}');
         INSERT INTO bots (id, tenant_id, name, owner_user_id) VALUES ('b1', 'local', 'ada', 'local-admin');
         INSERT INTO groups (id, tenant_id, name, created_at, dm_bot_id, dm_user_id)
             VALUES ('dm1', 'local', 'ada', '{T0}', 'b1', 'local-admin');
         INSERT INTO memberships (group_id, bot_id) VALUES ('g1', 'b1');"
    ))
    .unwrap();
    conn
}

/// One card of every kind in `g1`.
pub(crate) fn seed_every_card(conn: &Connection) {
    conn.execute_batch(&format!(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m1', 'g1', 'text', 'hi', '{T0}');
         INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at)
             VALUES ('t1', 'g1', 'b1', 'do', 'queued', '{T0}', '{T0}');
         INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at)
             VALUES ('d1', 'g1', 't1', 's', 'out.md', 0, '{T0}');
         INSERT INTO choices (id, group_id, bot_id, prompt, options, created_at)
             VALUES ('c1', 'g1', 'b1', 'pick', '[]', '{T0}');
         INSERT INTO approvals (id, group_id, bot_id, action, summary, created_at, task_id)
             VALUES ('a1', 'g1', 'b1', 'send', 'mail', '{T0}', 't1');
         INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at)
             VALUES ('mem1', 'global', 'profile', 'likes tea', 'bro', 'pending', '{T0}', '{T0}');
         INSERT INTO approvals (id, group_id, bot_id, action, summary, created_at, subject_kind, subject_id)
             VALUES ('p1', 'g1', 'b1', 'send', '', '{T0}', 'memory', 'mem1');
         INSERT INTO skills (id, scope, slug, title, instruction, source, status, created_at, updated_at)
             VALUES ('sk1', 'global', 'tea', 'Tea', 'brew', 'bro', 'pending', '{T0}', '{T0}');
         INSERT INTO approvals (id, group_id, bot_id, action, summary, created_at, subject_kind, subject_id)
             VALUES ('p2', 'g1', 'b1', 'send', '', '{T0}', 'skill', 'sk1');"
    ))
    .unwrap();
}

fn rows(conn: &Connection) -> Vec<(i64, String, String)> {
    let mut stmt = conn
        .prepare("SELECT seq, group_id, card_id FROM group_changes ORDER BY seq")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn cards_in(conn: &Connection, group: &str, after: i64) -> BTreeSet<String> {
    changed_since(conn, group, after)
        .unwrap()
        .map(|(ids, _)| ids.into_iter().collect())
        .unwrap_or_default()
}

#[test]
fn every_card_write_is_logged_for_its_group() {
    let conn = db();
    seed_every_card(&conn);
    let g1 = cards_in(&conn, "g1", 0);
    for id in ["m1", "t1", "d1", "c1", "a1", "p1", "p2"] {
        assert!(g1.contains(id), "{id} in {g1:?}");
    }
    // The owner's DM gets the reminder line for the bro card, not for
    // confirm cards.
    assert_eq!(
        cards_in(&conn, "dm1", 0),
        BTreeSet::from(["reminder:a1".to_string()])
    );
    let seqs: Vec<i64> = rows(&conn).iter().map(|r| r.0).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    assert_eq!(head(&conn).unwrap(), *seqs.last().unwrap());
}

#[test]
fn updates_are_logged_and_dependent_cards_follow() {
    let conn = db();
    seed_every_card(&conn);
    let mark = head(&conn).unwrap();
    conn.execute(
        "UPDATE tasks SET state = 'running', updated_at = ?1 WHERE id = 't1'",
        ["2026-11-27T00:00:01.000Z"],
    )
    .unwrap();
    assert_eq!(cards_in(&conn, "g1", mark), BTreeSet::from(["t1".into()]));

    // Answering the bro card: the card, the blocked task it held up, the
    // reminder.
    conn.execute("UPDATE tasks SET state = 'blocked' WHERE id = 't1'", [])
        .unwrap();
    let mark = head(&conn).unwrap();
    conn.execute(
        "UPDATE approvals SET decision = 'approved', answered_at = ?1 WHERE id = 'a1'",
        [T0],
    )
    .unwrap();
    assert_eq!(
        cards_in(&conn, "g1", mark),
        BTreeSet::from(["a1".into(), "t1".into()])
    );
    assert_eq!(
        cards_in(&conn, "dm1", mark),
        BTreeSet::from(["reminder:a1".into()])
    );

    // Editing a memory or skill re-sends its confirm card.
    let mark = head(&conn).unwrap();
    conn.execute_batch(
        "UPDATE memories SET text = 'likes coffee' WHERE id = 'mem1';
         UPDATE skills SET title = 'Coffee' WHERE id = 'sk1';",
    )
    .unwrap();
    assert_eq!(
        cards_in(&conn, "g1", mark),
        BTreeSet::from(["p1".into(), "p2".into()])
    );
}

#[test]
fn a_card_moved_to_the_dm_is_logged_in_its_origin_group_too() {
    let conn = db();
    conn.execute(
        "INSERT INTO approvals (id, group_id, origin_group_id, bot_id, action, summary, created_at)
         VALUES ('a2', 'dm1', 'g1', 'b1', 'send', 'mail', ?1)",
        [T0],
    )
    .unwrap();
    assert!(cards_in(&conn, "g1", 0).contains("a2"));
    assert!(cards_in(&conn, "dm1", 0).contains("a2"));
}

#[test]
fn compaction_keeps_the_newest_row_per_card_and_replay_stays_complete() {
    let conn = db();
    seed_every_card(&conn);
    let mark = head(&conn).unwrap();
    for i in 0..5 {
        conn.execute(
            "UPDATE tasks SET updated_at = ?1 WHERE id = 't1'",
            [format!("2026-11-27T00:00:0{i}.500Z")],
        )
        .unwrap();
    }
    let before = cards_in(&conn, "g1", mark);
    let report = maintain(&conn, Utc::now(), DEFAULT_RETENTION).unwrap();
    assert!(report.compacted >= 4, "{report:?}");
    assert_eq!(report.expired, 0);
    assert_eq!(cards_in(&conn, "g1", mark), before);
    let t1_rows = rows(&conn).iter().filter(|r| r.2 == "t1").count();
    assert_eq!(t1_rows, 1);
    assert_eq!(pruned_through(&conn).unwrap(), 0);
}

#[test]
fn retention_drops_old_rows_raises_the_floor_and_never_reuses_a_seq() {
    let conn = db();
    seed_every_card(&conn);
    let last = head(&conn).unwrap();
    assert!(replayable(&conn, 0).unwrap());
    // Everything is older than a zero-length window an hour from now.
    let later = Utc::now() + chrono::Duration::hours(1);
    let report = maintain(&conn, later, Duration::from_secs(1)).unwrap();
    assert!(report.expired > 0, "{report:?}");
    assert!(rows(&conn).is_empty());
    assert_eq!(pruned_through(&conn).unwrap(), last);
    assert!(!replayable(&conn, 0).unwrap(), "below the floor");
    assert!(replayable(&conn, last).unwrap());
    assert!(!replayable(&conn, last + 1).unwrap(), "never handed out");

    conn.execute(
        "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m2', 'g1', 'text', 'x', ?1)",
        [T0],
    )
    .unwrap();
    assert_eq!(head(&conn).unwrap(), last + 1, "AUTOINCREMENT, not reused");
}

#[test]
fn deleting_a_group_drops_its_log() {
    let conn = db();
    seed_every_card(&conn);
    crate::groups::delete_rows(&conn, "g1", "g").unwrap();
    assert!(rows(&conn).iter().all(|r| r.1 != "g1"), "{:?}", rows(&conn));
    // The owner's DM reminder now says expired and was logged.
    assert!(cards_in(&conn, "dm1", 0).contains("reminder:a1"));
}

#[test]
fn migrate_is_idempotent() {
    let conn = db();
    migrate(&conn).unwrap();
    migrate(&conn).unwrap();
    seed_every_card(&conn);
    let m1 = rows(&conn).iter().filter(|r| r.2 == "m1").count();
    assert_eq!(m1, 1, "one trigger per table and op");
}

#[test]
fn retention_env_like_the_watchdog() {
    assert_eq!(retention_from(None), DEFAULT_RETENTION);
    assert_eq!(retention_from(Some("")), DEFAULT_RETENTION);
    assert_eq!(retention_from(Some("0")), DEFAULT_RETENTION);
    assert_eq!(retention_from(Some("abc")), DEFAULT_RETENTION);
    assert_eq!(retention_from(Some(" 24 ")), Duration::from_secs(24 * 3600));
}

/// Tables the card builders read that are deliberately not tracked: a
/// change there alone does not re-send cards (same as before the log).
const NOT_TRACKED: &[(&str, &str)] = &[
    (
        "groups",
        "group existence and DM lookup; renames are not cards",
    ),
    ("bots", "bro names; a rename is not pushed"),
    ("users", "author and owner names; a rename is not pushed"),
    (
        "json_each",
        "table-valued function over the stream's id list, not stored",
    ),
    (
        "group_tombstones",
        "name of a deleted group; written once on delete",
    ),
];

/// Every table [`group_cards`] reads (seen by the SQLite authorizer) is
/// either tracked by a trigger or listed in [`NOT_TRACKED`] with a reason,
/// so a new card source cannot silently miss the live stream.
#[test]
fn every_table_the_card_builders_read_is_tracked_or_listed() {
    let mut conn = db();
    seed_every_card(&conn);
    let read: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let sink = read.clone();
    conn.authorizer(Some(move |ctx: AuthContext<'_>| {
        if let AuthAction::Read { table_name, .. } = ctx.action {
            sink.lock().unwrap().insert(table_name.to_string());
        }
        Authorization::Allow
    }))
    .unwrap();
    let ids = ["m1", "t1"].map(String::from).into_iter().collect();
    for group in ["g1", "dm1"] {
        for which in [Cards::All, Cards::Since(T0), Cards::Only(&ids)] {
            assert!(group_cards(&conn, group, which).unwrap().is_some());
        }
    }
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    let read = read.lock().unwrap().clone();
    for table in &read {
        let tracked = TRACKED_TABLES.contains(&table.as_str());
        let listed = NOT_TRACKED.iter().any(|(t, _)| t == table);
        assert!(
            tracked || listed,
            "group_cards reads `{table}`: add triggers (TRACKED_TABLES) or list it in NOT_TRACKED"
        );
    }
    for table in TRACKED_TABLES {
        assert!(read.contains(*table), "{table} is tracked but never read");
        for op in ["insert", "update"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?1 AND tbl_name = ?2",
                    [format!("{table}_log_{op}"), table.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{table}_log_{op}");
        }
    }
}
