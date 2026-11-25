//! Decode host events.
//!
//! `reason` and `expiresAt` live on `Event::Approval`. Unknown `action`
//! values parse as [`robroz_contract::ApprovalAction::Unknown`]. `rule_add`
//! / `rule_delete` are on the contract and parse as themselves.

use robroz_contract::Event;
use robroz_ui::cards::ApprovalExtras;
use serde_json::Value;

/// Event types this client does not draw yet; decoded as unknown so the
/// caller skips them (and still moves its cursor).
const NOT_DRAWN_YET: &[&str] = &["approval_reminder"];

/// Decode one event. Unknown event *types* still fail (the caller skips
/// them), and so do [`NOT_DRAWN_YET`] types.
pub fn parse_event(value: Value) -> Result<(Event, Option<ApprovalExtras>), serde_json::Error> {
    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        if NOT_DRAWN_YET.contains(&kind) {
            return Err(serde::de::Error::custom(format!(
                "event type {kind} is not drawn yet"
            )));
        }
    }
    let event = serde_json::from_value(value)?;
    Ok((event, None))
}

pub fn parse_event_str(body: &str) -> Result<(Event, Option<ApprovalExtras>), serde_json::Error> {
    let value = serde_json::from_str(body)?;
    parse_event(value)
}

/// Settings-page rule cards must not appear in any chat, stream or unread.
pub fn is_settings_approval(event: &Event) -> bool {
    matches!(
        event,
        Event::Approval { bot_id, .. } if bot_id == robroz_contract::SETTINGS_APPROVAL_BOT_ID
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use robroz_contract::{ApprovalAction, ApprovalDecision, Event, SETTINGS_APPROVAL_BOT_ID};
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
        assert!(extras.is_none());
        let Event::Approval { action, .. } = &event else {
            panic!("not approval");
        };
        assert_eq!(*action, ApprovalAction::RuleAdd);
        let card = ApprovalCard::from_event(&event).unwrap();
        assert_eq!(card.kind, ActionKind::RuleAdd);
        assert_eq!(card.summary, "允许发周报");
        assert_eq!(card.state, ApprovalState::Pending);
    }

    #[test]
    fn rule_delete_parses() {
        let (event, extras) = ev(&approval_json("rule_delete", ""));
        assert!(extras.is_none());
        let card = ApprovalCard::from_event(&event).unwrap();
        assert_eq!(card.kind, ActionKind::RuleDelete);
    }

    #[test]
    fn settings_bot_id_is_a_settings_approval() {
        let json = format!(
            r#"{{"type":"approval","id":"a1","botId":"{SETTINGS_APPROVAL_BOT_ID}","botName":"设置","action":"rule_add","summary":"允许发周报","path":null,"decision":null,"createdAt":"2026-11-20T08:00:00Z","answeredAt":null}}"#
        );
        let (event, _) = ev(&json);
        assert!(is_settings_approval(&event));
        assert!(!is_settings_approval(&ev(&approval_json("call", "")).0));
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
