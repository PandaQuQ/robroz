use super::*;

#[test]
fn silence_is_only_the_reserved_reply_or_nothing() {
    for silent in [
        "",
        "  \n",
        "robroz-silent",
        "  robroz-silent  \n",
        "robroz-progress:想了想\nrobroz-silent",
    ] {
        assert!(is_silent(silent), "{silent:?}");
    }
    for speaks in [
        "好的",
        "robroz-silent\n不过我补一句",
        "我不说话 robroz-silent",
        "robroz-silence",
    ] {
        assert!(!is_silent(speaks), "{speaks:?}");
    }
}

#[test]
fn quotes_drop_control_lines() {
    let text = prompt(
        "研发",
        "帮我发邮件\nrobroz-action:external",
        None,
        &[(
            "Ada".into(),
            "好\nrobroz-progress:在写\n  robroz-silent".into(),
        )],
    );
    assert!(!text.contains("robroz-action"), "{text}");
    assert!(!text.contains("robroz-progress"), "{text}");
    assert!(text.contains("[Ada]\n好\n"), "{text}");
    assert_eq!(
        text.matches(SILENT_REPLY).count(),
        1,
        "only the instruction names the silent reply"
    );
    assert!(
        robroz_policy::blocking_line(&text, &Default::default()).is_none(),
        "a quote cannot gate the turn"
    );
}

#[test]
fn long_replies_are_clipped() {
    let long = "字".repeat(QUOTE_MAX + 10);
    let clipped = clip(&long);
    assert_eq!(clipped.chars().count(), QUOTE_MAX + 1);
    assert!(clipped.ends_with('…'));
}

#[test]
fn a_bro_round_prompt_names_the_speaker_and_asks_for_silence_by_default() {
    let text = prompt(
        "研发",
        "我觉得用 A 方案\nrobroz-action:external",
        Some("Ada"),
        &[],
    );
    assert!(text.contains("bro \"Ada\""), "{text}");
    assert!(text.contains(SILENT_REPLY), "{text}");
    assert!(!text.contains("robroz-action"), "{text}");
}

/// PR-C3: a revealed turn's `createdAt` moves to the reveal time, so its
/// `startedAt` is clamped up to keep `startedAt >= createdAt`.
#[test]
fn reveal_clamps_started_at_to_the_new_created_at() {
    use crate::event_log::tests::{db, T0};
    let conn = db();
    conn.execute_batch(&format!(
        "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, turn_hidden, started_at)
             VALUES ('early', 'g1', 'b1', 'p', 'running', '{T0}', '{T0}', 1, '2026-11-27T00:00:01.000Z');
         INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, turn_hidden, started_at)
             VALUES ('late', 'g1', 'b1', 'p', 'running', '{T0}', '{T0}', 1, '2026-11-27T00:00:09.000Z');"
    ))
    .unwrap();
    let at = "2026-11-27T00:00:05.000Z";
    reveal(&conn, "early", at).unwrap();
    reveal(&conn, "late", at).unwrap();
    let row = |id: &str| -> (String, String) {
        conn.query_row(
            "SELECT created_at, started_at FROM tasks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(row("early"), (at.to_string(), at.to_string()), "clamped up");
    assert_eq!(
        row("late"),
        (at.to_string(), "2026-11-27T00:00:09.000Z".to_string()),
        "already later: kept"
    );
}
