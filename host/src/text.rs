//! Small text helpers shared by cards.

/// The first `keep` characters (CJK counts one each), plus "…" when anything
/// was cut. Never fails.
pub(crate) fn clip(text: &str, keep: usize) -> String {
    let mut chars = text.chars();
    let short: String = chars.by_ref().take(keep).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_counts_chars_and_marks_the_cut() {
        assert_eq!(clip("周报", 2), "周报");
        assert_eq!(clip("周报写好了", 2), "周报…");
        assert_eq!(clip("", 3), "");
    }
}
