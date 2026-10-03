/// Pure decisions. This crate does not start containers or touch the clock.

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
}
