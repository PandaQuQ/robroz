//! Scripted events for offline runs (`--fake`) and unit tests. No network.

use std::time::Duration;

use robroz_contract::{Event, MessageKind, TaskState};

use super::{EventSource, GroupEvent, GroupTab, SourceError};

/// Scripted events, released one step per `poll`. No network.
pub struct FakeSource {
    groups: Vec<GroupTab>,
    steps: Vec<Vec<GroupEvent>>,
    next: usize,
}

const SITE: &str = "g-site";
const OPS: &str = "g-ops";

impl FakeSource {
    pub fn new() -> Self {
        let groups = vec![group(SITE, "公司官网建设"), group(OPS, "运营日报")];
        let steps = vec![
            vec![
                msg(
                    SITE,
                    "m1",
                    MessageKind::Text,
                    "首页 hero 换成大透明 logo，手机上也要清楚。",
                ),
                msg(SITE, "m2", MessageKind::Image, "hero-draft.png"),
                status(SITE, "t-hero", "bot-ada", "Ada", TaskState::Queued, false),
                msg(
                    OPS,
                    "m3",
                    MessageKind::Text,
                    "每天早上 8 点出一份询价简报。",
                ),
                status(OPS, "t-crawl", "bot-cy", "Cy", TaskState::Queued, false),
            ],
            vec![status(
                SITE,
                "t-hero",
                "bot-ada",
                "Ada",
                TaskState::Running,
                true,
            )],
            vec![
                msg(SITE, "m4", MessageKind::File, "brand-guide.pdf"),
                status(SITE, "t-copy", "bot-bea", "Bea", TaskState::Queued, false),
            ],
            vec![status(
                OPS,
                "t-crawl",
                "bot-cy",
                "Cy",
                TaskState::Running,
                true,
            )],
            vec![status(
                SITE,
                "t-copy",
                "bot-bea",
                "Bea",
                TaskState::Blocked,
                false,
            )],
            vec![
                status(SITE, "t-hero", "bot-ada", "Ada", TaskState::Done, false),
                delivery(SITE, "d1", "已写入结果", "tasks/t-hero/result.txt", false),
            ],
            vec![status(
                OPS,
                "t-crawl",
                "bot-cy",
                "Cy",
                TaskState::Running,
                false,
            )],
            // Node 5: a schedule fires a normal task. Its card is a normal
            // status card and its result is a normal delivery with
            // `briefing: true`. No separate card kind.
            vec![status(
                OPS,
                "t-brief",
                "bot-dee",
                "Dee",
                TaskState::Queued,
                false,
            )],
            vec![status(
                OPS,
                "t-brief",
                "bot-dee",
                "Dee",
                TaskState::Running,
                true,
            )],
            vec![
                status(OPS, "t-brief", "bot-dee", "Dee", TaskState::Done, false),
                delivery(OPS, "d2", "已写入结果", "tasks/t-brief/result.txt", true),
            ],
        ];
        Self {
            groups,
            steps,
            next: 0,
        }
    }
}

impl Default for FakeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for FakeSource {
    fn groups(&mut self) -> Result<Vec<GroupTab>, SourceError> {
        Ok(self.groups.clone())
    }

    fn poll(&mut self) -> Result<Vec<GroupEvent>, SourceError> {
        Ok(match self.steps.get(self.next) {
            Some(step) => {
                self.next += 1;
                step.clone()
            }
            None => Vec::new(),
        })
    }

    fn has_more(&self) -> bool {
        self.next < self.steps.len()
    }

    fn interval(&self) -> Duration {
        Duration::from_millis(900)
    }
}

fn group(id: &str, name: &str) -> GroupTab {
    GroupTab {
        id: id.into(),
        name: name.into(),
    }
}

fn at(seq: &str) -> String {
    format!("2026-10-07T19:00:{seq}.000Z")
}

fn msg(group: &str, id: &str, kind: MessageKind, body: &str) -> GroupEvent {
    GroupEvent {
        group_id: group.into(),
        event: Event::Message {
            id: id.into(),
            kind,
            body: body.into(),
            created_at: at("00"),
        },
    }
}

fn status(
    group: &str,
    task: &str,
    bot_id: &str,
    bot_name: &str,
    state: TaskState,
    container_up: bool,
) -> GroupEvent {
    GroupEvent {
        group_id: group.into(),
        event: Event::Status {
            id: task.into(),
            task_id: task.into(),
            bot_id: bot_id.into(),
            bot_name: bot_name.into(),
            state,
            label: state.label().into(),
            created_at: at("01"),
            updated_at: at("02"),
            container_up,
        },
    }
}

fn delivery(group: &str, id: &str, summary: &str, path: &str, briefing: bool) -> GroupEvent {
    GroupEvent {
        group_id: group.into(),
        event: Event::Delivery {
            id: id.into(),
            summary: summary.into(),
            path: path.into(),
            briefing,
            created_at: at("03"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Feed, Row};
    use std::collections::HashMap;

    fn drain() -> HashMap<String, Feed> {
        let mut source = FakeSource::new();
        let mut feeds: HashMap<String, Feed> = HashMap::new();
        while source.has_more() {
            for ge in source.poll().unwrap() {
                feeds.entry(ge.group_id).or_default().apply(&ge.event);
            }
        }
        assert!(source.poll().unwrap().is_empty());
        feeds
    }

    #[test]
    fn fake_script_covers_every_case() {
        let mut source = FakeSource::new();
        assert_eq!(source.groups().unwrap().len(), 2);
        let feeds = drain();
        let site: Vec<_> = feeds[SITE].rows().cloned().collect();
        let ops: Vec<_> = feeds[OPS].rows().cloned().collect();

        let status_of = |rows: &[Row], bot: &str| {
            rows.iter()
                .find_map(|r| match r {
                    Row::Status(c) if c.bot_name == bot => Some(c.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let count_status = |rows: &[Row], bot: &str| {
            rows.iter()
                .filter(|r| matches!(r, Row::Status(c) if c.bot_name == bot))
                .count()
        };

        assert_eq!(count_status(&site, "Ada"), 1);
        assert_eq!(
            status_of(&site, "Ada").status_line,
            robroz_card::status_line(TaskState::Done, false)
        );
        assert_eq!(
            status_of(&site, "Bea").action,
            robroz_card::appearance(TaskState::Blocked, false).action
        );
        assert_eq!(
            status_of(&ops, "Cy").status_line,
            robroz_card::status_line(TaskState::Running, false)
        );
        assert!(site
            .iter()
            .any(|r| matches!(r, Row::Delivery(d) if d.badge.is_none())));
        assert!(ops.iter().any(
            |r| matches!(r, Row::Delivery(d) if d.badge == Some(robroz_card::briefing_badge()))
        ));
        assert_eq!(count_status(&ops, "Dee"), 1);
        assert_eq!(
            status_of(&ops, "Dee").status_line,
            robroz_card::status_line(TaskState::Done, false)
        );
        assert!(site.iter().any(|r| matches!(
            r,
            Row::Attachment {
                kind: MessageKind::Image,
                ..
            }
        )));
        assert!(site.iter().any(|r| matches!(
            r,
            Row::Attachment {
                kind: MessageKind::File,
                ..
            }
        )));
    }
}
