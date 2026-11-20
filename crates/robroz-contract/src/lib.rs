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

/// `GET /api/groups` body, oldest group first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Groups {
    pub groups: Vec<Group>,
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
#[serde(rename_all = "camelCase")]
pub struct CreateSchedule {
    pub group_id: String,
    pub assignee_bot_id: String,
    pub cron: String,
    pub prompt: String,
}

/// `lastTaskId` is always present and is `null` until the first run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub group_id: String,
    pub assignee_bot_id: String,
    pub cron: String,
    pub prompt: String,
    pub next_run_at: String,
    pub last_task_id: Option<String>,
}

/// A schedule as listed for the routines view: the frozen [`Schedule`] fields
/// plus `paused`. `POST /api/schedules` still returns a plain [`Schedule`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEntry {
    #[serde(flatten)]
    pub schedule: Schedule,
    pub paused: bool,
}

/// `GET /api/groups/{id}/schedules` body, oldest schedule first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedules {
    pub schedules: Vec<ScheduleEntry>,
}

/// `PATCH /api/schedules/{id}` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSchedule {
    pub paused: bool,
}

/// One option on a choice card. `id` is what comes back in `choice`.
/// `primary` marks the bot's recommendation; at most one, ignored when `multi`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub primary: bool,
}

/// `POST /api/groups/{id}/choices` body: a bot in the group asks the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChoice {
    pub bot_id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub multi: bool,
}

/// `POST /api/choices/{id}/answer` body: option `id`s, always an array.
/// Single choice sends exactly one; multi sends one or more, no repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerChoice {
    pub choice: Vec<String>,
}

/// Choice cards hold 2 to 4 options with distinct, non-empty values.
pub const CHOICE_OPTIONS_MIN: usize = 2;
pub const CHOICE_OPTIONS_MAX: usize = 4;

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
    /// Added after the node-1 freeze; the three types above are unchanged.
    /// `choice` and `answeredAt` are always present and `null` until answered.
    #[serde(rename = "choice")]
    Choice {
        id: String,
        bot_id: String,
        bot_name: String,
        prompt: String,
        options: Vec<ChoiceOption>,
        multi: bool,
        choice: Option<Vec<String>>,
        created_at: String,
        answered_at: Option<String>,
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
    fn schedule_json() {
        let request: CreateSchedule = serde_json::from_value(serde_json::json!({
            "groupId": "g1",
            "assigneeBotId": "bot-ada",
            "cron": "0 9 * * *",
            "prompt": "早报"
        }))
        .unwrap();
        assert_eq!(request.group_id, "g1");
        assert_eq!(request.assignee_bot_id, "bot-ada");

        let schedule = Schedule {
            id: "s1".into(),
            group_id: "g1".into(),
            assignee_bot_id: "bot-ada".into(),
            cron: "0 9 * * *".into(),
            prompt: "早报".into(),
            next_run_at: "2026-10-08T09:00:00.000Z".into(),
            last_task_id: None,
        };
        let value = serde_json::to_value(&schedule).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "id": "s1",
                "groupId": "g1",
                "assigneeBotId": "bot-ada",
                "cron": "0 9 * * *",
                "prompt": "早报",
                "nextRunAt": "2026-10-08T09:00:00.000Z",
                "lastTaskId": null
            })
        );
        let back: Schedule = serde_json::from_value(value).unwrap();
        assert_eq!(back, schedule);

        let ran = Schedule {
            last_task_id: Some("t1".into()),
            ..schedule
        };
        assert_eq!(serde_json::to_value(&ran).unwrap()["lastTaskId"], "t1");
    }

    #[test]
    fn choice_event_json() {
        let options = vec![
            ChoiceOption { id: "a".into(), label: "方案 A".into(), primary: true },
            ChoiceOption { id: "b".into(), label: "方案 B".into(), primary: false },
        ];
        assert_round_trip(
            &Event::Choice {
                id: "c1".into(),
                bot_id: "bot-ada".into(),
                bot_name: "Ada".into(),
                prompt: "用哪个方案？".into(),
                options: options.clone(),
                multi: false,
                choice: None,
                created_at: "2026-10-03T19:00:03.000Z".into(),
                answered_at: None,
            },
            serde_json::json!({
                "type": "choice",
                "id": "c1",
                "botId": "bot-ada",
                "botName": "Ada",
                "prompt": "用哪个方案？",
                "options": [
                    { "id": "a", "label": "方案 A", "primary": true },
                    { "id": "b", "label": "方案 B", "primary": false }
                ],
                "multi": false,
                "choice": null,
                "createdAt": "2026-10-03T19:00:03.000Z",
                "answeredAt": null
            }),
        );
        let request: CreateChoice = serde_json::from_value(serde_json::json!({
            "botId": "bot-ada",
            "prompt": "用哪个方案？",
            "options": [{ "id": "a", "label": "方案 A" }]
        }))
        .unwrap();
        assert_eq!(request.bot_id, "bot-ada");
        assert!(!request.multi && !request.options[0].primary, "multi and primary default to false");
        let answer: AnswerChoice = serde_json::from_value(serde_json::json!({ "choice": ["b"] })).unwrap();
        assert_eq!(answer.choice, vec!["b".to_string()]);
    }

    #[test]
    fn schedule_entry_flattens_and_adds_paused() {
        let entry = ScheduleEntry {
            schedule: Schedule {
                id: "s1".into(),
                group_id: "g1".into(),
                assignee_bot_id: "bot-ada".into(),
                cron: "0 9 * * *".into(),
                prompt: "早报".into(),
                next_run_at: "2026-10-08T09:00:00.000Z".into(),
                last_task_id: None,
            },
            paused: true,
        };
        let value = serde_json::to_value(&Schedules { schedules: vec![entry.clone()] }).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "schedules": [{
                "id": "s1",
                "groupId": "g1",
                "assigneeBotId": "bot-ada",
                "cron": "0 9 * * *",
                "prompt": "早报",
                "nextRunAt": "2026-10-08T09:00:00.000Z",
                "lastTaskId": null,
                "paused": true
            }]})
        );
        let back: Schedules = serde_json::from_value(value).unwrap();
        assert_eq!(back.schedules[0], entry);
        let patch: UpdateSchedule = serde_json::from_value(serde_json::json!({ "paused": false })).unwrap();
        assert!(!patch.paused);
    }

    #[test]
    fn groups_json() {
        let groups = Groups {
            groups: vec![
                Group { id: "g1".into(), name: "设计群".into(), tenant_id: "local".into() },
                Group { id: "g2".into(), name: "运营群".into(), tenant_id: "local".into() },
            ],
        };
        let value = serde_json::to_value(&groups).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "groups": [
                    { "id": "g1", "name": "设计群", "tenantId": "local" },
                    { "id": "g2", "name": "运营群", "tenantId": "local" }
                ]
            })
        );
        let back: Groups = serde_json::from_value(value).unwrap();
        assert_eq!(back, groups);

        let empty = Groups { groups: vec![] };
        assert_eq!(serde_json::to_value(&empty).unwrap(), serde_json::json!({ "groups": [] }));
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
