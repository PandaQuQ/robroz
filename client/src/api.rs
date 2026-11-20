//! Calls the client makes when someone clicks a card or a routine toggle.
//!
//! All endpoint paths live in [`paths`], so switching to final backend names
//! is a one-place change.
//! - `choice` and routines use robroz#15: `POST /api/choices/:id/answer`,
//!   `GET /api/groups/:id/schedules`, `PATCH /api/schedules/:id`.
//! - `approval` and `draft` have no backend endpoints yet. They use the UI
//!   SPEC §9 suggestions (`POST /api/cards/:id/decision`,
//!   `POST /api/cards/:id/send`) until the backend defines them.

use std::sync::Mutex;
use std::time::Duration;

use robroz_contract::{AnswerChoice, ErrorBody, Event, ScheduleEntry, Schedules, UpdateSchedule};
use robroz_ui::cards::interact::Note;

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
    /// Returns the updated `choice` event when the host sends it back.
    fn answer_choice(&self, id: &str, choice: &[String]) -> Result<Option<Event>, ApiError>;
    fn decide(&self, id: &str, approve: bool) -> Result<(), ApiError>;
    fn send_draft(&self, id: &str) -> Result<(), ApiError>;
    fn schedules(&self, group_id: &str) -> Result<Vec<ScheduleEntry>, ApiError>;
    fn set_paused(&self, id: &str, paused: bool) -> Result<ScheduleEntry, ApiError>;
}

pub struct HttpApi {
    base: String,
    agent: ureq::Agent,
}

impl HttpApi {
    pub fn new(base: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            agent,
        }
    }

    fn finish(
        &self,
        method: &str,
        url: &str,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<String, ApiError> {
        let mut response =
            result.map_err(|e| ApiError::Transport(format!("连不上宿主机 {}: {e}", self.base)))?;
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
        let url = format!("{}{path}", self.base);
        let req = self.agent.post(&url);
        let result = match body {
            Some(b) => req.header("content-type", "application/json").send(b),
            None => req.send_empty(),
        };
        self.finish("POST", &url, result)
    }

    fn patch(&self, path: &str, body: String) -> Result<String, ApiError> {
        let url = format!("{}{path}", self.base);
        let result = self
            .agent
            .patch(&url)
            .header("content-type", "application/json")
            .send(body);
        self.finish("PATCH", &url, result)
    }

    fn get(&self, path: &str) -> Result<String, ApiError> {
        let url = format!("{}{path}", self.base);
        let result = self.agent.get(&url).call();
        self.finish("GET", &url, result)
    }
}

fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Decode(format!("与契约不符: {e}")))
}

impl CardApi for HttpApi {
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
/// schedules in memory.
pub struct FakeApi {
    schedules: Mutex<Vec<ScheduleEntry>>,
}

impl FakeApi {
    pub fn new(schedules: Vec<ScheduleEntry>) -> Self {
        Self {
            schedules: Mutex::new(schedules),
        }
    }
}

impl CardApi for FakeApi {
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
        let api = HttpApi::new(format!("http://127.0.0.1:{port}"));
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
            let db = std::env::temp_dir().join(format!(
                "robroz-client-api-test-{}-{}.db",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let router = robroz_host::router(&db).unwrap();
            let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            std_listener.set_nonblocking(true).unwrap();
            let addr = std_listener.local_addr().unwrap();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
                    axum::serve(listener, router).await.unwrap();
                });
            });
            format!("http://{addr}")
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
            let api = HttpApi::new(&base);
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
            let api = HttpApi::new(&base);
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
    }
}
