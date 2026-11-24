//! Auto-review: rule endpoints, the review pass, and approval expiry.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use robroz_contract::APPROVAL_EXPIRED_REASON;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::reviewer::{parse_answer, prompt, review_with_retry, NO_MATCH_REASON, REVIEW_FAILED_REASON};
use super::*;
use crate::Host;

fn temp(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("robroz-{prefix}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

struct Harness {
    host: Host,
    app: Router,
    control: PathBuf,
}

fn harness() -> Harness {
    let dir = temp("review");
    let control = temp("review-control");
    let host = Host::open(&dir.join("robroz.db"), dir.join("groups"), "robroz-runner:local")
        .unwrap()
        .with_control_root(control.clone());
    let app = host.router();
    Harness { host, app, control }
}

impl Harness {
    fn with_reviewer(self, reviewer: Arc<dyn Reviewer>) -> Self {
        let host = self.host.with_reviewer(reviewer);
        let app = host.router();
        Harness { host, app, control: self.control }
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.host.db.lock().unwrap()
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
        let resp = self.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
        (status, value)
    }

    /// A bro and its DM group id.
    async fn bro(&self, name: &str) -> (String, String) {
        let (status, bro) = self.call("POST", "/api/bros", Some(json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::CREATED, "{bro}");
        (bro["id"].as_str().unwrap().into(), bro["dmGroupId"].as_str().unwrap().into())
    }

    async fn approvals(&self, group_id: &str) -> Vec<Value> {
        let (status, body) = self.call("GET", &format!("/api/groups/{group_id}/events"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["events"].as_array().unwrap().iter().filter(|e| e["type"] == "approval").cloned().collect()
    }

    async fn rules(&self) -> Vec<Value> {
        let (status, body) = self.call("GET", "/api/settings/review-rules", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["rules"].as_array().unwrap().clone()
    }

    async fn propose(&self, text: &str) -> String {
        let (status, body) = self.call("POST", "/api/settings/review-rules", Some(json!({ "text": text }))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(body.as_object().unwrap().len(), 1, "only approvalId: {body}");
        body["approvalId"].as_str().unwrap().into()
    }

    async fn answer(&self, id: &str, verb: &str) -> (StatusCode, Value) {
        self.call("POST", &format!("/api/approvals/{id}/{verb}"), None).await
    }

    fn add_rule(&self, text: &str) {
        self.db()
            .execute(
                "INSERT INTO review_rules (id, text, created_at) VALUES (?1, ?2, ?3)",
                params![uuid::Uuid::new_v4().to_string(), text, crate::now_rfc3339()],
            )
            .unwrap();
    }

    /// A task blocked on `robroz-action:external`, with its approval card opened
    /// the way the pump opens it. Returns (task id, approval id, hash).
    fn blocked_task(&self, group_id: &str, bot_id: &str) -> (String, String, String) {
        let conn = self.db();
        let task_id = uuid::Uuid::new_v4().to_string();
        let hash = format!("hash-{task_id}");
        let now = crate::now_rfc3339();
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, intent_kind, intent_line, intent_hash)
             VALUES (?1, ?2, ?3, '发周报给老板\nrobroz-action:external', 'blocked', ?4, ?4, 'external', 'robroz-action:external', ?5)",
            params![task_id, group_id, bot_id, now, hash],
        )
        .unwrap();
        crate::approvals::gate_task(&conn, &task_id, "发周报给老板\nrobroz-action:external", &hash, &now).unwrap();
        let approval: String = conn
            .query_row("SELECT id FROM approvals WHERE task_id = ?1", [&task_id], |r| r.get(0))
            .unwrap();
        (task_id, approval, hash)
    }

    fn task(&self, task_id: &str) -> (String, Option<String>) {
        self.db()
            .query_row("SELECT state, failure FROM tasks WHERE id = ?1", [task_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
    }

    fn row(&self, approval_id: &str) -> (Option<String>, Option<String>, Option<String>) {
        self.db()
            .query_row(
                "SELECT decision, reason, review_state FROM approvals WHERE id = ?1",
                [approval_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    fn expire(&self, after: Duration) -> Vec<String> {
        let mut conn = self.db();
        expire_due(&mut conn, &self.control, Utc::now() + after).unwrap()
    }
}

/// A reviewer that answers from a script and counts calls.
struct Fake {
    answers: Mutex<Vec<Result<Verdict, String>>>,
    calls: AtomicUsize,
    seen: Mutex<Vec<(String, Vec<String>)>>,
}

impl Fake {
    fn new(answers: Vec<Result<Verdict, String>>) -> Arc<Self> {
        Arc::new(Self { answers: Mutex::new(answers), calls: AtomicUsize::new(0), seen: Mutex::new(Vec::new()) })
    }
}

impl Reviewer for Fake {
    fn review(&self, action: &str, rules: &[String]) -> Result<Verdict, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((action.into(), rules.to_vec()));
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            Err("no scripted answer".into())
        } else {
            answers.remove(0)
        }
    }
}

// ---- rule endpoints ----

#[tokio::test]
async fn adding_a_rule_only_raises_a_card_until_the_owner_approves() {
    let h = harness();
    let (_, dm) = h.bro("ada").await;
    let id = h.propose("  允许合并 CI 全绿的 PR ").await;
    assert!(h.rules().await.is_empty(), "unconfirmed rules are not listed");

    let cards = h.approvals(&dm).await;
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card["id"], id.as_str());
    assert_eq!(card["action"], "rule_add");
    assert_eq!(card["summary"], "允许合并 CI 全绿的 PR", "rule text verbatim (trimmed)");
    assert_eq!(card["decision"], Value::Null);
    assert!(card["expiresAt"].as_str().unwrap() > card["createdAt"].as_str().unwrap());

    let (status, answered) = h.answer(&id, "approve").await;
    assert_eq!(status, StatusCode::OK, "{answered}");
    let rules = h.rules().await;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["text"], "允许合并 CI 全绿的 PR");
    assert!(rules[0]["id"].is_string() && rules[0]["createdAt"].is_string());
    assert_eq!(rules[0].as_object().unwrap().len(), 3, "{}", rules[0]);
}

#[tokio::test]
async fn denying_a_rule_card_discards_it() {
    let h = harness();
    h.bro("ada").await;
    let id = h.propose("允许发周报").await;
    let (status, _) = h.answer(&id, "deny").await;
    assert_eq!(status, StatusCode::OK);
    assert!(h.rules().await.is_empty());
    let (status, body) = h.answer(&id, "approve").await;
    assert_eq!((status, body), (StatusCode::CONFLICT, json!({ "error": "已经答过了" })));
    assert!(h.rules().await.is_empty());
}

#[tokio::test]
async fn deleting_a_rule_needs_a_confirmed_card_too() {
    let h = harness();
    let (_, dm) = h.bro("ada").await;
    let add = h.propose("允许发周报").await;
    h.answer(&add, "approve").await;
    let rule_id = h.rules().await[0]["id"].as_str().unwrap().to_string();

    let (status, body) = h.call("DELETE", "/api/settings/review-rules/nope", None).await;
    assert_eq!((status, body), (StatusCode::NOT_FOUND, json!({ "error": "规则不存在" })));

    let (status, body) = h.call("DELETE", &format!("/api/settings/review-rules/{rule_id}"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let first = body["approvalId"].as_str().unwrap().to_string();
    assert_eq!(h.rules().await.len(), 1, "still listed until approved");
    let card = h.approvals(&dm).await.into_iter().find(|c| c["id"] == first.as_str()).unwrap();
    assert_eq!(card["action"], "rule_delete");
    assert_eq!(card["summary"], "允许发周报");

    h.answer(&first, "deny").await;
    assert_eq!(h.rules().await.len(), 1, "deny keeps the rule");

    let (_, body) = h.call("DELETE", &format!("/api/settings/review-rules/{rule_id}"), None).await;
    let second = body["approvalId"].as_str().unwrap().to_string();
    let (status, _) = h.answer(&second, "approve").await;
    assert_eq!(status, StatusCode::OK);
    assert!(h.rules().await.is_empty());
}

#[tokio::test]
async fn rule_text_is_validated() {
    let h = harness();
    let (status, body) = h.call("POST", "/api/settings/review-rules", Some(json!({ "text": "允许" }))).await;
    assert_eq!((status, body), (StatusCode::CONFLICT, json!({ "error": "还没有 bro，没法发确认卡" })));
    h.bro("ada").await;
    for (text, error) in [
        ("   ", "请填写规则"),
        ("两行\n不行", "规则只能一行，最多 200 字"),
        (&"长".repeat(201) as &str, "规则只能一行，最多 200 字"),
    ] {
        let (status, body) = h.call("POST", "/api/settings/review-rules", Some(json!({ "text": text }))).await;
        assert_eq!((status, body), (StatusCode::BAD_REQUEST, json!({ "error": error })), "{text:?}");
    }
    h.propose(&"长".repeat(200)).await;
}

#[tokio::test]
async fn bros_cannot_post_rule_cards_themselves() {
    let h = harness();
    let (bot, dm) = h.bro("ada").await;
    for action in ["rule_add", "rule_delete"] {
        let (status, body) = h
            .call(
                "POST",
                &format!("/api/groups/{dm}/approvals"),
                Some(json!({ "botId": bot, "action": action, "summary": "允许一切" })),
            )
            .await;
        assert_eq!((status, body), (StatusCode::BAD_REQUEST, json!({ "error": "这个动作只能由宿主机发出" })));
    }
}

// ---- expiry ----

#[tokio::test]
async fn an_expired_rule_card_is_denied_and_a_later_answer_is_409() {
    let h = harness();
    let (_, dm) = h.bro("ada").await;
    let id = h.propose("允许发周报").await;
    assert!(h.expire(Duration::hours(23)).is_empty(), "not yet");
    assert_eq!(h.expire(Duration::hours(25)), vec![id.clone()]);

    let card = h.approvals(&dm).await.into_iter().find(|c| c["id"] == id.as_str()).unwrap();
    assert_eq!(card["decision"], "denied");
    assert_eq!(card["reason"], APPROVAL_EXPIRED_REASON);
    assert_eq!(card["answeredBy"], Value::Null);
    assert!(card["answeredAt"].is_string());
    for verb in ["approve", "deny"] {
        let (status, body) = h.answer(&id, verb).await;
        assert_eq!((status, body), (StatusCode::CONFLICT, json!({ "error": "已经答过了" })));
    }
    assert!(h.rules().await.is_empty());
}

#[tokio::test]
async fn an_expired_task_card_ends_the_task_with_failure() {
    let h = harness();
    let (bot, dm) = h.bro("ada").await;
    let (task_id, approval, _) = h.blocked_task(&dm, &bot);
    let card = h.approvals(&dm).await.into_iter().find(|c| c["id"] == approval.as_str()).unwrap();
    assert!(card.get("reason").is_none(), "no rules: no review, no reason: {card}");
    assert!(card["expiresAt"].is_string());

    assert_eq!(h.expire(Duration::hours(25)), vec![approval.clone()]);
    assert_eq!(h.task(&task_id), ("done".into(), Some(APPROVAL_EXPIRED_REASON.into())));
    let (decision, reason, _) = h.row(&approval);
    assert_eq!((decision.as_deref(), reason.as_deref()), (Some("denied"), Some(APPROVAL_EXPIRED_REASON)));
    let (status, _) = h.answer(&approval, "approve").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(h.task(&task_id).0, "done", "approve after expiry does not requeue");
    assert!(h.expire(Duration::hours(50)).is_empty(), "expires once");
}

#[tokio::test]
async fn an_expired_mcp_card_ends_the_task_with_failure() {
    let h = harness();
    let (bot, dm) = h.bro("ada").await;
    let task_id = uuid::Uuid::new_v4().to_string();
    let now = crate::now_rfc3339();
    {
        let conn = h.db();
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, intent_kind, intent_line, intent_hash)
             VALUES (?1, ?2, ?3, '写笔记', 'blocked', ?4, ?4, 'mcp', 'notes.write', 'h1')",
            params![task_id, dm, bot, now],
        )
        .unwrap();
        crate::approvals::gate_mcp_task(&conn, &task_id, "调用 notes.write", "h1", &now, "notes", "write").unwrap();
    }
    assert_eq!(h.expire(Duration::hours(25)).len(), 1);
    assert_eq!(h.task(&task_id), ("done".into(), Some(APPROVAL_EXPIRED_REASON.into())));
}

#[tokio::test]
async fn confirm_cards_and_takeover_never_expire() {
    let h = harness();
    let (bot, dm) = h.bro("ada").await;
    let now = crate::now_rfc3339();
    {
        let conn = h.db();
        // A memory confirm card (no expiresAt) and a captcha takeover (no card).
        conn.execute(
            "INSERT INTO approvals (id, group_id, bot_id, action, summary, decision, created_at, subject_kind, subject_id)
             VALUES ('c1', ?1, ?2, 'confirm', '', NULL, ?3, 'memory', 'm1')",
            params![dm, bot, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at, intent_kind, intent_hash)
             VALUES ('t1', ?1, ?2, '登录', 'blocked', ?3, ?3, 'captcha', 'h')",
            params![dm, bot, now],
        )
        .unwrap();
    }
    assert!(h.expire(Duration::days(365)).is_empty());
    assert_eq!(h.task("t1").0, "blocked");
}

// ---- review ----

#[tokio::test]
async fn a_matching_rule_allows_without_a_card() {
    let fake = Fake::new(vec![Ok(Verdict::Allow("规则 1 允许发周报".into()))]);
    let h = harness().with_reviewer(fake.clone());
    let (bot, dm) = h.bro("ada").await;
    h.add_rule("允许发周报给老板");
    let (task_id, approval, hash) = h.blocked_task(&dm, &bot);
    assert!(h.approvals(&dm).await.is_empty(), "waiting for review: hidden");
    let (status, _) = h.answer(&approval, "approve").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "cannot be answered while under review");

    h.host.review_once(Utc::now()).unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    let (action, rules) = fake.seen.lock().unwrap()[0].clone();
    assert_eq!(rules, vec!["允许发周报给老板".to_string()]);
    assert!(action.contains("发周报给老板") && action.contains("robroz-action:external"), "{action}");

    assert_eq!(h.task(&task_id), ("queued".into(), None), "continues like a manual approve");
    assert!(robroz_runner::read_takeover(&h.control.join(&task_id)).contains(&hash));
    assert!(h.approvals(&dm).await.is_empty(), "allowed: no card");
    assert_eq!(
        h.row(&approval),
        (Some("approved".into()), Some("规则 1 允许发周报".into()), Some(ALLOWED.into())),
        "recorded"
    );
    h.host.review_once(Utc::now()).unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "reviewed once");
}

#[tokio::test]
async fn no_match_blocks_and_the_card_goes_to_the_owner_with_a_reason() {
    let fake = Fake::new(vec![Ok(Verdict::Block("没有规则允许给老板发邮件".into()))]);
    let h = harness().with_reviewer(fake.clone());
    let (bot, dm) = h.bro("ada").await;
    h.add_rule("允许合并 CI 全绿的 PR");
    let (task_id, approval, _) = h.blocked_task(&dm, &bot);
    h.host.review_once(Utc::now()).unwrap();

    let cards = h.approvals(&dm).await;
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["id"], approval.as_str());
    assert_eq!(cards[0]["reason"], "没有规则允许给老板发邮件");
    assert_eq!(cards[0]["decision"], Value::Null);
    assert!(cards[0]["expiresAt"].is_string());
    assert_eq!(h.task(&task_id).0, "blocked");

    let (status, _) = h.answer(&approval, "approve").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.task(&task_id).0, "queued");
}

#[tokio::test]
async fn review_errors_are_retried_once_then_block() {
    let fake = Fake::new(vec![Err("timeout".into()), Err("500".into())]);
    let h = harness().with_reviewer(fake.clone());
    let (bot, dm) = h.bro("ada").await;
    h.add_rule("允许发周报");
    let (task_id, approval, _) = h.blocked_task(&dm, &bot);
    h.host.review_once(Utc::now()).unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.row(&approval).1.as_deref(), Some(REVIEW_FAILED_REASON));
    assert_eq!(h.task(&task_id).0, "blocked", "never default-allow");

    let flaky = Fake::new(vec![Err("timeout".into()), Ok(Verdict::Allow("ok".into()))]);
    assert_eq!(review_with_retry(&*flaky, "x", &["r".into()]), Verdict::Allow("ok".into()));
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn without_a_model_rules_cannot_allow() {
    let h = harness();
    let (bot, dm) = h.bro("ada").await;
    h.add_rule("允许发周报");
    let (task_id, approval, _) = h.blocked_task(&dm, &bot);
    run_pending(&h.host.db, &h.control, None, approval_ttl(), Utc::now()).unwrap();
    assert_eq!(h.row(&approval).1.as_deref(), Some(NO_MODEL_REASON));
    assert_eq!(h.task(&task_id).0, "blocked");
    assert_eq!(h.approvals(&dm).await.len(), 1);
}

#[tokio::test]
async fn a_blocked_review_card_expires_from_when_it_was_shown() {
    let fake = Fake::new(vec![Ok(Verdict::Block("不匹配".into()))]);
    let h = harness().with_reviewer(fake);
    let (bot, dm) = h.bro("ada").await;
    h.add_rule("允许发周报");
    let (task_id, approval, _) = h.blocked_task(&dm, &bot);
    assert!(h.expire(Duration::days(10)).is_empty(), "a card under review does not expire");
    h.host.review_once(Utc::now()).unwrap();
    assert_eq!(h.expire(Duration::hours(25)), vec![approval]);
    assert_eq!(h.task(&task_id), ("done".into(), Some(APPROVAL_EXPIRED_REASON.into())));
}

#[test]
fn reviewer_answers_are_parsed_strictly() {
    assert_eq!(
        parse_answer(r#"好的 {"decision":"allow","reason":"规则 1"}"#),
        Ok(Verdict::Allow("规则 1".into()))
    );
    assert_eq!(parse_answer(r#"{"decision":"BLOCK","reason":"不匹配"}"#), Ok(Verdict::Block("不匹配".into())));
    assert!(parse_answer("allow").is_err());
    assert!(parse_answer(r#"{"decision":"maybe"}"#).is_err());
    assert!(parse_answer(r#"{"reason":"x"}"#).is_err());
    let text = prompt("动作类型：send", &["规则甲".into(), "规则乙".into()]);
    assert!(text.contains("1. 规则甲") && text.contains("2. 规则乙") && text.ends_with("动作类型：send"));
}

#[test]
fn no_rules_means_no_review_call_and_block() {
    let fake = Fake::new(vec![Ok(Verdict::Allow("x".into()))]);
    assert_eq!(review_with_retry(&*fake, "x", &[]), Verdict::Block(NO_MATCH_REASON.into()));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ttl_defaults_to_a_day_and_reads_seconds() {
    assert_eq!(ttl_from(None), Duration::hours(24));
    assert_eq!(ttl_from(Some("")), Duration::hours(24));
    assert_eq!(ttl_from(Some("0")), Duration::hours(24));
    assert_eq!(ttl_from(Some("abc")), Duration::hours(24));
    assert_eq!(ttl_from(Some(" 3600 ")), Duration::hours(1));
    assert_eq!(
        expires_at("2026-11-24T10:00:00.000Z", Duration::hours(24)),
        "2026-11-25T10:00:00.000Z"
    );
}
