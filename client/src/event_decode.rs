//! Decode host events.
//!
//! `reason` and `expiresAt` now live on `Event::Approval`, and unknown
//! `action` values parse as [`robroz_contract::ApprovalAction::Unknown`].
//!
//! `rule_add` / `rule_delete` are not on the contract yet, so they also
//! parse as `Unknown`. A small stash keeps the real action so the settings
//! confirm cards still render. TODO: drop the stash once the auto-review
//! backend PR adds those variants.

use robroz_contract::Event;
use robroz_ui::cards::{ApprovalExtras, ClientApprovalAction};
use serde_json::Value;

/// Decode one event. Unknown event *types* still fail (the caller skips
/// them). `rule_add` / `rule_delete` come back as `Unknown` plus extras.
pub fn parse_event(value: Value) -> Result<(Event, Option<ApprovalExtras>), serde_json::Error> {
    let extras = rule_action_extras(&value);
    let event = serde_json::from_value(value)?;
    Ok((event, extras))
}

pub fn parse_event_str(body: &str) -> Result<(Event, Option<ApprovalExtras>), serde_json::Error> {
    let value = serde_json::from_str(body)?;
    parse_event(value)
}

/// TODO: drop once `ApprovalAction` has `rule_add` / `rule_delete`.
fn rule_action_extras(value: &Value) -> Option<ApprovalExtras> {
    if value.get("type").and_then(Value::as_str) != Some("approval") {
        return None;
    }
    let action = match value.get("action").and_then(Value::as_str) {
        Some("rule_add") => ClientApprovalAction::RuleAdd,
        Some("rule_delete") => ClientApprovalAction::RuleDelete,
        _ => return None,
    };
    Some(ApprovalExtras {
        action: Some(action),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::{ApprovalAction, ApprovalDecision, Event};
    use robroz_ui::cards::{review_rules, ActionKind, ApprovalCard, ApprovalState};

    fn ev(json: &str) -> (Event, Option<ApprovalExtras>) {
        parse_event_str(json).expect("parse")
    }

    fn approval_json(action: &str, extra: &str) -> String {
        format!(
            r#"{{"type":"approval","id":"a1","botId":"bot","botName":"文档哥","action":"{action}","summary":"允许发周报","path":null,"decision":null,"createdAt":"2026-11-20T08:00:00Z","answeredAt":null{extra}}}"#
        )
    }

    #[test]
    fn known_approval_is_unchanged() {
        let (event, extras) = ev(&approval_json("call", ""));
        let Event::Approval {
            action,
            reason,
            expires_at,
            ..
        } = &event
        else {
            panic!("not approval");
        };
        assert_eq!(*action, ApprovalAction::Call);
        assert_eq!(reason, &None);
        assert_eq!(expires_at, &None);
        assert!(extras.is_none());
        let card = ApprovalCard::from_event(&event).unwrap();
        assert_eq!(card.kind, ActionKind::Call);
    }

    #[test]
    fn rule_add_parses_as_a_rule_card() {
        let (event, extras) = ev(&approval_json("rule_add", r#","reason":null"#));
        let Event::Approval { action, .. } = &event else {
            panic!("not approval");
        };
        assert_eq!(*action, ApprovalAction::Unknown);
        let extras = extras.expect("rule_* shim");
        assert_eq!(extras.action, Some(ClientApprovalAction::RuleAdd));
        let card = ApprovalCard::from_event_with(&event, Some(&extras)).unwrap();
        assert_eq!(card.kind, ActionKind::RuleAdd);
        assert_eq!(card.summary, "允许发周报");
        assert_eq!(card.state, ApprovalState::Pending);
    }

    #[test]
    fn rule_delete_parses() {
        let (event, extras) = ev(&approval_json("rule_delete", ""));
        let extras = extras.expect("rule_* shim");
        assert_eq!(extras.action, Some(ClientApprovalAction::RuleDelete));
        let card = ApprovalCard::from_event_with(&event, Some(&extras)).unwrap();
        assert_eq!(card.kind, ActionKind::RuleDelete);
    }

    #[test]
    fn unknown_action_is_a_generic_card() {
        let (event, extras) = ev(&approval_json("shell", r#","reason":"要改生产""#));
        assert!(extras.is_none());
        let Event::Approval { action, reason, .. } = &event else {
            panic!("not approval");
        };
        assert_eq!(*action, ApprovalAction::Unknown);
        assert_eq!(reason.as_deref(), Some("要改生产"));
        let card = ApprovalCard::from_event(&event).unwrap();
        assert_eq!(card.kind, ActionKind::Unknown);
        assert_eq!(card.reason.as_deref(), Some("要改生产"));
        let raw = approval_json("shell", "");
        let raw_event: Event = serde_json::from_str(&raw).unwrap();
        assert!(matches!(
            raw_event,
            Event::Approval {
                action: ApprovalAction::Unknown,
                ..
            }
        ));
    }

    #[test]
    fn expired_reason_becomes_expired_state() {
        let json = format!(
            r#"{{"type":"approval","id":"a1","botId":"bot","botName":"文档哥","action":"call","summary":"发周报","path":null,"decision":"denied","createdAt":"2026-11-20T08:00:00Z","answeredAt":"2026-11-21T08:00:00Z","reason":"{}","expiresAt":"2026-11-21T08:00:00Z"}}"#,
            review_rules::APPROVAL_EXPIRED_REASON
        );
        let (event, extras) = ev(&json);
        assert!(extras.is_none());
        let Event::Approval {
            decision: Some(ApprovalDecision::Denied),
            reason,
            expires_at,
            ..
        } = &event
        else {
            panic!("expected denied event");
        };
        assert_eq!(
            reason.as_deref(),
            Some(review_rules::APPROVAL_EXPIRED_REASON)
        );
        assert!(expires_at.is_some());
        let card = ApprovalCard::from_event(&event).unwrap();
        assert!(matches!(card.state, ApprovalState::Expired { .. }));
    }
}
