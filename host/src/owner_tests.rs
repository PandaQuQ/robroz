//! v0.2 accounts: bro owners, per-(user, bro) DMs, owner-only approvals and
//! the DM reminder line. A second user is inserted straight into the
//! database and put on the request, since there is no user API yet.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use robroz_contract::{BOOTSTRAP_ADMIN_ID, BOOTSTRAP_ADMIN_NAME};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::users::{insert_for_test, CurrentUser};
use crate::Host;

const MEMBER: &str = "u-mei";
const OTHER: &str = "u-otto";

struct H {
    db: PathBuf,
    _host: Host,
    app: Router,
}

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("robroz-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn harness_on(db: PathBuf) -> H {
    let dir = temp("owner");
    let host = Host::open(&db, dir.join("groups"), "robroz-runner:local")
        .unwrap()
        .with_control_root(dir.join("control"));
    let app = host.router();
    let h = H {
        db,
        _host: host,
        app,
    };
    let conn = h.conn();
    insert_for_test(&conn, MEMBER, "Mei", "member");
    insert_for_test(&conn, OTHER, "Otto", "member");
    h
}

fn harness() -> H {
    harness_on(temp("owner-db").join("robroz.db"))
}

impl H {
    fn conn(&self) -> Connection {
        Connection::open(&self.db).unwrap()
    }

    /// `as_user: None` is a plain v0.1 request (the bootstrap admin).
    async fn call(
        &self,
        as_user: Option<&str>,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let mut req = req.body(body).unwrap();
        if let Some(u) = as_user {
            req.extensions_mut().insert(CurrentUser(u.to_string()));
        }
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value)
    }

    /// A bro owned by `user`, `(botId, dmGroupId)`.
    async fn bro(&self, user: Option<&str>, name: &str) -> (String, String) {
        let (status, bro) = self
            .call(user, "POST", "/api/bros", Some(json!({ "name": name })))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{bro}");
        (
            bro["id"].as_str().unwrap().into(),
            bro["dmGroupId"].as_str().unwrap().into(),
        )
    }

    async fn group_with(&self, user: Option<&str>, bot: &str) -> String {
        let (_, g) = self
            .call(
                user,
                "POST",
                "/api/groups",
                Some(json!({ "name": "作战组" })),
            )
            .await;
        let id = g["id"].as_str().unwrap().to_string();
        let (status, body) = self
            .call(
                user,
                "POST",
                &format!("/api/groups/{id}/members"),
                Some(json!({ "botId": bot, "name": "x" })),
            )
            .await;
        assert!(status.is_success(), "{body}");
        id
    }

    async fn card(&self, group: &str, bot: &str) -> Value {
        let (status, card) = self
            .call(
                None,
                "POST",
                &format!("/api/groups/{group}/approvals"),
                Some(json!({ "botId": bot, "action": "send", "summary": "发周报" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{card}");
        card
    }

    async fn answer(&self, user: Option<&str>, id: &str, verb: &str) -> (StatusCode, Value) {
        self.call(user, "POST", &format!("/api/approvals/{id}/{verb}"), None)
            .await
    }

    async fn events(&self, group: &str, typ: &str) -> Vec<Value> {
        let (status, body) = self
            .call(None, "GET", &format!("/api/groups/{group}/events"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == typ)
            .cloned()
            .collect()
    }
}

// ---- identity ----

#[tokio::test]
async fn a_plain_request_is_the_bootstrap_admin() {
    let h = harness();
    let (status, me) = h.call(None, "GET", "/api/me", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        me,
        json!({ "id": BOOTSTRAP_ADMIN_ID, "name": BOOTSTRAP_ADMIN_NAME, "role": "admin" })
    );
    let (_, me) = h.call(Some(MEMBER), "GET", "/api/me", None).await;
    assert_eq!(me["role"], "member");
}

#[tokio::test]
async fn the_env_token_is_the_bootstrap_admin_and_a_bad_token_is_still_401() {
    let dir = temp("owner-token");
    let token = crate::auth::ApiToken::parse("0123456789abcdef-owner")
        .unwrap()
        .unwrap();
    let app = Host::open(
        &dir.join("robroz.db"),
        dir.join("groups"),
        "robroz-runner:local",
    )
    .unwrap()
    .with_token(Some(token))
    .router();
    let send = |auth: Option<&'static str>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().uri("/api/me");
            if let Some(a) = auth {
                b = b.header("authorization", a);
            }
            let resp = app.oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }
    };
    let (status, me) = send(Some("Bearer 0123456789abcdef-owner")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["id"], BOOTSTRAP_ADMIN_ID);
    let (status, _) = send(Some("Bearer wrong-wrong-wrong-wrong")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---- bro owners ----

#[tokio::test]
async fn any_user_creates_a_bro_and_owns_it() {
    let h = harness();
    let (status, bro) = h
        .call(
            Some(MEMBER),
            "POST",
            "/api/bros",
            Some(json!({ "name": "mei 的 bro" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{bro}");
    assert_eq!(bro["ownerUserId"], MEMBER);
    let (admin_bot, _) = h.bro(None, "admin 的 bro").await;

    let (_, mine) = h.call(Some(MEMBER), "GET", "/api/bros", None).await;
    let ids: Vec<_> = mine["bros"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].clone())
        .collect();
    assert_eq!(ids, vec![bro["id"].clone()], "only the caller's bros");
    let (_, admins) = h.call(None, "GET", "/api/bros", None).await;
    assert_eq!(admins["bros"].as_array().unwrap().len(), 1);
    assert_eq!(admins["bros"][0]["id"], admin_bot.as_str());
    assert_eq!(admins["bros"][0]["ownerUserId"], BOOTSTRAP_ADMIN_ID);
}

#[tokio::test]
async fn only_the_owner_renames_a_bro_and_admin_is_no_exception() {
    let h = harness();
    let (bot, dm) = h.bro(Some(MEMBER), "mei").await;
    for who in [None, Some(OTHER)] {
        let (status, body) = h
            .call(
                who,
                "PATCH",
                &format!("/api/bros/{bot}"),
                Some(json!({ "name": "抢走" })),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who:?}: {body}");
        assert_eq!(
            body,
            json!({
                "error": robroz_contract::NOT_BRO_OWNER,
                "code": "not_bro_owner",
                "ownerUserId": MEMBER,
                "ownerName": "Mei"
            })
        );
    }
    let (status, body) = h
        .call(
            Some(MEMBER),
            "PATCH",
            &format!("/api/bros/{bot}"),
            Some(json!({ "name": "mei 2" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "mei 2");
    assert_eq!(body["dmGroupId"], dm.as_str());
    let (status, _) = h
        .call(
            Some(MEMBER),
            "PATCH",
            "/api/bros/nope",
            Some(json!({ "name": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown stays 404");
}

#[tokio::test]
async fn only_the_owner_adds_an_existing_bro_to_a_group() {
    let h = harness();
    let (bot, _) = h.bro(Some(MEMBER), "mei").await;
    let (_, g) = h
        .call(None, "POST", "/api/groups", Some(json!({ "name": "群" })))
        .await;
    let g = g["id"].as_str().unwrap();
    let uri = format!("/api/groups/{g}/members");
    let (status, body) = h
        .call(
            None,
            "POST",
            &uri,
            Some(json!({ "botId": bot, "name": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body,
        json!({
            "error": robroz_contract::NOT_BRO_OWNER,
            "code": "not_bro_owner",
            "ownerUserId": MEMBER,
            "ownerName": "Mei"
        })
    );
    let (status, _) = h
        .call(
            Some(MEMBER),
            "POST",
            &uri,
            Some(json!({ "botId": bot, "name": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // A new id added by botId becomes a bro of the caller.
    let (status, _) = h
        .call(
            Some(OTHER),
            "POST",
            &uri,
            Some(json!({ "botId": "bot-new", "name": "New" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let owner: String = h
        .conn()
        .query_row(
            "SELECT owner_user_id FROM bots WHERE id = 'bot-new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owner, OTHER);
}

// ---- DMs ----

#[tokio::test]
async fn a_dm_is_unique_per_user_and_bro_and_only_the_owner_opens_it() {
    let h = harness();
    let (bot, dm) = h.bro(Some(MEMBER), "mei").await;
    let (user, dm_bot): (String, String) = h
        .conn()
        .query_row(
            "SELECT dm_user_id, dm_bot_id FROM groups WHERE id = ?1",
            [&dm],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((user.as_str(), dm_bot.as_str()), (MEMBER, bot.as_str()));

    let uri = format!("/api/bros/{bot}/dm");
    let (status, body) = h.call(Some(MEMBER), "POST", &uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "existing DM: {body}");
    assert_eq!(body["dmGroupId"], dm.as_str());
    for who in [None, Some(OTHER)] {
        let (status, body) = h.call(who, "POST", &uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(
            body,
            json!({
                "error": robroz_contract::NOT_BRO_OWNER,
                "code": "not_bro_owner",
                "ownerUserId": MEMBER,
                "ownerName": "Mei"
            })
        );
    }

    let insert = |id: &str, user: &str| {
        h.conn().execute(
            "INSERT INTO groups (id, tenant_id, name, created_at, dm_bot_id, dm_user_id)
             VALUES (?1, 'local', 'x', '2026-11-25T00:00:00.000Z', ?2, ?3)",
            params![id, bot, user],
        )
    };
    assert!(insert("dup", MEMBER).is_err(), "one DM per (user, bro)");
    assert!(
        insert("other-user", OTHER).is_ok(),
        "the index is per user, not per bro"
    );
    let (_, body) = h.call(Some(MEMBER), "POST", &uri, Some(json!({}))).await;
    assert_eq!(
        body["dmGroupId"],
        dm.as_str(),
        "the owner's DM, not another user's"
    );
}

// ---- owner-only approvals ----

#[tokio::test]
async fn only_the_bros_owner_answers_its_group_card() {
    let h = harness();
    let (bot, dm) = h.bro(Some(MEMBER), "mei").await;
    let group = h.group_with(Some(MEMBER), &bot).await;
    let card = h.card(&group, &bot).await;
    assert_eq!(card["ownerUserId"], MEMBER);
    assert_eq!(card["ownerName"], "Mei");
    let id = card["id"].as_str().unwrap();

    for who in [None, Some(OTHER)] {
        for verb in ["approve", "deny"] {
            let (status, body) = h.answer(who, id, verb).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who:?} {verb}: {body}");
            assert_eq!(
                body,
                json!({
                    "error": robroz_contract::NOT_BRO_OWNER,
                    "code": "approval_forbidden",
                    "reason": "not_owner",
                    "ownerUserId": MEMBER,
                    "ownerName": "Mei"
                })
            );
        }
    }
    assert_eq!(
        h.events(&group, "approval").await[0]["decision"],
        Value::Null,
        "still open"
    );

    let reminder = &h.events(&dm, "approval_reminder").await[0];
    assert_eq!(reminder["ownerUserId"], MEMBER);
    assert_eq!(reminder["state"], "waiting");

    let (status, body) = h.answer(Some(MEMBER), id, "approve").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["decision"], "approved");
    let (status, body) = h.answer(Some(MEMBER), id, "deny").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["reason"], "decided");
    assert_eq!(body["error"], robroz_contract::host_text::ALREADY_ANSWERED);
    let (_, body) = h.answer(Some(OTHER), id, "deny").await;
    assert_eq!(body["reason"], "not_owner", "not the owner comes first");
    assert_eq!(
        h.events(&dm, "approval_reminder").await[0]["state"],
        "approved"
    );
}

#[tokio::test]
async fn a_denied_and_an_expired_card_update_the_reminder() {
    let h = harness();
    let (bot, dm) = h.bro(Some(MEMBER), "mei").await;
    let group = h.group_with(Some(MEMBER), &bot).await;
    let denied = h.card(&group, &bot).await;
    let expiring = h.card(&group, &bot).await;
    let (status, _) = h
        .answer(Some(MEMBER), denied["id"].as_str().unwrap(), "deny")
        .await;
    assert_eq!(status, StatusCode::OK);
    let expiring_id = expiring["id"].as_str().unwrap();
    {
        let mut conn = h.conn();
        let expired = crate::review::expire_due(
            &mut conn,
            &std::env::temp_dir().join("robroz-owner-control"),
            chrono::Utc::now() + chrono::Duration::hours(25),
        )
        .unwrap();
        assert_eq!(expired, vec![expiring_id.to_string()]);
    }
    let (status, body) = h.answer(Some(MEMBER), expiring_id, "approve").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["reason"], "expired");
    assert_eq!(body["error"], robroz_contract::APPROVAL_EXPIRED_REASON);

    let reminders = h.events(&dm, "approval_reminder").await;
    let state_of =
        |id: &Value| reminders.iter().find(|r| r["approvalId"] == *id).unwrap()["state"].clone();
    assert_eq!(state_of(&denied["id"]), "denied");
    assert_eq!(state_of(&expiring["id"]), "expired");
    assert!(reminders.iter().all(|r| r["answeredAt"].is_string()));
    assert!(
        reminders.iter().all(|r| r.get("actions").is_none()),
        "no buttons"
    );
}

#[tokio::test]
async fn a_dm_task_card_stays_in_the_dm_with_no_reminder() {
    let h = harness();
    let (bot, dm) = h.bro(Some(MEMBER), "mei").await;
    let card = h.card(&dm, &bot).await;
    assert_eq!(h.events(&dm, "approval").await.len(), 1);
    assert!(h.events(&dm, "approval_reminder").await.is_empty());
    let (status, _) = h
        .answer(None, card["id"].as_str().unwrap(), "approve")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "owner only in the DM too");
    let (status, _) = h
        .answer(Some(MEMBER), card["id"].as_str().unwrap(), "approve")
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_settings_card_is_answered_by_an_admin_only() {
    let h = harness();
    let (status, created) = h
        .call(
            None,
            "POST",
            "/api/settings/review-rules",
            Some(json!({ "text": "允许发周报" })),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{created}");
    let id = created["approvalId"]
        .as_str()
        .or_else(|| created["id"].as_str())
        .unwrap()
        .to_string();
    let (status, body) = h.answer(Some(MEMBER), &id, "approve").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["reason"], "not_owner");
    assert!(body.get("ownerUserId").is_none(), "{body}");
    let (status, body) = h.answer(None, &id, "approve").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// ---- migration ----

#[test]
fn a_v0_1_database_migrates_and_its_data_stays_visible() {
    let dir = temp("owner-v01");
    let db = dir.join("robroz.db");
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(crate::tests::PREVIOUS_MAIN_SCHEMA)
            .unwrap();
        conn.execute_batch(crate::tests::PREVIOUS_MAIN_ROWS)
            .unwrap();
        // v0.1.x: DMs by `dm_bot_id`, unique per bro.
        conn.execute_batch(
            "ALTER TABLE groups ADD COLUMN dm_bot_id TEXT;
             CREATE UNIQUE INDEX groups_dm_bot ON groups(dm_bot_id) WHERE dm_bot_id IS NOT NULL;
             INSERT INTO groups (id, tenant_id, name, created_at, dm_bot_id)
               VALUES ('dm-ada', 'local', 'Ada', '2026-10-01T00:00:00.500Z', 'bot-ada');
             INSERT INTO memberships VALUES ('dm-ada', 'bot-ada');",
        )
        .unwrap();
    }
    let h = harness_on(db);
    let conn = h.conn();
    let (owner, dm_user): (String, String) = conn
        .query_row(
            "SELECT b.owner_user_id, g.dm_user_id FROM bots b JOIN groups g ON g.dm_bot_id = b.id
             WHERE b.id = 'bot-ada'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (owner.as_str(), dm_user.as_str()),
        (BOOTSTRAP_ADMIN_ID, BOOTSTRAP_ADMIN_ID)
    );
    let indexes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'groups'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        indexes.contains(&"groups_dm_user_bot".to_string()),
        "{indexes:?}"
    );
    assert!(
        !indexes.contains(&"groups_dm_bot".to_string()),
        "{indexes:?}"
    );
    let admins: i64 = conn
        .query_row("SELECT COUNT(*) FROM users WHERE role = 'admin'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(admins, 1);
    drop(conn);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (_, bros) = h.call(None, "GET", "/api/bros", None).await;
        assert_eq!(
            bros["bros"],
            json!([{ "id": "bot-ada", "name": "Ada", "dmGroupId": "dm-ada", "ownerUserId": BOOTSTRAP_ADMIN_ID }])
        );
        let (status, _) = h.call(None, "POST", "/api/bros/bot-ada/dm", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "the old DM is reused");
        let messages = h.events("g1", "message").await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["authorUserId"], BOOTSTRAP_ADMIN_ID, "v0.1 messages are the admin's");
        assert_eq!(messages[0]["authorName"], BOOTSTRAP_ADMIN_NAME);
        let cards = h.events("g1", "approval").await;
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0]["ownerUserId"], BOOTSTRAP_ADMIN_ID);
        assert_eq!(h.events("dm-ada", "approval_reminder").await.len(), 1, "old group card gets a reminder");
        let (status, _) = h.answer(None, "a1", "approve").await;
        assert_eq!(status, StatusCode::OK, "the v0.1 user still answers old cards");
    });
    // Opening again is a no-op.
    let again = Host::open(&h.db, temp("owner-v01b"), "robroz-runner:local");
    assert!(again.is_ok());
}

// ---- message authors ----

#[tokio::test]
async fn a_person_message_carries_its_author_in_the_list_and_on_sse() {
    let h = harness();
    let (_, g) = h
        .call(None, "POST", "/api/groups", Some(json!({ "name": "群" })))
        .await;
    let g = g["id"].as_str().unwrap().to_string();

    // Open the stream first, so the messages arrive live, not in the snapshot.
    let mut req = Request::builder()
        .uri(format!("/api/groups/{g}/stream"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(CurrentUser(MEMBER.into()));
    let mut stream = h.app.clone().oneshot(req).await.unwrap().into_body();

    let (status, posted) = h
        .call(
            Some(MEMBER),
            "POST",
            &format!("/api/groups/{g}/messages"),
            Some(json!({ "kind": "text", "body": "mei 说" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{posted}");
    assert_eq!(posted["authorUserId"], MEMBER);
    assert_eq!(posted["authorName"], "Mei");
    let (status, said) = h
        .call(
            None,
            "POST",
            &format!("/api/groups/{g}/say"),
            Some(json!({ "body": "admin 说" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{said}");
    assert_eq!(said["message"]["authorUserId"], BOOTSTRAP_ADMIN_ID);
    assert_eq!(said["message"]["authorName"], BOOTSTRAP_ADMIN_NAME);

    let listed = h.events(&g, "message").await;
    let authors: Vec<_> = listed
        .iter()
        .map(|m| {
            (
                m["body"].clone(),
                m["authorUserId"].clone(),
                m["authorName"].clone(),
            )
        })
        .collect();
    assert_eq!(
        authors,
        vec![
            (json!("mei 说"), json!(MEMBER), json!("Mei")),
            (
                json!("admin 说"),
                json!(BOOTSTRAP_ADMIN_ID),
                json!(BOOTSTRAP_ADMIN_NAME)
            ),
        ]
    );

    let pushed: Vec<_> = crate::tests::sse_take(&mut stream, 2, std::time::Duration::from_secs(5))
        .await
        .into_iter()
        .filter(|(name, _, _)| name == "message")
        .map(|(_, _, data)| (data["authorUserId"].clone(), data["authorName"].clone()))
        .collect();
    assert_eq!(
        pushed,
        vec![
            (json!(MEMBER), json!("Mei")),
            (json!(BOOTSTRAP_ADMIN_ID), json!(BOOTSTRAP_ADMIN_NAME)),
        ]
    );
}
