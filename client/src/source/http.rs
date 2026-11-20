//! Live events from the host over plain HTTP.
//!
//! Base URL: `ROBROZ_HOST`, default `http://127.0.0.1:8790`.
//! Groups: `ROBROZ_GROUPS`, comma-separated `id` or `id=名字`. The host
//! has no `GET /api/groups` yet, so the client cannot discover groups on its
//! own. When that endpoint lands, `groups()` switches to it.
//! Token: `ROBROZ_TOKEN`, sent as `Authorization: Bearer …` when set. Never printed.

use std::time::Duration;

use robroz_contract::Events;

use super::{EventSource, GroupEvent, GroupTab, SourceError};

pub const DEFAULT_HOST: &str = "http://127.0.0.1:8790";

pub struct HttpSource {
    base: String,
    groups: Vec<GroupTab>,
    agent: ureq::Agent,
    token: Option<String>,
}

impl HttpSource {
    pub fn new(base: impl Into<String>, groups: Vec<GroupTab>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            groups,
            agent,
            token: None,
        }
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        self
    }

    pub fn from_env() -> Self {
        let base = std::env::var("ROBROZ_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
        let groups = std::env::var("ROBROZ_GROUPS")
            .map(|v| parse_groups(&v))
            .unwrap_or_default();
        Self::new(base, groups).with_token(std::env::var("ROBROZ_TOKEN").ok())
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// `GET /api/groups/{id}/events`, decoded straight into the contract's
    /// [`Events`]. A shape mismatch is reported, not patched over.
    pub fn events(&self, group_id: &str) -> Result<Events, SourceError> {
        let url = format!("{}/api/groups/{}/events", self.base, group_id);
        let mut request = self.agent.get(&url);
        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        let mut response = request
            .call()
            .map_err(|err| SourceError(format!("连不上宿主机 {}: {err}", self.base)))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|err| SourceError(format!("读 {url} 失败: {err}")))?;
        if status != 200 {
            return Err(SourceError(format!("GET {url} 返回 {status}: {body}")));
        }
        serde_json::from_str::<Events>(&body)
            .map_err(|err| SourceError(format!("GET {url} 与契约不符: {err}")))
    }
}

impl EventSource for HttpSource {
    fn groups(&mut self) -> Result<Vec<GroupTab>, SourceError> {
        if self.groups.is_empty() {
            return Err(SourceError(
                "没有群：宿主机还没有列群接口，请用 ROBROZ_GROUPS=<id>[=名字],… 指定".into(),
            ));
        }
        Ok(self.groups.clone())
    }

    fn poll(&mut self) -> Result<Vec<GroupEvent>, SourceError> {
        let mut out = Vec::new();
        for group in &self.groups {
            for event in self.events(&group.id)?.events {
                out.push(GroupEvent {
                    group_id: group.id.clone(),
                    event,
                });
            }
        }
        Ok(out)
    }

    fn has_more(&self) -> bool {
        true
    }
}

fn parse_groups(value: &str) -> Vec<GroupTab> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|entry| match entry.split_once('=') {
            Some((id, name)) => GroupTab {
                id: id.trim().to_string(),
                name: name.trim().to_string(),
            },
            None => GroupTab {
                id: entry.to_string(),
                name: entry.to_string(),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_group_list() {
        assert_eq!(
            parse_groups(" g1=官网 , g2 ,,"),
            vec![
                GroupTab {
                    id: "g1".into(),
                    name: "官网".into()
                },
                GroupTab {
                    id: "g2".into(),
                    name: "g2".into()
                },
            ]
        );
    }

    #[test]
    fn no_groups_is_an_error_not_an_empty_list() {
        let mut source = HttpSource::new(DEFAULT_HOST, Vec::new());
        assert!(source.groups().is_err());
    }

    #[test]
    fn trailing_slash_is_trimmed() {
        assert_eq!(
            HttpSource::new("http://h:1/", Vec::new()).base(),
            "http://h:1"
        );
    }

    /// Runs the real host router in-process and reads it through
    /// `HttpSource`, so the client decodes exactly what the host sends.
    /// No Docker: tasks stay queued.
    mod against_host {
        use super::*;
        use crate::model::{Feed, Row};
        use robroz_contract::{
            AddMember, CreateGroup, CreateTask, Group, MessageKind, PostMessage, Task, TaskState,
        };

        fn spawn_host() -> String {
            let db = std::env::temp_dir().join(format!(
                "robroz-client-test-{}-{}.db",
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

        #[test]
        fn reads_groups_and_events_from_real_host() {
            let base = spawn_host();
            let group: Group = post(
                &base,
                "/api/groups",
                &CreateGroup {
                    name: "官网".into(),
                },
            );
            let _: serde_json::Value = post(
                &base,
                &format!("/api/groups/{}/members", group.id),
                &AddMember {
                    bot_id: "bot-ada".into(),
                    name: "Ada".into(),
                },
            );
            let _: serde_json::Value = post(
                &base,
                &format!("/api/groups/{}/messages", group.id),
                &PostMessage {
                    kind: MessageKind::Image,
                    body: "hero.png".into(),
                },
            );
            let task: Task = post(
                &base,
                &format!("/api/groups/{}/tasks", group.id),
                &CreateTask {
                    assignee_bot_id: "bot-ada".into(),
                    prompt: "画草图".into(),
                },
            );

            let mut source = HttpSource::new(base, parse_groups(&format!("{}=官网", group.id)));
            assert_eq!(source.groups().unwrap()[0].name, "官网");

            let mut feed = Feed::default();
            for _ in 0..2 {
                // Two full snapshots: the second must not duplicate rows.
                for ge in source.poll().unwrap() {
                    assert_eq!(ge.group_id, group.id);
                    feed.apply(&ge.event);
                }
            }
            let rows: Vec<Row> = feed.rows().cloned().collect();
            assert_eq!(rows.len(), 2, "{rows:?}");
            assert!(
                matches!(&rows[0], Row::Attachment { kind: MessageKind::Image, name } if name == "hero.png")
            );
            match &rows[1] {
                Row::Status(card) => {
                    assert_eq!(card.task_id, task.id);
                    assert_eq!(card.bot_name, "Ada");
                    assert_eq!(
                        card.status_line,
                        robroz_card::status_line(TaskState::Queued, false)
                    );
                }
                other => panic!("expected status card, got {other:?}"),
            }
        }

        #[test]
        fn unknown_group_is_an_error() {
            let base = spawn_host();
            let mut source = HttpSource::new(base, parse_groups("nope"));
            let err = source.poll().unwrap_err();
            assert!(err.0.contains("404"), "{err}");
        }

        #[test]
        fn host_down_is_an_error() {
            // Bind then drop to get a port nothing listens on.
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let mut source = HttpSource::new(format!("http://127.0.0.1:{port}"), parse_groups("g"));
            assert!(source.poll().is_err());
        }
    }
}
