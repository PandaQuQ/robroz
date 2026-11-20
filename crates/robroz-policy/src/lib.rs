/// Pure decisions. This crate does not start containers or touch the clock.

use std::collections::HashSet;

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outbound {
    Allow,
    Fail,
}

/// `used >= limit` is over the limit. Only GET under the limit is allowed.
pub fn outbound(method: &str, used: u64, limit: u64) -> Outbound {
    if method == "GET" && used < limit {
        Outbound::Allow
    } else {
        Outbound::Fail
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffect {
    WorkspaceRead,
    WorkspaceWrite,
    ExternalWrite,
    Captcha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffectDecision {
    Allow,
    BlockAndDropContainer,
}

pub fn side_effect(effect: SideEffect) -> SideEffectDecision {
    match effect {
        SideEffect::WorkspaceRead | SideEffect::WorkspaceWrite => SideEffectDecision::Allow,
        SideEffect::ExternalWrite | SideEffect::Captcha => SideEffectDecision::BlockAndDropContainer,
    }
}

pub fn should_start_container(due: bool, already_running: bool) -> bool {
    due && !already_running
}

/// Kind of a blocking action line, or None. Matching ignores surrounding spaces.
pub fn blocking_kind(line: &str) -> Option<&'static str> {
    match line.trim() {
        "robroz-action:external" => Some("external"),
        "robroz-action:captcha" => Some("captcha"),
        _ => None,
    }
}

/// Lowercase hex sha256 of the exact line bytes (no newline).
pub fn action_hash(line: &str) -> String {
    Sha256::digest(line.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Where a task stops: a blocking action line nobody has taken over yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingLine {
    pub kind: &'static str,
    /// The exact line from the prompt (surrounding spaces kept).
    pub line: String,
    /// [`action_hash`] of `line`.
    pub hash: String,
}

/// The first `robroz-action:external` / `robroz-action:captcha` line of `prompt`, in
/// order, whose sha256 is not in `approved`. None when every blocking line has
/// been taken over (or there are none).
///
/// The runner uses this to decide where to stop; the host uses it after the
/// container exits, from the stored prompt and the takeover hashes, to know where
/// it stopped. Identical lines share a hash, so one takeover approves all of them.
pub fn blocking_line(prompt: &str, approved: &HashSet<String>) -> Option<BlockingLine> {
    prompt.lines().find_map(|line| {
        let kind = blocking_kind(line)?;
        let hash = action_hash(line);
        if approved.contains(&hash) {
            return None;
        }
        Some(BlockingLine {
            kind,
            line: line.to_string(),
            hash,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_rules() {
        assert_eq!(outbound("GET", 0, 1), Outbound::Allow);
        assert_eq!(outbound("GET", 1, 2), Outbound::Allow);
        assert_eq!(outbound("GET", 1, 1), Outbound::Fail);
        assert_eq!(outbound("GET", 3, 1), Outbound::Fail);
        assert_eq!(outbound("POST", 0, 10), Outbound::Fail);
        assert_eq!(outbound("PUT", 0, 10), Outbound::Fail);
        assert_eq!(outbound("get", 0, 10), Outbound::Fail);
    }

    #[test]
    fn side_effect_rules() {
        assert_eq!(side_effect(SideEffect::WorkspaceRead), SideEffectDecision::Allow);
        assert_eq!(side_effect(SideEffect::WorkspaceWrite), SideEffectDecision::Allow);
        assert_eq!(
            side_effect(SideEffect::ExternalWrite),
            SideEffectDecision::BlockAndDropContainer
        );
        assert_eq!(
            side_effect(SideEffect::Captcha),
            SideEffectDecision::BlockAndDropContainer
        );
    }

    #[test]
    fn should_start_container_only_when_due_and_idle() {
        assert!(should_start_container(true, false));
        assert!(!should_start_container(true, true));
        assert!(!should_start_container(false, false));
        assert!(!should_start_container(false, true));
    }

    fn set(lines: &[&str]) -> HashSet<String> {
        lines.iter().map(|line| action_hash(line)).collect()
    }

    #[test]
    fn action_hash_is_sha256_hex() {
        assert_eq!(
            action_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn blocking_line_none_without_blocking_actions() {
        assert_eq!(blocking_line("", &HashSet::new()), None);
        assert_eq!(blocking_line("正文\nrobroz-action:get\nrobroz-action:other", &HashSet::new()), None);
    }

    #[test]
    fn blocking_line_returns_first_in_order() {
        let prompt = "开头\nrobroz-action:captcha\nrobroz-action:external";
        let found = blocking_line(prompt, &HashSet::new()).unwrap();
        assert_eq!(found.kind, "captcha");
        assert_eq!(found.line, "robroz-action:captcha");
        assert_eq!(found.hash, action_hash("robroz-action:captcha"));
    }

    #[test]
    fn blocking_line_skips_approved_lines() {
        let prompt = "robroz-action:external\n正文\nrobroz-action:captcha";
        let found = blocking_line(prompt, &set(&["robroz-action:external"])).unwrap();
        assert_eq!(found.kind, "captcha");
        assert_eq!(blocking_line(prompt, &set(&["robroz-action:external", "robroz-action:captcha"])), None);
        // Approving the later line does not skip the earlier one.
        let found = blocking_line(prompt, &set(&["robroz-action:captcha"])).unwrap();
        assert_eq!(found.kind, "external");
    }

    #[test]
    fn blocking_line_hashes_exact_line_and_shares_identical_lines() {
        let prompt = "  robroz-action:external \nrobroz-action:external";
        let found = blocking_line(prompt, &HashSet::new()).unwrap();
        assert_eq!(found.line, "  robroz-action:external ");
        assert_eq!(found.hash, action_hash("  robroz-action:external "));
        // Different bytes, different hash: the second line still blocks.
        let found = blocking_line(prompt, &set(&["  robroz-action:external "])).unwrap();
        assert_eq!(found.line, "robroz-action:external");
        // Identical lines share approval.
        let twice = "robroz-action:external\n正文\nrobroz-action:external";
        assert_eq!(blocking_line(twice, &set(&["robroz-action:external"])), None);
    }
}
