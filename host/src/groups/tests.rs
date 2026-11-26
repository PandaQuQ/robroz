//! Group members, rename, delete and search.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use robroz_mcp::{ConnectConfig, ListedTool};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::mcp_tools::McpRuntime;
use crate::sandbox::FakeSandbox;
use crate::users::{insert_for_test, CurrentUser};
use crate::Host;

const MEI: &str = "u-mei";
const OTTO: &str = "u-otto";

/// Records `evict_group` calls.
#[derive(Default)]
struct RecordingMcp(Mutex<Vec<String>>);

impl McpRuntime for RecordingMcp {
    fn list_tools(&self, _: &str, _: &str, _: &ConnectConfig) -> Result<Vec<ListedTool>, String> {
        Ok(Vec::new())
    }
    fn call_tool(
        &self,
        _: &str,
        _: &str,
        _: &ConnectConfig,
        _: &str,
        _: &Value,
    ) -> Result<String, String> {
        Err("unused".into())
    }
    fn evict_group(&self, group_id: &str) {
        self.0.lock().unwrap().push(group_id.to_string());
    }
}

struct H {
    db: PathBuf,
    volumes: PathBuf,
    control: PathBuf,
    host: Host,
    app: Router,
    mcp: Arc<RecordingMcp>,
}

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("robroz-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn harness_on(db: PathBuf) -> H {
    let dir = temp("groups");
    let mcp = Arc::new(RecordingMcp::default());
    let host = Host::open(&db, dir.join("volumes"), "robroz-runner:local")
        .unwrap()
        .with_control_root(dir.join("control"))
        .with_mcp(mcp.clone());
    let app = host.router();
    let h = H {
        db,
        volumes: dir.join("volumes"),
        control: dir.join("control"),
        host,
        app,
        mcp,
    };
    let conn = h.conn();
    if crate::users::load(&conn, MEI).unwrap().is_none() {
        insert_for_test(&conn, MEI, "Mei", "member");
        insert_for_test(&conn, OTTO, "Otto", "member");
    }
    h
}

fn harness() -> H {
    harness_on(temp("groups-db").join("robroz.db"))
}

impl H {
    fn conn(&self) -> Connection {
        Connection::open(&self.db).unwrap()
    }

    /// `None` is the bootstrap admin.
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

    /// A bro owned by `user`: `(botId, dmGroupId)`.
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

    async fn group(&self, user: Option<&str>, name: &str) -> String {
        let (status, g) = self
            .call(user, "POST", "/api/groups", Some(json!({ "name": name })))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{g}");
        g["id"].as_str().unwrap().to_string()
    }

    async fn add(&self, user: Option<&str>, group: &str, bot: &str) {
        let (status, body) = self
            .call(
                user,
                "POST",
                &format!("/api/groups/{group}/members"),
                Some(json!({ "botId": bot, "name": "x" })),
            )
            .await;
        assert!(status.is_success(), "{body}");
    }

    async fn say(&self, user: Option<&str>, group: &str, text: &str) -> String {
        let (status, m) = self
            .call(
                user,
                "POST",
                &format!("/api/groups/{group}/messages"),
                Some(json!({ "kind": "text", "body": text })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{m}");
        m["id"].as_str().unwrap().to_string()
    }

    async fn search(&self, user: Option<&str>, group: &str, query: &str) -> (StatusCode, Value) {
        self.call(
            user,
            "GET",
            &format!("/api/groups/{group}/search?{query}"),
            None,
        )
        .await
    }

    fn count(&self, sql: &str, id: &str) -> i64 {
        self.conn().query_row(sql, [id], |r| r.get(0)).unwrap()
    }
}

fn assert_code(status: StatusCode, body: &Value, want: StatusCode, code: &str) {
    assert_eq!(status, want, "{body}");
    assert_eq!(body["code"], code, "{body}");
}

// ---- created_by ----

#[tokio::test]
async fn new_groups_record_their_creator() {
    let h = harness();
    let g = h.group(Some(MEI), "周报").await;
    let (_, list) = h.call(None, "GET", "/api/groups", None).await;
    let row = list["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == g)
        .unwrap()
        .clone();
    assert_eq!(row["createdBy"], MEI);
    assert_eq!(row["dm"], false);
}

#[tokio::test]
async fn groups_mark_whether_they_are_a_dm() {
    let h = harness();
    let (_ada, dm) = h.bro(Some(MEI), "Ada").await;
    let (status, created) = h
        .call(
            Some(MEI),
            "POST",
            "/api/groups",
            Some(json!({ "name": "周报" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["dm"], false, "{created}");
    assert_eq!(created["createdBy"], MEI);
    let g = created["id"].as_str().unwrap().to_string();

    let (status, renamed) = h
        .call(
            Some(MEI),
            "PATCH",
            &format!("/api/groups/{g}"),
            Some(json!({ "name": "周报组" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["dm"], false, "{renamed}");

    let (_, list) = h.call(None, "GET", "/api/groups", None).await;
    let find = |id: &str| {
        list["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .cloned()
            .unwrap()
    };
    let ordinary = find(&g);
    assert_eq!(ordinary["dm"], false, "{ordinary}");
    assert_eq!(ordinary["createdBy"], MEI);
    let dm_row = find(&dm);
    assert_eq!(dm_row["dm"], true, "{dm_row}");
    assert!(dm_row.get("createdBy").is_none(), "{dm_row}");
}

// ---- members ----

#[tokio::test]
async fn members_lists_bros_and_their_owners_for_members_only() {
    let h = harness();
    let (ada, _) = h.bro(Some(MEI), "Ada").await;
    let (bob, _) = h.bro(Some(OTTO), "Bob").await;
    let (cat, _) = h.bro(Some(MEI), "Cat").await;
    let g = h.group(Some(MEI), "作战组").await;
    h.add(Some(MEI), &g, &ada).await;
    h.add(Some(OTTO), &g, &bob).await;
    h.add(Some(MEI), &g, &cat).await;

    let uri = format!("/api/groups/{g}/members");
    let (status, body) = h.call(Some(OTTO), "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["groupId"], g);
    assert_eq!(body["createdBy"], MEI);
    let bros = body["bros"].as_array().unwrap();
    assert_eq!(bros.len(), 3);
    assert_eq!(bros[0]["botId"], ada);
    assert_eq!(bros[0]["ownerUserId"], MEI);
    assert_eq!(bros[0]["ownerName"], "Mei");
    assert_eq!(bros[1]["ownerName"], "Otto");
    let users = body["users"].as_array().unwrap();
    assert_eq!(users.len(), 2, "{body}");
    assert_eq!(users[0]["userId"], MEI, "creator first");
    assert_eq!(users[0]["creator"], true);
    assert_eq!(users[0]["broIds"], json!([ada, cat]));
    assert_eq!(users[1]["userId"], OTTO);
    assert_eq!(users[1]["creator"], false);
    assert_eq!(users[1]["broIds"], json!([bob]));

    // Admin sees every group.
    let (status, _) = h.call(None, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);

    // Otto's only bro leaves: Otto is no longer in the group.
    h.conn()
        .execute("DELETE FROM memberships WHERE bot_id = ?1", [&bob])
        .unwrap();
    let (status, body) = h.call(Some(OTTO), "GET", &uri, None).await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_member");

    let (status, body) = h
        .call(Some(MEI), "GET", "/api/groups/nope/members", None)
        .await;
    assert_code(status, &body, StatusCode::NOT_FOUND, "group_not_found");
}

#[tokio::test]
async fn a_dm_shows_its_bro_to_its_user_only() {
    let h = harness();
    let (ada, dm) = h.bro(Some(MEI), "Ada").await;
    let uri = format!("/api/groups/{dm}/members");
    let (status, body) = h.call(Some(MEI), "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["bros"][0]["botId"], ada);
    assert_eq!(body["users"][0]["userId"], MEI);
    assert_eq!(body["users"][0]["creator"], false);
    let (status, body) = h.call(Some(OTTO), "GET", &uri, None).await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_member");
}

// ---- rename ----

#[tokio::test]
async fn rename_is_for_the_creator_or_an_admin() {
    let h = harness();
    let (bob, bob_dm) = h.bro(Some(OTTO), "Bob").await;
    let g = h.group(Some(MEI), "旧名").await;
    h.add(Some(OTTO), &g, &bob).await;
    let uri = format!("/api/groups/{g}");

    let (status, body) = h
        .call(Some(MEI), "PATCH", &uri, Some(json!({ "name": "  新名 " })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "新名");
    assert_eq!(body["createdBy"], MEI);

    let (status, body) = h
        .call(Some(OTTO), "PATCH", &uri, Some(json!({ "name": "抢" })))
        .await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_creator");

    let (status, body) = h
        .call(None, "PATCH", &uri, Some(json!({ "name": "管理员改" })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = h
        .call(Some(MEI), "PATCH", &uri, Some(json!({ "name": "  " })))
        .await;
    assert_code(status, &body, StatusCode::BAD_REQUEST, "need_name");
    let (status, body) = h.call(Some(MEI), "PATCH", &uri, Some(json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["code"].is_string(), "{body}");

    let (status, body) = h
        .call(
            Some(OTTO),
            "PATCH",
            &format!("/api/groups/{bob_dm}"),
            Some(json!({ "name": "x" })),
        )
        .await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "group_is_dm");

    let (status, body) = h
        .call(
            None,
            "PATCH",
            "/api/groups/nope",
            Some(json!({ "name": "x" })),
        )
        .await;
    assert_code(status, &body, StatusCode::NOT_FOUND, "group_not_found");

    let (_, list) = h.call(None, "GET", "/api/groups", None).await;
    assert!(list["groups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["name"] == "管理员改"));
}

#[tokio::test]
async fn an_old_group_without_a_creator_is_admin_only() {
    let h = harness();
    let g = h.group(Some(MEI), "老群").await;
    h.conn()
        .execute("UPDATE groups SET created_by = NULL WHERE id = ?1", [&g])
        .unwrap();
    let uri = format!("/api/groups/{g}");
    let (status, body) = h
        .call(Some(MEI), "PATCH", &uri, Some(json!({ "name": "x" })))
        .await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_creator");
    let (status, body) = h.call(Some(MEI), "DELETE", &uri, None).await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_creator");
    let (status, body) = h
        .call(None, "PATCH", &uri, Some(json!({ "name": "x" })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("createdBy").is_none(), "{body}");
}

// ---- delete ----

#[tokio::test]
async fn delete_is_for_the_creator_or_an_admin_and_never_a_dm() {
    let h = harness();
    let (ada, dm) = h.bro(Some(MEI), "Ada").await;
    let g = h.group(Some(MEI), "要删").await;
    h.add(Some(MEI), &g, &ada).await;

    let (status, body) = h
        .call(Some(OTTO), "DELETE", &format!("/api/groups/{g}"), None)
        .await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_creator");

    for user in [Some(MEI), None] {
        let (status, body) = h
            .call(user, "DELETE", &format!("/api/groups/{dm}"), None)
            .await;
        assert_code(status, &body, StatusCode::FORBIDDEN, "group_is_dm");
    }
    assert!(h.mcp.0.lock().unwrap().is_empty());

    let (status, body) = h
        .call(None, "DELETE", &format!("/api/groups/{g}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = h
        .call(None, "DELETE", &format!("/api/groups/{g}"), None)
        .await;
    assert_code(status, &body, StatusCode::NOT_FOUND, "group_not_found");
}

#[tokio::test]
async fn delete_removes_the_group_and_keeps_the_bros() {
    let h = harness();
    let (ada, ada_dm) = h.bro(Some(MEI), "Ada").await;
    let (bob, _) = h.bro(Some(OTTO), "Bob").await;
    let g = h.group(Some(MEI), "作战组").await;
    let other = h.group(Some(MEI), "别的群").await;
    h.add(Some(MEI), &g, &ada).await;
    h.add(Some(OTTO), &g, &bob).await;
    h.add(Some(MEI), &other, &ada).await;
    h.say(Some(MEI), &g, "要删掉的消息内容").await;
    let kept_msg = h.say(Some(MEI), &other, "要留下的消息内容").await;

    // A running task with a container.
    let (status, task) = h
        .call(
            Some(MEI),
            "POST",
            &format!("/api/groups/{g}/tasks"),
            Some(json!({ "assigneeBotId": ada, "prompt": "写周报" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["id"].as_str().unwrap().to_string();
    let mut fake = FakeSandbox::new();
    h.host.pump_once(&mut fake).unwrap();
    assert_eq!(fake.names(), vec![format!("robroz-{task_id}")]);
    assert!(
        h.volumes.join(&g).exists(),
        "task files under the group volume"
    );
    std::fs::create_dir_all(h.control.join(&task_id)).unwrap();

    // An open bro card: the owner's DM shows it as a reminder.
    let (status, card) = h
        .call(
            None,
            "POST",
            &format!("/api/groups/{g}/approvals"),
            Some(json!({ "botId": ada, "action": "send", "summary": "发周报" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{card}");
    let card_id = card["id"].as_str().unwrap().to_string();

    // Rows the delete must clear.
    {
        let conn = h.conn();
        conn.execute(
            "INSERT INTO group_reads (group_id, read_at) VALUES (?1, '2026-01-01T00:00:00.000Z')",
            [&g],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO group_skills (group_id, slug, created_at)
             VALUES (?1, 'weekly', '2026-01-01T00:00:00.000Z')",
            [&g],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, scope, kind, text, source, status, created_at, updated_at)
             VALUES ('mem-ada', ?1, 'fact', '周五交周报', 'user', 'active',
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [format!("bot:{ada}")],
        )
        .unwrap();
    }

    let (status, body) = h
        .call(Some(MEI), "DELETE", &format!("/api/groups/{g}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(body, Value::Null, "204 has no body");

    for table in [
        "groups WHERE id",
        "messages WHERE group_id",
        "tasks WHERE group_id",
        "deliveries WHERE group_id",
        "schedules WHERE group_id",
        "choices WHERE group_id",
        "memberships WHERE group_id",
        "group_reads WHERE group_id",
        "group_skills WHERE group_id",
        "group_mcp WHERE group_id",
        "turns WHERE group_id",
    ] {
        assert_eq!(
            h.count(&format!("SELECT COUNT(*) FROM {table} = ?1"), &g),
            0,
            "{table}"
        );
    }
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?1",
            "\"要删掉\""
        ),
        0,
        "the search index follows"
    );
    // Kept: both bros, their DMs, Ada's memory, the other group and its message.
    assert_eq!(h.count("SELECT COUNT(*) FROM bots WHERE id = ?1", &ada), 1);
    assert_eq!(h.count("SELECT COUNT(*) FROM bots WHERE id = ?1", &bob), 1);
    assert_eq!(
        h.count("SELECT COUNT(*) FROM groups WHERE id = ?1", &ada_dm),
        1
    );
    assert_eq!(
        h.count("SELECT COUNT(*) FROM memories WHERE id = ?1", "mem-ada"),
        1
    );
    assert_eq!(
        h.count("SELECT COUNT(*) FROM messages WHERE id = ?1", &kept_msg),
        1
    );
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM memberships WHERE group_id = ?1",
            &other
        ),
        1
    );

    // Files go after commit; pooled MCP processes are stopped.
    assert!(!h.volumes.join(&g).exists());
    assert!(!h.control.join(&task_id).exists());
    assert_eq!(*h.mcp.0.lock().unwrap(), vec![g.clone()]);

    // The pump stops the container on its next pass.
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM container_reaps WHERE task_id = ?1",
            &task_id
        ),
        1
    );
    h.host.pump_once(&mut fake).unwrap();
    assert!(fake.names().is_empty(), "container stopped");
    assert_eq!(
        h.count(
            "SELECT COUNT(*) FROM container_reaps WHERE task_id = ?1",
            &task_id
        ),
        0
    );

    // The owner's DM still shows the card, now expired, under the old name.
    let (_, events) = h
        .call(
            Some(MEI),
            "GET",
            &format!("/api/groups/{ada_dm}/events"),
            None,
        )
        .await;
    let reminders: Vec<&Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "approval_reminder")
        .collect();
    assert_eq!(reminders.len(), 1, "{events}");
    assert_eq!(reminders[0]["state"], "expired", "{}", reminders[0]);
    assert_eq!(reminders[0]["groupName"], "作战组", "{}", reminders[0]);
    let (status, body) = h
        .call(
            Some(MEI),
            "POST",
            &format!("/api/approvals/{card_id}/approve"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

// ---- search ----

#[tokio::test]
async fn search_ranks_matches_and_is_for_members() {
    let h = harness();
    let (ada, _) = h.bro(Some(MEI), "Ada").await;
    let g = h.group(Some(MEI), "搜索").await;
    let other = h.group(Some(MEI), "别处").await;
    h.add(Some(MEI), &g, &ada).await;
    let weak = h
        .say(
            Some(MEI),
            &g,
            "今天开会讨论了很多事情，其中顺便提到了周报，之后又聊了别的很长很长的话题和安排",
        )
        .await;
    let strong = h.say(Some(MEI), &g, "周报周报：周报模板").await;
    h.say(Some(MEI), &g, "完全无关的内容").await;
    h.say(Some(MEI), &other, "别的群里的周报").await;

    let (status, body) = h.search(Some(MEI), &g, "q=%E5%91%A8%E6%8A%A5").await; // 周报 (2 chars)
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["messageId"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![strong.as_str(), weak.as_str()],
        "short query: newest first"
    );

    let (status, body) = h
        .search(Some(MEI), &g, "q=%E5%91%A8%E6%8A%A5%E6%A8%A1")
        .await; // 周报模
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
    let hit = &body["results"][0];
    assert_eq!(hit["messageId"], strong);
    assert_eq!(hit["authorUserId"], MEI);
    assert_eq!(hit["authorName"], "Mei");
    assert!(hit["createdAt"].is_string());
    assert!(
        hit["snippet"].as_str().unwrap().contains("周报模板"),
        "{hit}"
    );

    // Ranking: a message that says it more often and is shorter comes first.
    let (_, body) = h
        .search(Some(MEI), &g, "q=%E5%91%A8%E6%8A%A5%E5%91%A8")
        .await; // 周报周
    assert_eq!(body["results"][0]["messageId"], strong, "{body}");
    h.say(Some(MEI), &g, "abc report weekly").await;
    let long = h
        .say(Some(MEI), &g, "the weekly thing, said once in a very long and rambling message about many other topics")
        .await;
    let short = h.say(Some(MEI), &g, "weekly weekly weekly").await;
    let (_, body) = h.search(Some(MEI), &g, "q=Weekly").await;
    let ids: Vec<&str> = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["messageId"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "{body}");
    assert_eq!(ids[0], short.as_str(), "{body}");
    assert!(ids.contains(&long.as_str()));

    // limit
    let (_, body) = h.search(Some(MEI), &g, "q=weekly&limit=1").await;
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
    let (status, _) = h.search(Some(MEI), &g, "q=weekly&limit=1000").await;
    assert_eq!(status, StatusCode::OK, "clamped to the max");
    let (status, body) = h.search(Some(MEI), &g, "q=weekly&limit=x").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["code"].is_string(), "{body}");

    // Operators are plain text, not FTS syntax.
    let (status, body) = h.search(Some(MEI), &g, "q=%22weekly%20OR%20*").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = h.search(Some(MEI), &g, "q=%25").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["results"].as_array().unwrap().is_empty(),
        "% is literal"
    );

    let (status, body) = h.search(Some(MEI), &g, "q=%20%20").await;
    assert_code(status, &body, StatusCode::BAD_REQUEST, "need_text");
    let (status, body) = h.search(Some(MEI), &g, "").await;
    assert_code(status, &body, StatusCode::BAD_REQUEST, "need_text");
    let (status, body) = h.search(Some(OTTO), &g, "q=weekly").await;
    assert_code(status, &body, StatusCode::FORBIDDEN, "not_group_member");
    let (status, body) = h.search(Some(MEI), "nope", "q=weekly").await;
    assert_code(status, &body, StatusCode::NOT_FOUND, "group_not_found");
}

#[tokio::test]
async fn the_search_index_is_built_from_existing_messages() {
    let db = temp("groups-fts").join("robroz.db");
    let g = {
        let h = harness_on(db.clone());
        let g = h.group(None, "老消息").await;
        h.say(None, &g, "升级前就有的消息 robroz").await;
        g
    };
    // An install from before the index: drop it, as if it never existed.
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "DROP TRIGGER messages_fts_ai; DROP TRIGGER messages_fts_ad;
             DROP TRIGGER messages_fts_au; DROP TABLE messages_fts;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES ('m-old', ?1, 'text', 'older robroz note', ?2)",
            params![g, "2026-01-01T00:00:00.000Z"],
        )
        .unwrap();
    }
    let h = harness_on(db);
    let (status, body) = h.search(None, &g, "q=robroz").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"].as_array().unwrap().len(), 2, "{body}");
    assert!(
        body["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["messageId"] == "m-old"),
        "{body}"
    );
}

#[test]
fn like_snippets_cut_around_the_match() {
    let body = format!("{}周报{}", "前".repeat(40), "后".repeat(40));
    let s = super::like_snippet(&body, "周报");
    assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
    assert!(s.contains("周报"));
    assert_eq!(super::like_snippet("短周报", "周报"), "短周报");
}
