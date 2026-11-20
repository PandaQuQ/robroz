//! Calls the client makes when someone clicks a card, a routine toggle, a
//! chat or 「＋ 新 bro」.
//!
//! All endpoint paths live in [`paths`], so switching to final backend names
//! is a one-place change.
//! - `choice` and routines use robroz#15: `POST /api/choices/:id/answer`,
//!   `GET /api/groups/:id/schedules`, `PATCH /api/schedules/:id`.
//! - Unread uses robroz#26: `GET /api/unread`, `POST /api/groups/:id/read`.
//! - Bros and DMs use robroz#28: `GET /api/bros`, `POST /api/bros`.
//! - The composer uses the existing `POST /api/groups/:id/messages` (store
//!   the text) and `POST /api/groups/:id/tasks` (queue it for a bro).
//! - `approval` and `draft` have no backend endpoints yet. They use the UI
//!   SPEC §9 suggestions (`POST /api/cards/:id/decision`,
//!   `POST /api/cards/:id/send`) until the backend defines them.

use std::sync::Mutex;

use robroz_contract::{
    next_bro_name, AnswerChoice, Bro, Bros, CreateBro, CreateTask, ErrorBody, Event, Message,
    MessageKind, PostMessage, RenameBro, ScheduleEntry, Schedules, Task, TaskState, Unread,
    Unreads, UpdateSchedule,
};
use robroz_ui::cards::interact::Note;

use crate::conn::Conn;

pub mod paths {
    /// robroz#15.
    pub fn answer_choice(id: &str) -> String {
        format!("/api/choices/{id}/answer")
    }
    /// robroz#15.
    pub fn group_schedules(group_id: &str) -> String {
        format!("/api/groups/{group_id}/schedules")
    }
    /// robroz#15.
    pub fn schedule(id: &str) -> String {
        format!("/api/schedules/{id}")
    }
    /// robroz#26.
    pub const UNREAD: &str = "/api/unread";
    /// robroz#26.
    pub fn mark_read(group_id: &str) -> String {
        format!("/api/groups/{group_id}/read")
    }
    /// robroz#28.
    pub const BROS: &str = "/api/bros";
    /// robroz#28: rename a bro (its DM follows).
    pub fn bro(id: &str) -> String {
        format!("/api/bros/{id}")
    }
    /// Existing host route: store a user message in a group.
    pub fn messages(group_id: &str) -> String {
        format!("/api/groups/{group_id}/messages")
    }
    /// Existing host route: queue a task for a bro in a group.
    pub fn tasks(group_id: &str) -> String {
        format!("/api/groups/{group_id}/tasks")
    }
    /// SPEC §9 suggestion; not in the backend yet.
    pub fn approval_decision(id: &str) -> String {
        format!("/api/cards/{id}/decision")
    }
    /// SPEC §9 suggestion; not in the backend yet.
    pub fn send_draft(id: &str) -> String {
        format!("/api/cards/{id}/send")
    }
}

/// Request bodies, kept as pure functions so their shape is tested.
pub mod bodies {
    use super::*;

    /// `{"choice":[ids]}`; an array even for single choice.
    pub fn answer(choice: &[String]) -> String {
        serde_json::to_string(&AnswerChoice {
            choice: choice.to_vec(),
        })
        .expect("serialize answer")
    }

    pub fn decision(approve: bool) -> String {
        serde_json::json!({ "approve": approve }).to_string()
    }

    /// `{"kind":"text","body":…}`.
    pub fn text(body: &str) -> String {
        serde_json::to_string(&PostMessage {
            kind: MessageKind::Text,
            body: body.to_string(),
        })
        .expect("serialize message")
    }

    /// `{"assigneeBotId":…,"prompt":…}`.
    pub fn task(bot_id: &str, prompt: &str) -> String {
        serde_json::to_string(&CreateTask {
            assignee_bot_id: bot_id.to_string(),
            prompt: prompt.to_string(),
        })
        .expect("serialize task")
    }

    /// `{"name":…}`.
    pub fn rename(name: &str) -> String {
        serde_json::to_string(&RenameBro {
            name: name.to_string(),
        })
        .expect("serialize rename")
    }

    pub fn paused(paused: bool) -> String {
        serde_json::to_string(&UpdateSchedule { paused }).expect("serialize patch")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 409, e.g. the choice was already answered.
    Conflict(String),
    /// 400, e.g. an option that is not on the card.
    BadRequest(String),
    NotFound(String),
    Status(u16, String),
    Transport(String),
    Decode(String),
}

impl ApiError {
    /// Footnote for a card whose action failed.
    pub fn note(&self) -> Note {
        match self {
            ApiError::Conflict(_) => Note::AlreadyAnswered,
            ApiError::BadRequest(msg) => Note::Rejected(msg.clone()),
            ApiError::Transport(_) => Note::Retry,
            ApiError::NotFound(msg) | ApiError::Status(_, msg) | ApiError::Decode(msg) => {
                Note::Failed(msg.clone())
            }
        }
    }
}

/// Blocking calls; the app runs them off the UI thread.
pub trait CardApi: Send + Sync {
    /// Every group's unread count and last activity, newest first.
    fn unread(&self) -> Result<Vec<Unread>, ApiError>;
    /// Mark a chat read up to now; returns its new unread state.
    fn mark_read(&self, group_id: &str) -> Result<Unread, ApiError>;
    /// All bros with their DM group.
    fn bros(&self) -> Result<Vec<Bro>, ApiError>;
    /// New bro with the next default name and its DM.
    fn create_bro(&self) -> Result<Bro, ApiError>;
    /// Rename a bro; the host renames its DM too.
    fn rename_bro(&self, id: &str, name: &str) -> Result<Bro, ApiError>;
    /// Composer: store a text message in a group.
    fn send_message(&self, group_id: &str, body: &str) -> Result<Message, ApiError>;
    /// Composer: queue `prompt` for a bro who is in the group.
    fn create_task(&self, group_id: &str, bot_id: &str, prompt: &str) -> Result<Task, ApiError>;
    /// Returns the updated `choice` event when the host sends it back.
    fn answer_choice(&self, id: &str, choice: &[String]) -> Result<Option<Event>, ApiError>;
    fn decide(&self, id: &str, approve: bool) -> Result<(), ApiError>;
    fn send_draft(&self, id: &str) -> Result<(), ApiError>;
    fn schedules(&self, group_id: &str) -> Result<Vec<ScheduleEntry>, ApiError>;
    fn set_paused(&self, id: &str, paused: bool) -> Result<ScheduleEntry, ApiError>;
}

pub struct HttpApi {
    conn: Conn,
}

impl HttpApi {
    pub fn new(conn: Conn) -> Self {
        Self { conn }
    }

    fn finish(
        &self,
        method: &str,
        url: &str,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<String, ApiError> {
        let mut response = result
            .map_err(|e| ApiError::Transport(format!("连不上宿主机 {}: {e}", self.conn.base())))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| ApiError::Transport(format!("读 {url} 失败: {e}")))?;
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let msg = serde_json::from_str::<ErrorBody>(&body)
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("{method} {url} 返回 {status}"));
        Err(match status {
            400 => ApiError::BadRequest(msg),
            404 => ApiError::NotFound(msg),
            409 => ApiError::Conflict(msg),
            _ => ApiError::Status(status, msg),
        })
    }

    fn post(&self, path: &str, body: Option<String>) -> Result<String, ApiError> {
        let url = self.conn.url(path);
        let req = self.conn.post(path);
        let result = match body {
            Some(b) => req.header("content-type", "application/json").send(b),
            None => req.send_empty(),
        };
        self.finish("POST", &url, result)
    }

    fn patch(&self, path: &str, body: String) -> Result<String, ApiError> {
        let url = self.conn.url(path);
        let result = self
            .conn
            .patch(path)
            .header("content-type", "application/json")
            .send(body);
        self.finish("PATCH", &url, result)
    }

    fn get(&self, path: &str) -> Result<String, ApiError> {
        let url = self.conn.url(path);
        let result = self.conn.get(path).call();
        self.finish("GET", &url, result)
    }
}

fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Decode(format!("与契约不符: {e}")))
}

impl CardApi for HttpApi {
    fn unread(&self) -> Result<Vec<Unread>, ApiError> {
        Ok(decode::<Unreads>(&self.get(paths::UNREAD)?)?.groups)
    }

    fn mark_read(&self, group_id: &str) -> Result<Unread, ApiError> {
        decode(&self.post(&paths::mark_read(group_id), None)?)
    }

    fn bros(&self) -> Result<Vec<Bro>, ApiError> {
        Ok(decode::<Bros>(&self.get(paths::BROS)?)?.bros)
    }

    fn create_bro(&self) -> Result<Bro, ApiError> {
        let body = serde_json::to_string(&CreateBro::default()).expect("serialize bro");
        decode(&self.post(paths::BROS, Some(body))?)
    }

    fn rename_bro(&self, id: &str, name: &str) -> Result<Bro, ApiError> {
        decode(&self.patch(&paths::bro(id), bodies::rename(name))?)
    }

    fn send_message(&self, group_id: &str, body: &str) -> Result<Message, ApiError> {
        decode(&self.post(&paths::messages(group_id), Some(bodies::text(body)))?)
    }

    fn create_task(&self, group_id: &str, bot_id: &str, prompt: &str) -> Result<Task, ApiError> {
        decode(&self.post(&paths::tasks(group_id), Some(bodies::task(bot_id, prompt)))?)
    }

    fn answer_choice(&self, id: &str, choice: &[String]) -> Result<Option<Event>, ApiError> {
        let body = self.post(&paths::answer_choice(id), Some(bodies::answer(choice)))?;
        Ok(decode::<Event>(&body).ok())
    }

    fn decide(&self, id: &str, approve: bool) -> Result<(), ApiError> {
        self.post(
            &paths::approval_decision(id),
            Some(bodies::decision(approve)),
        )
        .map(|_| ())
    }

    fn send_draft(&self, id: &str) -> Result<(), ApiError> {
        self.post(&paths::send_draft(id), None).map(|_| ())
    }

    fn schedules(&self, group_id: &str) -> Result<Vec<ScheduleEntry>, ApiError> {
        let body = self.get(&paths::group_schedules(group_id))?;
        Ok(decode::<Schedules>(&body)?.schedules)
    }

    fn set_paused(&self, id: &str, paused: bool) -> Result<ScheduleEntry, ApiError> {
        decode(&self.patch(&paths::schedule(id), bodies::paused(paused))?)
    }
}

/// Offline API for `--fake` and the gallery: accepts everything and keeps
/// schedules, unread counts and bros in memory.
pub struct FakeApi {
    schedules: Mutex<Vec<ScheduleEntry>>,
    unread: Mutex<Vec<Unread>>,
    bros: Mutex<Vec<Bro>>,
}

impl FakeApi {
    pub fn new(schedules: Vec<ScheduleEntry>) -> Self {
        Self {
            schedules: Mutex::new(schedules),
            unread: Mutex::new(Vec::new()),
            bros: Mutex::new(Vec::new()),
        }
    }

    /// Seed unread counts and DMs for the fake chats.
    pub fn with_chats(self, unread: Vec<Unread>, bros: Vec<Bro>) -> Self {
        *self.unread.lock().expect("fake unread lock") = unread;
        *self.bros.lock().expect("fake bros lock") = bros;
        self
    }
}

impl CardApi for FakeApi {
    fn unread(&self) -> Result<Vec<Unread>, ApiError> {
        let mut all = self.unread.lock().expect("fake unread lock").clone();
        all.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
        Ok(all)
    }

    fn mark_read(&self, group_id: &str) -> Result<Unread, ApiError> {
        let mut all = self.unread.lock().expect("fake unread lock");
        let u = all
            .iter_mut()
            .find(|u| u.group_id == group_id)
            .ok_or_else(|| ApiError::NotFound("群不存在".into()))?;
        u.unread = 0;
        u.read_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(u.clone())
    }

    fn bros(&self) -> Result<Vec<Bro>, ApiError> {
        Ok(self.bros.lock().expect("fake bros lock").clone())
    }

    fn create_bro(&self) -> Result<Bro, ApiError> {
        let mut bros = self.bros.lock().expect("fake bros lock");
        let n = bros.len() + 1;
        let bro = Bro {
            id: format!("fake-bro-{n}"),
            name: next_bro_name(bros.iter().map(|b| b.name.as_str())),
            dm_group_id: Some(format!("fake-dm-{n}")),
        };
        bros.push(bro.clone());
        self.unread.lock().expect("fake unread lock").push(Unread {
            group_id: format!("fake-dm-{n}"),
            read_at: None,
            unread: 0,
            last_activity_at: Some(chrono::Utc::now().to_rfc3339()),
        });
        Ok(bro)
    }

    fn rename_bro(&self, id: &str, name: &str) -> Result<Bro, ApiError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ApiError::BadRequest("请填写名称".into()));
        }
        let mut bros = self.bros.lock().expect("fake bros lock");
        let bro = bros
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or_else(|| ApiError::NotFound("bro 不存在".into()))?;
        bro.name = name.to_string();
        Ok(bro.clone())
    }

    fn send_message(&self, _: &str, body: &str) -> Result<Message, ApiError> {
        if body.trim().is_empty() {
            return Err(ApiError::BadRequest("请填写内容".into()));
        }
        Ok(Message {
            id: format!("fake-msg-{}", uuid_ish()),
            kind: MessageKind::Text,
            body: body.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    fn create_task(&self, _: &str, bot_id: &str, prompt: &str) -> Result<Task, ApiError> {
        Ok(Task {
            id: format!("fake-task-{}", uuid_ish()),
            assignee_bot_id: bot_id.to_string(),
            prompt: prompt.to_string(),
            state: TaskState::Queued,
        })
    }

    fn answer_choice(&self, _: &str, _: &[String]) -> Result<Option<Event>, ApiError> {
        Ok(None)
    }

    fn decide(&self, _: &str, _: bool) -> Result<(), ApiError> {
        Ok(())
    }

    fn send_draft(&self, _: &str) -> Result<(), ApiError> {
        Ok(())
    }

    fn schedules(&self, group_id: &str) -> Result<Vec<ScheduleEntry>, ApiError> {
        let all = self.schedules.lock().expect("fake schedules lock");
        Ok(all
            .iter()
            .filter(|s| s.schedule.group_id == group_id)
            .cloned()
            .collect())
    }

    fn set_paused(&self, id: &str, paused: bool) -> Result<ScheduleEntry, ApiError> {
        let mut all = self.schedules.lock().expect("fake schedules lock");
        let entry = all
            .iter_mut()
            .find(|s| s.schedule.id == id)
            .ok_or_else(|| ApiError::NotFound("例行任务不存在".into()))?;
        entry.paused = paused;
        Ok(entry.clone())
    }
}

/// Unique-enough id for fake echoes.
fn uuid_ish() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    nanos ^ u128::from(N.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_backend_and_spec() {
        assert_eq!(paths::answer_choice("c1"), "/api/choices/c1/answer");
        assert_eq!(paths::group_schedules("g1"), "/api/groups/g1/schedules");
        assert_eq!(paths::schedule("s1"), "/api/schedules/s1");
        assert_eq!(paths::approval_decision("a1"), "/api/cards/a1/decision");
        assert_eq!(paths::send_draft("d1"), "/api/cards/d1/send");
        assert_eq!(paths::UNREAD, "/api/unread");
        assert_eq!(paths::mark_read("g1"), "/api/groups/g1/read");
        assert_eq!(paths::BROS, "/api/bros");
        assert_eq!(paths::messages("g1"), "/api/groups/g1/messages");
        assert_eq!(paths::tasks("g1"), "/api/groups/g1/tasks");
        assert_eq!(paths::bro("b1"), "/api/bros/b1");
        assert_eq!(bodies::rename("Ada"), r#"{"name":"Ada"}"#);
    }

    #[test]
    fn bodies() {
        assert_eq!(bodies::answer(&["cargo".into()]), r#"{"choice":["cargo"]}"#);
        assert_eq!(
            bodies::answer(&["a".into(), "b".into()]),
            r#"{"choice":["a","b"]}"#
        );
        assert_eq!(bodies::decision(true), r#"{"approve":true}"#);
        assert_eq!(bodies::paused(false), r#"{"paused":false}"#);
    }

    #[test]
    fn errors_map_to_notes() {
        assert_eq!(ApiError::Conflict("x".into()).note(), Note::AlreadyAnswered);
        assert_eq!(
            ApiError::BadRequest("没有这个选项".into()).note(),
            Note::Rejected("没有这个选项".into())
        );
        assert_eq!(ApiError::Transport("down".into()).note(), Note::Retry);
    }

    #[test]
    fn host_down_is_transport_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let api = HttpApi::new(Conn::new(format!("http://127.0.0.1:{port}")));
        assert!(matches!(
            api.answer_choice("c", &["a".into()]),
            Err(ApiError::Transport(_))
        ));
    }

    /// The real host router in-process (robroz#15 endpoints).
    mod against_host {
        use super::*;
        use robroz_contract::{
            AddMember, ChoiceOption, CreateChoice, CreateGroup, CreateSchedule, Group, Schedule,
        };

        fn spawn_host() -> String {
            crate::source::http_test_host()
        }

        fn post<T: serde::Serialize, R: serde::de::DeserializeOwned>(
            base: &str,
            path: &str,
            body: &T,
        ) -> R {
            let mut response = ureq::post(&format!("{base}{path}"))
                .header("content-type", "application/json")
                .send(serde_json::to_string(body).unwrap())
                .unwrap();
            serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap()
        }

        fn group_with_bot(base: &str) -> Group {
            let group: Group = post(
                base,
                "/api/groups",
                &CreateGroup {
                    name: "官网".into(),
                },
            );
            let _: serde_json::Value = post(
                base,
                &format!("/api/groups/{}/members", group.id),
                &AddMember {
                    bot_id: "bot-doc".into(),
                    name: "文档哥".into(),
                },
            );
            group
        }

        #[test]
        fn answer_choice_then_409_and_400() {
            let base = spawn_host();
            let group = group_with_bot(&base);
            let option = |id: &str| ChoiceOption {
                id: id.into(),
                label: id.into(),
                primary: false,
            };
            let created: Event = post(
                &base,
                &format!("/api/groups/{}/choices", group.id),
                &CreateChoice {
                    bot_id: "bot-doc".into(),
                    prompt: "先写哪一种？".into(),
                    options: vec![option("docker"), option("cargo")],
                    multi: false,
                },
            );
            let Event::Choice { id, .. } = created else {
                panic!("expected choice event");
            };
            let api = HttpApi::new(Conn::new(&base));
            assert!(matches!(
                api.answer_choice(&id, &["nope".into()]),
                Err(ApiError::BadRequest(_))
            ));
            let answered = api.answer_choice(&id, &["cargo".into()]).unwrap();
            match answered {
                Some(Event::Choice { choice, .. }) => {
                    assert_eq!(choice, Some(vec!["cargo".to_string()]))
                }
                other => panic!("expected answered choice, got {other:?}"),
            }
            assert!(matches!(
                api.answer_choice(&id, &["docker".into()]),
                Err(ApiError::Conflict(_))
            ));
        }

        #[test]
        fn list_and_pause_schedules() {
            let base = spawn_host();
            let group = group_with_bot(&base);
            let created: Schedule = post(
                &base,
                "/api/schedules",
                &CreateSchedule {
                    group_id: group.id.clone(),
                    assignee_bot_id: "bot-doc".into(),
                    cron: "0 8 * * *".into(),
                    prompt: "每日简报".into(),
                },
            );
            let api = HttpApi::new(Conn::new(&base));
            let list = api.schedules(&group.id).unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].schedule.id, created.id);
            assert!(!list[0].paused);
            let paused = api.set_paused(&created.id, true).unwrap();
            assert!(paused.paused);
            assert!(api.schedules(&group.id).unwrap()[0].paused);
            assert!(matches!(
                api.set_paused("missing", true),
                Err(ApiError::NotFound(_))
            ));
        }

        #[test]
        fn unread_list_and_mark_read() {
            let base = spawn_host();
            let quiet = group_with_bot(&base);
            std::thread::sleep(std::time::Duration::from_millis(5));
            let busy = group_with_bot(&base);
            // A choice card from a bro counts as unread.
            let _: Event = post(
                &base,
                &format!("/api/groups/{}/choices", busy.id),
                &CreateChoice {
                    bot_id: "bot-doc".into(),
                    prompt: "装法？".into(),
                    options: vec![
                        ChoiceOption {
                            id: "a".into(),
                            label: "A".into(),
                            primary: false,
                        },
                        ChoiceOption {
                            id: "b".into(),
                            label: "B".into(),
                            primary: false,
                        },
                    ],
                    multi: false,
                },
            );
            let api = HttpApi::new(Conn::new(&base));
            let list = api.unread().unwrap();
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].group_id, busy.id, "most recently active first");
            assert_eq!(list[0].unread, 1);
            assert_eq!(list[1].group_id, quiet.id);
            assert_eq!(list[1].unread, 0);

            let read = api.mark_read(&busy.id).unwrap();
            assert_eq!(read.unread, 0);
            assert!(read.read_at.is_some());
            assert_eq!(api.unread().unwrap()[0].unread, 0);
            assert!(matches!(
                api.mark_read("missing"),
                Err(ApiError::NotFound(_))
            ));
        }

        #[test]
        fn new_bro_gets_default_name_and_a_dm_in_the_list() {
            let base = spawn_host();
            let api = HttpApi::new(Conn::new(&base));
            let first = api.create_bro().unwrap();
            let second = api.create_bro().unwrap();
            assert_eq!(first.name, "bro");
            assert_eq!(second.name, "bro 2");
            let dm = first.dm_group_id.clone().expect("DM created");
            // The DM is an ordinary group: listed, in unread, readable.
            let mut source = crate::source::HttpSource::new(Conn::new(&base), Vec::new());
            use crate::source::EventSource;
            let groups = source.groups().unwrap();
            assert!(groups.iter().any(|g| g.id == dm && g.name == "bro"));
            assert!(api.unread().unwrap().iter().any(|u| u.group_id == dm));
            assert_eq!(api.mark_read(&dm).unwrap().unread, 0);
            let bros = api.bros().unwrap();
            assert_eq!(bros.len(), 2);
            assert_eq!(bros[0].dm_group_id.as_deref(), Some(dm.as_str()));
        }

        /// Events of a group as raw JSON (`type` + fields).
        fn events(base: &str, group: &str) -> Vec<serde_json::Value> {
            let source = crate::source::HttpSource::new(Conn::new(base), Vec::new());
            source
                .events(group, None)
                .unwrap()
                .events
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap())
                .collect()
        }

        #[test]
        fn composer_posts_message_and_queues_task() {
            let base = spawn_host();
            let group = group_with_bot(&base);
            let api = HttpApi::new(Conn::new(&base));
            // Group: the text is stored as a message.
            let sent = api.send_message(&group.id, "@文档哥 写 README").unwrap();
            assert_eq!(sent.body, "@文档哥 写 README");
            assert!(matches!(
                api.send_message(&group.id, "  "),
                Err(ApiError::BadRequest(_))
            ));
            // An @mention queues the rest for that bro.
            let task = api.create_task(&group.id, "bot-doc", "写 README").unwrap();
            assert_eq!(task.state, TaskState::Queued);
            assert!(api.create_task(&group.id, "bot-nobody", "x").is_err());
            let seen = events(&base, &group.id);
            assert!(seen
                .iter()
                .any(|e| e["type"] == "message" && e["body"] == "@文档哥 写 README"));
            assert!(seen
                .iter()
                .any(|e| e["type"] == "status" && e["botId"] == "bot-doc"));

            // DM: the bro behind it gets the task.
            let bro = api.create_bro().unwrap();
            let dm = bro.dm_group_id.clone().unwrap();
            api.send_message(&dm, "整理一下周报").unwrap();
            let task = api.create_task(&dm, &bro.id, "整理一下周报").unwrap();
            assert_eq!(task.assignee_bot_id, bro.id);
            let seen = events(&base, &dm);
            assert!(seen
                .iter()
                .any(|e| e["type"] == "message" && e["body"] == "整理一下周报"));
            assert!(seen
                .iter()
                .any(|e| e["type"] == "status" && e["botId"] == bro.id.as_str()));
        }

        #[test]
        fn rename_bro_renames_its_dm() {
            let base = spawn_host();
            let api = HttpApi::new(Conn::new(&base));
            let bro = api.create_bro().unwrap();
            let dm = bro.dm_group_id.clone().unwrap();
            let renamed = api.rename_bro(&bro.id, "  文档哥 ").unwrap();
            assert_eq!(renamed.name, "文档哥");
            assert_eq!(renamed.dm_group_id.as_deref(), Some(dm.as_str()));
            // The list and the header read the DM group's name.
            let mut source = crate::source::HttpSource::new(Conn::new(&base), Vec::new());
            use crate::source::EventSource;
            let groups = source.groups().unwrap();
            assert!(groups.iter().any(|g| g.id == dm && g.name == "文档哥"));
            assert_eq!(api.bros().unwrap()[0].name, "文档哥");
            // The next default name is free again.
            assert_eq!(api.create_bro().unwrap().name, "bro");
            assert!(matches!(
                api.rename_bro(&bro.id, "   "),
                Err(ApiError::BadRequest(_))
            ));
            assert!(matches!(
                api.rename_bro("nope", "x"),
                Err(ApiError::NotFound(_))
            ));
        }

        #[test]
        fn token_is_sent_on_every_call() {
            let token = "client-test-token-0123456789";
            let base = crate::source::http_test_host_with_token(token);
            let without = HttpApi::new(Conn::new(&base));
            assert!(matches!(without.unread(), Err(ApiError::Status(401, _))));
            let api = HttpApi::new(Conn::new(&base).with_token(Some(token.into())));
            let bro = api.create_bro().unwrap();
            let dm = bro.dm_group_id.unwrap();
            assert_eq!(api.mark_read(&dm).unwrap().unread, 0);
            let mut source = crate::source::HttpSource::new(
                Conn::new(&base).with_token(Some(token.into())),
                Vec::new(),
            );
            use crate::source::EventSource;
            assert!(source.groups().unwrap().iter().any(|g| g.id == dm));
            assert!(source.poll().is_ok());
        }
    }
}
