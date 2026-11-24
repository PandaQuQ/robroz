//! Pre-parse approval events so this client can read fields the contract
//! crate does not have yet (`reason`, `expiresAt`, `rule_add` / `rule_delete`)
//! and so an unknown `action` becomes a generic card instead of failing the
//! whole event. TODO: drop the action rewrite once `#[serde(other)]` ships,
//! and drop extras once the fields are on `Event::Approval`.

use robroz_contract::Event;
use robroz_ui::cards::{ApprovalExtras, ClientApprovalAction};
use serde_json::Value;

/// Decode one event. Unknown event *types* still fail (the caller skips
/// them). Unknown approval `action` values parse as `call` plus extras.
pub fn parse_event(value: Value) -> Result<(Event, Option<ApprovalExtras>), serde_json::Error> {
    let extras = approval_extras(&value);
    let value = patch_approval_action(value);
    let event = serde_json::from_value(value)?;
    Ok((event, extras))
}

pub fn parse_event_str(body: &str) -> Result<(Event, Option<ApprovalExtras>), serde_json::Error> {
    let value = serde_json::from_str(body)?;
    parse_event(value)
}

fn approval_extras(value: &Value) -> Option<ApprovalExtras> {
    if value.get("type").and_then(Value::as_str) != Some("approval") {
        return None;
    }
    let action = match value.get("action").and_then(Value::as_str) {
        Some("rule_add") => Some(ClientApprovalAction::RuleAdd),
        Some("rule_delete") => Some(ClientApprovalAction::RuleDelete),
        Some("delete" | "send" | "spend" | "call") => None,
        Some(_) => Some(ClientApprovalAction::Unknown),
        None => None,
    };
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let expires_at = value
        .get("expiresAt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(ApprovalExtras {
        action,
        reason,
        expires_at,
    })
}

fn patch_approval_action(mut value: Value) -> Value {
    if value.get("type").and_then(Value::as_str) != Some("approval") {
        return value;
    }
    let action = value.get("action").and_then(Value::as_str).unwrap_or("");
    let known = matches!(action, "delete" | "send" | "spend" | "call");
    if !known {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("action".into(), Value::String("call".into()));
        }
    }
    value
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
        let Event::Approval { action, .. } = event else {
            panic!("not approval");
        };
        assert_eq!(action, ApprovalAction::Call);
        let extras = extras.expect("extras");
        assert_eq!(extras.action, None);
        assert_eq!(extras.reason, None);
        let card = ApprovalCard::from_event_with(&event, Some(&extras)).unwrap();
        assert_eq!(card.kind, ActionKind::Call);
    }

    #[test]
    fn rule_add_parses_as_a_rule_card() {
        let (event, extras) = ev(&approval_json("rule_add", r#","reason":null"#));
        let extras = extras.expect("extras");
        assert_eq!(extras.action, Some(ClientApprovalAction::RuleAdd));
        let card = ApprovalCard::from_event_with(&event, Some(&extras)).unwrap();
        assert_eq!(card.kind, ActionKind::RuleAdd);
        assert_eq!(card.summary, "允许发周报");
        assert_eq!(card.state, ApprovalState::Pending);
    }

    #[test]
    fn rule_delete_parses() {
        let (event, extras) = ev(&approval_json("rule_delete", ""));
        let extras = extras.expect("extras");
        assert_eq!(extras.action, Some(ClientApprovalAction::RuleDelete));
        let card = ApprovalCard::from_event_with(&event, Some(&extras)).unwrap();
        assert_eq!(card.kind, ActionKind::RuleDelete);
    }

    #[test]
    fn unknown_action_is_a_generic_card() {
        let (event, extras) = ev(&approval_json("shell", r#","reason":"要改生产""#));
        let extras = extras.expect("extras");
        assert_eq!(extras.action, Some(ClientApprovalAction::Unknown));
        assert_eq!(extras.reason.as_deref(), Some("要改生产"));
        let card = ApprovalCard::from_event_with(&event, Some(&extras)).unwrap();
        assert_eq!(card.kind, ActionKind::Unknown);
        assert_eq!(card.reason.as_deref(), Some("要改生产"));
        // #[serde(other)] on ApprovalAction: unknown values parse as Unknown.
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
        let extra = format!(
            r#","decision":"denied","answeredAt":"2026-11-21T08:00:00Z","reason":"{}","expiresAt":"2026-11-21T08:00:00Z""#,
            review_rules::APPROVAL_EXPIRED_REASON
        );
        // approval_json already has decision:null — override by building fully.
        let json = format!(
            r#"{{"type":"approval","id":"a1","botId":"bot","botName":"文档哥","action":"call","summary":"发周报","path":null,"decision":"denied","createdAt":"2026-11-20T08:00:00Z","answeredAt":"2026-11-21T08:00:00Z","reason":"{}","expiresAt":"2026-11-21T08:00:00Z"}}"#,
            review_rules::APPROVAL_EXPIRED_REASON
        );
        let _ = extra;
        let (event, extras) = ev(&json);
        let Event::Approval {
            decision: Some(ApprovalDecision::Denied),
            ..
        } = event
        else {
            panic!("expected denied event");
        };
        let card = ApprovalCard::from_event_with(&event, extras.as_ref()).unwrap();
        assert!(matches!(card.state, ApprovalState::Expired { .. }));
    }
}
