//! Deprecated: the Chinese card copy the host still sends next to the
//! machine codes in [`crate::codes`]. It moved here from `robroz-card` so
//! the host does not depend on that crate. Nothing here is new API: a client
//! should render from the codes through its own i18n, and this copy stays
//! only so old clients keep working (additive contract: nothing is removed).

use crate::TaskState;

/// Deprecated: use [`crate::codes::FailureCode::NoResult`] and
/// [`crate::codes::CardStatusCode::NoResult`]. The dead-run line. The host
/// ends a dead run as `done` with exactly this `failure`; older data may
/// still have it as `running` with no container.
pub const NO_RESULT: &str = "没写出结果";

/// Deprecated: use [`crate::codes::CardStatusCode`]. The `label` of a status
/// card. `container_up` is ignored except for a `running` card: a `running`
/// card with no container is a dead run in old data and reads [`NO_RESULT`].
pub fn status_line(state: TaskState, container_up: bool) -> &'static str {
    match state {
        TaskState::Queued => "排队",
        TaskState::Running if container_up => "进行",
        TaskState::Running => NO_RESULT,
        TaskState::Done => "完成",
        TaskState::Blocked => "卡住",
    }
}

/// Deprecated: use [`crate::codes::DeliveryBadge::Briefing`]. The corner
/// badge of a briefing delivery.
pub fn briefing_badge() -> &'static str {
    "简报"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_for_each_state() {
        assert_eq!(status_line(TaskState::Queued, false), "排队");
        assert_eq!(status_line(TaskState::Queued, true), "排队");
        assert_eq!(status_line(TaskState::Running, true), "进行");
        assert_eq!(status_line(TaskState::Running, false), "没写出结果");
        assert_eq!(status_line(TaskState::Done, false), "完成");
        assert_eq!(status_line(TaskState::Blocked, false), "卡住");
    }

    #[test]
    fn briefing_badge_text() {
        assert_eq!(briefing_badge(), "简报");
    }
}
