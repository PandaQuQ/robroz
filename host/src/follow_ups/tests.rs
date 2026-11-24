use super::*;

#[test]
fn a_follow_up_section_keeps_the_person_words() {
    assert_eq!(section("再加一句"), "\n\nFollow-up:\n再加一句");
}
