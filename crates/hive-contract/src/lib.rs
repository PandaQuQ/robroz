use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "done")]
    Done,
}

impl TaskState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "排队",
            Self::Running => "进行",
            Self::Blocked => "卡住",
            Self::Done => "完成",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageKind {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "image")]
    Image,
    #[serde(rename = "file")]
    File,
}

impl MessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroup {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMember {
    pub bot_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMessage {
    pub kind: MessageKind,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTask {
    pub assignee_bot_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub group_id: String,
    pub bot_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub kind: MessageKind,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub assignee_bot_id: String,
    pub prompt: String,
    pub state: TaskState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

/// Internally tagged on `type` so message, status, and delivery round-trip
/// with the JSON already returned by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum Event {
    #[serde(rename = "message")]
    Message {
        id: String,
        kind: MessageKind,
        body: String,
        created_at: String,
    },
    #[serde(rename = "status")]
    Status {
        id: String,
        task_id: String,
        bot_id: String,
        bot_name: String,
        state: TaskState,
        label: String,
        created_at: String,
        updated_at: String,
        container_up: bool,
    },
    #[serde(rename = "delivery")]
    Delivery {
        id: String,
        summary: String,
        path: String,
        briefing: bool,
        created_at: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Events {
    pub events: Vec<Event>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_round_trip(event: &Event, expected: serde_json::Value) {
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value, expected);
        let back: Event = serde_json::from_value(value).unwrap();
        assert_eq!(&back, event);
    }

    #[test]
    fn message_event_json() {
        assert_round_trip(
            &Event::Message {
                id: "m1".into(),
                kind: MessageKind::Text,
                body: "hello".into(),
                created_at: "2026-10-03T19:00:00.000Z".into(),
            },
            serde_json::json!({
                "type": "message",
                "id": "m1",
                "kind": "text",
                "body": "hello",
                "createdAt": "2026-10-03T19:00:00.000Z"
            }),
        );
    }

    #[test]
    fn queued_status_event_json() {
        assert_round_trip(
            &Event::Status {
                id: "t1".into(),
                task_id: "t1".into(),
                bot_id: "bot-ada".into(),
                bot_name: "Ada".into(),
                state: TaskState::Queued,
                label: TaskState::Queued.label().into(),
                created_at: "2026-10-03T19:00:01.000Z".into(),
                updated_at: "2026-10-03T19:00:01.000Z".into(),
                container_up: false,
            },
            serde_json::json!({
                "type": "status",
                "id": "t1",
                "taskId": "t1",
                "botId": "bot-ada",
                "botName": "Ada",
                "state": "queued",
                "label": "排队",
                "createdAt": "2026-10-03T19:00:01.000Z",
                "updatedAt": "2026-10-03T19:00:01.000Z",
                "containerUp": false
            }),
        );
    }

    #[test]
    fn delivery_event_json() {
        assert_round_trip(
            &Event::Delivery {
                id: "d1".into(),
                summary: "草图".into(),
                path: "out/sketch.png".into(),
                briefing: true,
                created_at: "2026-10-03T19:00:02.000Z".into(),
            },
            serde_json::json!({
                "type": "delivery",
                "id": "d1",
                "summary": "草图",
                "path": "out/sketch.png",
                "briefing": true,
                "createdAt": "2026-10-03T19:00:02.000Z"
            }),
        );
    }

    #[test]
    fn task_state_labels() {
        assert_eq!(TaskState::Queued.label(), "排队");
        assert_eq!(TaskState::Running.label(), "进行");
        assert_eq!(TaskState::Blocked.label(), "卡住");
        assert_eq!(TaskState::Done.label(), "完成");
        assert_eq!(serde_json::to_string(&TaskState::Queued).unwrap(), "\"queued\"");
        assert_eq!(serde_json::to_string(&TaskState::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&TaskState::Blocked).unwrap(), "\"blocked\"");
        assert_eq!(serde_json::to_string(&TaskState::Done).unwrap(), "\"done\"");
    }
}
