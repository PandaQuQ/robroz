//! Single responder: each rule, the model fallbacks, and the HTTP behaviour.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::Host;

// ---- pure pieces ----

fn cand(id: &str, name: &str) -> Candidate {
    Candidate {
        id: id.into(),
        name: name.into(),
        profile: None,
    }
}

fn ctx(members: &[(&str, &str)], waiting: &[&str], recent: &[&str]) -> Context {
    Context {
        members: members.iter().map(|(i, n)| cand(i, n)).collect(),
        waiting: waiting.iter().map(|s| s.to_string()).collect(),
        recent: recent.iter().map(|s| s.to_string()).collect(),
    }
}

/// A scripted [`Selector`] that records what it was asked.
struct Scripted {
    answer: Mutex<Result<String, String>>,
    delay: Duration,
    asked: Mutex<Vec<(Vec<String>, String)>>,
}

impl Scripted {
    fn answering(id: &str) -> Arc<Self> {
        Self::with(Ok(id.into()), Duration::ZERO)
    }

    fn failing() -> Arc<Self> {
        Self::with(Err("boom".into()), Duration::ZERO)
    }

    fn with(answer: Result<String, String>, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            delay,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}

impl Selector for Scripted {
    fn pick(&self, candidates: &[Candidate], message: &str) -> Result<String, String> {
        self.asked.lock().unwrap().push((
            candidates.iter().map(|c| c.id.clone()).collect(),
            message.to_string(),
        ));
        std::thread::sleep(self.delay);
        self.answer.lock().unwrap().clone()
    }
}

fn as_dyn(s: &Arc<Scripted>) -> Option<Arc<dyn Selector>> {
    Some(s.clone() as Arc<dyn Selector>)
}

const FAST: Duration = Duration::from_millis(100);

#[test]
fn the_flag_is_on_unless_switched_off() {
    for on in [
        None,
        Some(""),
        Some("1"),
        Some("true"),
        Some("yes"),
        Some("x"),
    ] {
        assert!(parse_flag(on), "{on:?}");
    }
    for off in ["0", " 0 ", "false", "FALSE", "off", "No"] {
        assert!(!parse_flag(Some(off)), "{off:?}");
    }
}

#[test]
fn rule_one_a_single_waiting_bro_is_the_follow_up() {
    let c = ctx(&[("a", "Ada"), ("b", "Bob"), ("c", "Cara")], &["b"], &["a"]);
    let model = Scripted::answering("c");
    let d = choose(&c, "好的就这样", as_dyn(&model).as_ref(), FAST);
    assert_eq!((d.bot_id.as_str(), d.rule), ("b", Rule::FollowUp));
    assert_eq!(model.calls(), 0, "no model call for a follow-up");
}

#[test]
fn rule_one_with_several_waiting_bros_limits_the_model_to_them() {
    let c = ctx(
        &[("a", "Ada"), ("b", "Bob"), ("c", "Cara")],
        &["b", "c"],
        &[],
    );
    let model = Scripted::answering("c");
    let d = choose(&c, "第二个", as_dyn(&model).as_ref(), FAST);
    assert_eq!((d.bot_id.as_str(), d.rule), ("c", Rule::Model));
    assert_eq!(model.asked.lock().unwrap()[0].0, vec!["b", "c"]);

    // A bro that is not waiting is not a valid answer then.
    let model = Scripted::answering("a");
    let d = choose(&c, "第二个", as_dyn(&model).as_ref(), FAST);
    assert_eq!(d.model_failure, Some(ModelFailure::InvalidId));
    assert_eq!(d.bot_id, "b", "first of the waiting bros");
}

#[test]
fn rule_two_one_bro_in_the_group_is_it() {
    let c = ctx(&[("a", "Ada")], &[], &[]);
    let model = Scripted::answering("zzz");
    let d = choose(&c, "你好", as_dyn(&model).as_ref(), FAST);
    assert_eq!((d.bot_id.as_str(), d.rule), ("a", Rule::OnlyBro));
    assert_eq!(model.calls(), 0);
}

#[test]
fn rule_three_the_model_picks_one_bro() {
    let c = ctx(&[("a", "Ada"), ("b", "Bob")], &[], &["a"]);
    let model = Scripted::answering(" b ");
    let d = choose(&c, "帮我查一下天气", as_dyn(&model).as_ref(), FAST);
    assert_eq!(d.rule, Rule::Model);
    assert_eq!(d.bot_id, "b");
    assert_eq!(d.model_failure, None);
    let asked = model.asked.lock().unwrap();
    assert_eq!(asked.len(), 1, "asked exactly once");
    assert_eq!(asked[0].0, vec!["a", "b"]);
    assert_eq!(asked[0].1, "帮我查一下天气");
}

#[test]
fn rule_four_falls_back_on_error_timeout_invalid_id_and_no_model() {
    let c = ctx(
        &[("a", "Ada"), ("b", "Bob"), ("c", "Cara")],
        &[],
        &["x", "c", "a"],
    );

    let d = choose(&c, "hi", as_dyn(&Scripted::failing()).as_ref(), FAST);
    assert_eq!(
        (d.bot_id.as_str(), d.rule, d.model_failure),
        ("c", Rule::LastAnswered, Some(ModelFailure::Error)),
        "the most recent answerer that is still a member"
    );

    let slow = Scripted::with(Ok("a".into()), Duration::from_millis(600));
    let started = std::time::Instant::now();
    let d = choose(&c, "hi", as_dyn(&slow).as_ref(), FAST);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "gave up on time"
    );
    assert_eq!(
        (d.bot_id.as_str(), d.rule, d.model_failure),
        ("c", Rule::LastAnswered, Some(ModelFailure::Timeout))
    );

    for bad in ["nobody", "", "Ada", "a b"] {
        let d = choose(&c, "hi", as_dyn(&Scripted::answering(bad)).as_ref(), FAST);
        assert_eq!(d.model_failure, Some(ModelFailure::InvalidId), "{bad:?}");
        assert_eq!(d.bot_id, "c");
    }

    let d = choose(&c, "hi", None, FAST);
    assert_eq!(d.model_failure, Some(ModelFailure::NoModel));
    assert_eq!(d.bot_id, "c");

    // Nobody has answered yet: the first bro by join order.
    let fresh = ctx(&[("a", "Ada"), ("b", "Bob")], &[], &[]);
    let d = choose(&fresh, "hi", None, FAST);
    assert_eq!((d.bot_id.as_str(), d.rule), ("a", Rule::FirstJoined));
}

#[test]
fn the_model_answer_must_be_strict_json_with_one_bro_field() {
    assert_eq!(answer_id(r#"{"bro":"b-1"}"#).as_deref(), Some("b-1"));
    assert_eq!(
        answer_id("  {\"bro\": \" b-1 \"}\n").as_deref(),
        Some("b-1")
    );
    assert_eq!(
        answer_id("```json\n{\"bro\":\"b-1\"}\n```").as_deref(),
        Some("b-1")
    );
    for bad in [
        "b-1",
        "I pick b-1",
        r#"Sure! {"bro":"b-1"}"#,
        r#"{"bro":"b-1"} thanks"#,
        r#"{"bro":1}"#,
        r#"{"id":"b-1"}"#,
        r#"{"bro":"b-1","why":"x"}"#,
        r#"["b-1"]"#,
        "{",
        "",
    ] {
        assert_eq!(answer_id(bad), None, "{bad:?}");
    }
}

#[test]
fn the_prompt_lists_names_and_profiles_and_clips_the_message() {
    let mut ada = cand("b-ada", "Ada");
    ada.profile = Some("writes weekly reports".into());
    let text = prompt(
        &[ada, cand("b-bob", "Bob")],
        &format!("hi {}", "x".repeat(2000)),
    );
    assert!(text.contains("- id: b-ada | name: Ada | about: writes weekly reports"));
    assert!(text.contains("- id: b-bob | name: Bob\n"), "{text}");
    assert!(text.contains(r#"{"bro":"<id>"}"#));
    assert!(text.chars().count() < 1400, "{}", text.chars().count());
}

#[test]
fn the_log_line_has_ids_and_the_rule_but_never_the_message() {
    let secret = "my-secret-message-text";
    let c = ctx(&[("a", "Ada"), ("b", "Bob")], &[], &[]);
    let d = choose(&c, secret, as_dyn(&Scripted::failing()).as_ref(), FAST);
    let line = d.line("g-1");
    assert_eq!(
        line,
        "responder: group g-1 bro a by first_joined (model: error)"
    );
    assert!(!line.contains(secret));
    let ok = choose(&c, secret, as_dyn(&Scripted::answering("b")).as_ref(), FAST);
    assert_eq!(ok.line("g-1"), "responder: group g-1 bro b by model");
}

// ---- over HTTP ----

struct H {
    db: PathBuf,
    app: Router,
}

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("robroz-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn harness(on: bool, selector: Option<&Arc<Scripted>>) -> H {
    let dir = temp("responder");
    let db = dir.join("robroz.db");
    let mut host = Host::open(&db, dir.join("volumes"), "robroz-runner:local")
        .unwrap()
        .with_control_root(dir.join("control"))
        .with_single_responder(on);
    if let Some(s) = selector {
        host = host.with_selector(s.clone());
    }
    H {
        db,
        app: host.router(),
    }
}

impl H {
    fn conn(&self) -> Connection {
        Connection::open(&self.db).unwrap()
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let resp = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn group(&self, bros: &[(&str, &str)]) -> String {
        let (_, g) = self
            .call("POST", "/api/groups", Some(json!({ "name": "研发" })))
            .await;
        let id = g["id"].as_str().unwrap().to_string();
        for (bot, name) in bros {
            let (status, body) = self
                .call(
                    "POST",
                    &format!("/api/groups/{id}/members"),
                    Some(json!({ "botId": bot, "name": name })),
                )
                .await;
            assert!(status.is_success(), "{body}");
        }
        id
    }

    async fn say(&self, group: &str, text: &str) -> Value {
        let (status, said) = self
            .call(
                "POST",
                &format!("/api/groups/{group}/say"),
                Some(json!({ "body": text })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{said}");
        said
    }

    fn n(&self, sql: &str, id: &str) -> i64 {
        self.conn().query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    fn tasks(&self, group: &str) -> i64 {
        self.n("SELECT COUNT(*) FROM tasks WHERE group_id = ?1", group)
    }

    fn turns(&self, group: &str) -> i64 {
        self.n("SELECT COUNT(*) FROM turns WHERE group_id = ?1", group)
    }
}

const THREE: [(&str, &str); 3] = [("b-ada", "Ada"), ("b-bob", "Bob"), ("b-cara", "Cara")];

#[tokio::test]
async fn a_message_that_names_nobody_makes_exactly_one_task_for_the_picked_bro() {
    let model = Scripted::answering("b-bob");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let said = h.say(&g, "帮我整理一下周报").await;

    assert_eq!(said["task"]["assigneeBotId"], "b-bob", "{said}");
    assert_eq!(
        said["task"]["prompt"], "帮我整理一下周报",
        "no mention to strip"
    );
    assert_eq!(said["task"]["state"], "queued");
    assert!(said.get("appendedToTaskId").is_none(), "{said}");
    assert_eq!(h.tasks(&g), 1, "one task, not one per bro");
    assert_eq!(h.turns(&g), 1, "only the picked bro has a turn");
    assert_eq!(
        h.n(
            "SELECT COUNT(*) FROM turns WHERE group_id = ?1 AND single = 1 AND task_id IS NOT NULL",
            &g
        ),
        1
    );
    assert_eq!(
        h.n(
            "SELECT COUNT(*) FROM tasks WHERE group_id = ?1 AND turn_hidden = 1",
            &g
        ),
        0,
        "the picked bro's card is visible at once"
    );
    assert_eq!(model.calls(), 1);
    assert_eq!(
        model.asked.lock().unwrap()[0].0,
        vec!["b-ada", "b-bob", "b-cara"]
    );

    // A second message makes one more task, still one per message.
    let again = h.say(&g, "再加一条").await;
    assert!(again["task"].is_object());
    assert_eq!(h.turns(&g), 2);
}

#[tokio::test]
async fn deleting_a_group_drops_its_single_responder_rounds_and_tasks() {
    let model = Scripted::answering("b-bob");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let other = h.group(&THREE).await;
    h.say(&g, "帮我整理一下周报").await;
    h.say(&other, "另一个群里的话").await;
    assert_eq!((h.turns(&g), h.tasks(&g)), (1, 1));

    let (status, body) = h.call("DELETE", &format!("/api/groups/{g}"), None).await;
    assert!(status.is_success(), "{status} {body}");
    // The responder keeps nothing in memory: its state is the `turns` row
    // (with `single`) and the task, and the group delete removes both.
    assert_eq!((h.turns(&g), h.tasks(&g)), (0, 0));
    assert_eq!(
        (h.turns(&other), h.tasks(&other)),
        (1, 1),
        "other groups are untouched"
    );

    // A later message in another group still picks one bro.
    let again = h.say(&other, "再来一条").await;
    assert!(again["task"].is_object(), "{again}");
    assert_eq!(h.turns(&other), 2);
}

#[tokio::test]
async fn a_group_with_one_bro_goes_to_it_without_asking_a_model() {
    let model = Scripted::answering("b-zzz");
    let h = harness(true, Some(&model));
    let g = h.group(&[("b-ada", "Ada")]).await;
    let said = h.say(&g, "你好").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-ada");
    assert_eq!(model.calls(), 0);
    assert_eq!(h.tasks(&g), 1);
}

#[tokio::test]
async fn a_bro_with_an_open_task_gets_the_follow_up_without_asking_a_model() {
    let model = Scripted::answering("b-cara");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let (status, task) = h
        .call(
            "POST",
            &format!("/api/groups/{g}/tasks"),
            Some(json!({ "assigneeBotId": "b-bob", "prompt": "写周报" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");

    let said = h.say(&g, "再加上上周的数据").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-bob", "{said}");
    assert_eq!(said["appendedToTaskId"], task["id"], "joins the open task");
    assert_eq!(model.calls(), 0);
    assert_eq!(h.tasks(&g), 1, "no new task");
    assert_eq!(
        h.n(
            "SELECT COUNT(*) FROM follow_ups WHERE task_id = ?1",
            task["id"].as_str().unwrap()
        ),
        1
    );
}

#[tokio::test]
async fn an_unanswered_choice_card_makes_its_bro_the_follow_up() {
    let model = Scripted::answering("b-ada");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let (status, card) = h
        .call(
            "POST",
            &format!("/api/groups/{g}/choices"),
            Some(json!({ "botId": "b-cara", "prompt": "用哪个？", "options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{card}");
    let said = h.say(&g, "选 A 吧").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-cara", "{said}");
    assert_eq!(model.calls(), 0);
}

#[tokio::test]
async fn a_model_failure_falls_back_to_the_last_answerer_then_the_first_bro() {
    let model = Scripted::failing();
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;

    let said = h.say(&g, "第一条").await;
    assert_eq!(
        said["task"]["assigneeBotId"], "b-ada",
        "first by join order"
    );

    // Cara delivered the last reply here.
    {
        let conn = h.conn();
        conn.execute("UPDATE tasks SET state = 'done' WHERE group_id = ?1", [&g])
            .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at)
             VALUES ('t-cara', ?1, 'b-cara', 'x', 'done', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [&g],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at)
             VALUES ('d-1', ?1, 't-cara', '好了', 'r.txt', 0, '2099-01-01T00:00:00.000Z')",
            [&g],
        )
        .unwrap();
    }
    let said = h.say(&g, "第二条").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-cara", "{said}");
    assert_eq!(model.calls(), 2);
}

#[tokio::test]
async fn an_invalid_model_id_falls_back_and_still_makes_one_task() {
    let model = Scripted::answering("b-nobody");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let said = h.say(&g, "谁来做").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-ada");
    assert_eq!(h.tasks(&g), 1);
    assert_eq!(h.turns(&g), 1);
}

#[tokio::test]
async fn a_slow_model_does_not_hold_the_database_for_other_requests() {
    let model = Scripted::with(Ok("b-bob".into()), Duration::from_millis(700));
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let (said, waited) = tokio::join!(h.say(&g, "慢一点"), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = std::time::Instant::now();
        let (status, _) = h.call("GET", "/api/groups", None).await;
        assert_eq!(status, StatusCode::OK);
        started.elapsed()
    });
    assert_eq!(said["task"]["assigneeBotId"], "b-bob");
    assert!(
        waited < Duration::from_millis(300),
        "a second request waited {waited:?} behind the model call"
    );
}

#[tokio::test]
async fn an_at_mention_behaves_as_before() {
    let model = Scripted::answering("b-cara");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let said = h.say(&g, "@Bob 写个方案").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-bob");
    assert_eq!(
        said["task"]["prompt"], "写个方案",
        "mention stripped as today"
    );
    assert_eq!(model.calls(), 0, "a named bro is never re-routed");
    assert_eq!(h.turns(&g), 3, "every bro still has a turn in the round");
    assert_eq!(
        h.n(
            "SELECT COUNT(*) FROM turns WHERE group_id = ?1 AND single = 1",
            &g
        ),
        0
    );
    assert_eq!(h.tasks(&g), 1, "the others wait behind the named bro");
}

#[tokio::test]
async fn a_leading_name_that_is_not_a_member_names_nobody() {
    let model = Scripted::answering("b-cara");
    let h = harness(true, Some(&model));
    let g = h.group(&THREE).await;
    let said = h.say(&g, "@Zed 你好").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-cara");
    assert_eq!(model.calls(), 1);
}

#[tokio::test]
async fn with_the_flag_off_every_bro_takes_a_turn_as_before() {
    let model = Scripted::answering("b-bob");
    let h = harness(false, Some(&model));
    let g = h.group(&THREE).await;
    let said = h.say(&g, "大家好").await;
    assert!(said["task"].is_null(), "{said}");
    assert_eq!(h.turns(&g), 3);
    assert_eq!(model.calls(), 0);
    assert_eq!(
        h.n(
            "SELECT COUNT(*) FROM turns WHERE group_id = ?1 AND single = 1",
            &g
        ),
        0
    );
}

#[tokio::test]
async fn a_dm_is_unaffected() {
    let model = Scripted::answering("b-zzz");
    let h = harness(true, Some(&model));
    let (status, bro) = h
        .call("POST", "/api/bros", Some(json!({ "name": "Ada" })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{bro}");
    let dm = bro["dmGroupId"].as_str().unwrap();
    let said = h.say(dm, "你好").await;
    assert_eq!(said["task"]["assigneeBotId"], bro["id"]);
    assert_eq!(model.calls(), 0);
    assert_eq!(h.turns(dm), 0, "DMs never have rounds");
}

#[tokio::test]
async fn without_any_model_the_first_bro_answers() {
    let h = harness(true, None);
    let g = h.group(&THREE).await;
    let said = h.say(&g, "有人吗").await;
    assert_eq!(said["task"]["assigneeBotId"], "b-ada");
    assert_eq!(h.tasks(&g), 1);
}

#[tokio::test]
async fn a_group_without_bros_and_an_unknown_group_behave_as_before() {
    let model = Scripted::answering("b-ada");
    let h = harness(true, Some(&model));
    let g = h.group(&[]).await;
    let said = h.say(&g, "有人吗").await;
    assert!(said["task"].is_null());
    assert_eq!(h.tasks(&g), 0);
    let (status, body) = h
        .call("POST", "/api/groups/nope/say", Some(json!({ "body": "x" })))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(model.calls(), 0);
}

#[test]
fn context_reads_waiting_bros_profiles_and_recent_answers() {
    let dir = temp("responder-ctx");
    let host = Host::open(&dir.join("db"), dir.join("v"), "x").unwrap();
    drop(host);
    let conn = Connection::open(dir.join("db")).unwrap();
    for (id, name) in [("a", "Ada"), ("b", "Bob")] {
        conn.execute(
            "INSERT INTO bots (id, tenant_id, name) VALUES (?1, 'local', ?2)",
            params![id, name],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memberships (group_id, bot_id) VALUES ('g', ?1)",
            [id],
        )
        .unwrap();
    }
    let at = "2026-01-01T00:00:00.000Z";
    for (id, scope, kind, status, text) in [
        ("m1", "bot:a", "profile", "kept", "writes   weekly\nreports"),
        ("m2", "bot:a", "habit", "kept", "ignored: not a profile"),
        ("m3", "bot:b", "profile", "pending", "ignored: not kept"),
    ] {
        conn.execute(
            "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'user', ?5, ?6, ?6)",
            params![id, scope, kind, text, status, at],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, turn_hidden)
         VALUES ('t1', 'g', 'b', 'p', 'blocked', ?1, ?1, 0), ('t2', 'g', 'a', 'p', 'queued', ?1, ?1, 1),
                ('t3', 'g', 'a', 'p', 'done', ?1, ?1, 0)",
        [at],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO deliveries (id, group_id, task_id, summary, path, briefing, created_at)
         VALUES ('d1', 'g', 't3', 's', 'p', 0, ?1)",
        [at],
    )
    .unwrap();

    let ctx = context(&conn, "g").unwrap();
    assert_eq!(
        ctx.members
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(
        ctx.members[0].profile.as_deref(),
        Some("writes weekly reports")
    );
    assert_eq!(ctx.members[1].profile, None);
    assert_eq!(
        ctx.waiting,
        vec!["b"],
        "a hidden turn and a done task do not count"
    );
    assert_eq!(ctx.recent, vec!["a"]);
}
