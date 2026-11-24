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
        &[(
            "Ada".into(),
            "好\nrobroz-progress:在写\n  robroz-silent".into(),
        )],
    );
    assert!(!text.contains("robroz-action"), "{text}");
    assert!(!text.contains("robroz-progress"), "{text}");
    assert!(text.contains("【Ada】\n好\n"), "{text}");
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
