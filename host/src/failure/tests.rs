use super::*;
use robroz_contract::split_failure;
use rusqlite::Connection;

fn pending(error: &str) -> crate::mcp_tools::PendingError {
    crate::mcp_tools::PendingError::for_test(error)
}

#[test]
fn every_typed_failure_keeps_its_legacy_text_and_code() {
    use robroz_policy::OverQuota;
    let cases = [
        Failure::no_result(),
        Failure::approval_expired(),
        Failure::round_limit(),
        Failure::denied_line("发送周报", Some("发送周报")),
        Failure::denied_tool("notes", "write_note"),
        Failure::model(&robroz_model::ModelError::from_http_status(401)),
        Failure::over_quota(&OverQuota::TooMany { used: 3, limit: 2 }),
        Failure::over_quota(&OverQuota::NotGet("POST".into())),
        Failure::over_quota_unknown(),
        Failure::waiting_approval("发邮件", Some("send_email")),
        Failure::save_approval_state(&pending("disk full")),
        Failure::resume_approved_call(&pending("disk full")),
    ];
    for failure in cases {
        // The legacy text an older client reads still maps to the same code.
        assert_eq!(
            split_failure(&failure.text).0,
            failure.data.code,
            "{failure:?}"
        );
        let back: FailureData = serde_json::from_str(&failure.json()).unwrap();
        assert_eq!(back, failure.data);
    }
}

#[test]
fn detail_is_language_neutral_raw_data() {
    use robroz_model::ModelError;
    use robroz_policy::OverQuota;
    let detail = |f: Failure| f.data.detail();
    assert_eq!(
        detail(Failure::model(&ModelError::from_http_status(401))),
        Some("401".into())
    );
    assert_eq!(
        detail(Failure::model(&ModelError::with_error(
            "连不上模型服务：timed out",
            "connect_failed",
            "timed out"
        ))),
        Some("timed out".into())
    );
    assert_eq!(
        detail(Failure::model(&ModelError::with_reason(
            "模型返回的不是 JSON",
            "response_not_json"
        ))),
        Some("response_not_json".into())
    );
    assert_eq!(
        detail(Failure::over_quota(&OverQuota::TooMany {
            used: 3,
            limit: 2
        })),
        Some("3/2".into())
    );
    assert_eq!(
        detail(Failure::over_quota(&OverQuota::NotGet("POST".into()))),
        Some("POST".into())
    );
    assert_eq!(
        detail(Failure::denied_tool("notes", "write_note")),
        Some("notes.write_note".into())
    );
    assert_eq!(
        detail(Failure::waiting_approval("发邮件", Some("send_email"))),
        Some("send_email".into())
    );
    assert_eq!(
        detail(Failure::save_approval_state(&pending("disk full"))),
        Some("disk full".into())
    );
    assert_eq!(detail(Failure::no_result()), None);
    assert_eq!(detail(Failure::approval_expired()), None);
    assert_eq!(detail(Failure::round_limit()), None);
    assert_eq!(detail(Failure::denied_card("调用 x")), None);
}

#[test]
fn codes_prefers_the_typed_column_and_falls_back_to_the_text() {
    let typed = Failure::model(&robroz_model::ModelError::from_http_status(401));
    assert_eq!(
        codes(Some(&typed.text), Some(&typed.json())),
        (Some(FailureCode::ModelNoReply), Some("401".into()))
    );
    assert_eq!(
        codes(Some("模型没回：模型服务返回 401"), None),
        (Some(FailureCode::ModelNoReply), Some("401".into()))
    );
    assert_eq!(codes(None, None), (None, None));
}

fn legacy_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE tasks (id TEXT PRIMARY KEY, failure TEXT);
         CREATE TABLE deliveries (id TEXT PRIMARY KEY, failure TEXT);
         INSERT INTO tasks VALUES ('t1', '模型没回：模型服务返回 401');
         INSERT INTO tasks VALUES ('t2', '出网超额：已用 3 次，上限 2');
         INSERT INTO tasks VALUES ('t3', '你拒绝了：notes.write_note');
         INSERT INTO tasks VALUES ('t4', NULL);
         INSERT INTO tasks VALUES ('t5', 'runner exploded');
         INSERT INTO deliveries VALUES ('d1', '没写出结果');
         INSERT INTO deliveries VALUES ('d2', '无法继续已批准的调用：待批准状态损坏：EOF');",
    )
    .unwrap();
    conn
}

fn data_of(conn: &Connection, table: &str, id: &str) -> Option<FailureData> {
    let raw: Option<String> = conn
        .query_row(
            &format!("SELECT failure_data FROM {table} WHERE id = ?1"),
            [id],
            |r| r.get(0),
        )
        .unwrap();
    raw.map(|r| serde_json::from_str(&r).unwrap())
}

#[test]
fn migrate_adds_the_column_and_backfills_legacy_rows_once() {
    let conn = legacy_db();
    migrate(&conn).unwrap();
    let t1 = data_of(&conn, "tasks", "t1").unwrap();
    assert_eq!(t1.code, FailureCode::ModelNoReply);
    assert_eq!(t1.params.status, Some(401));
    assert_eq!(t1.detail().as_deref(), Some("401"));
    let t2 = data_of(&conn, "tasks", "t2").unwrap();
    assert_eq!((t2.params.used, t2.params.limit), (Some(3), Some(2)));
    assert_eq!(t2.detail().as_deref(), Some("3/2"));
    let t3 = data_of(&conn, "tasks", "t3").unwrap();
    assert_eq!(t3.params.tool.as_deref(), Some("notes.write_note"));
    assert_eq!(data_of(&conn, "tasks", "t4"), None);
    let t5 = data_of(&conn, "tasks", "t5").unwrap();
    assert_eq!(t5.code, FailureCode::Other);
    assert_eq!(t5.detail().as_deref(), Some("runner exploded"));
    assert_eq!(
        data_of(&conn, "deliveries", "d1").unwrap().code,
        FailureCode::NoResult
    );
    let d2 = data_of(&conn, "deliveries", "d2").unwrap();
    assert_eq!(d2.code, FailureCode::ResumeApprovedCallFailed);
    assert_eq!(d2.params.reason.as_deref(), Some("pending_corrupt"));
    assert_eq!(d2.detail().as_deref(), Some("EOF"));
    // The legacy text is untouched.
    let text: String = conn
        .query_row("SELECT failure FROM tasks WHERE id = 't1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(text, "模型没回：模型服务返回 401");
    // A second start reads no row again.
    migrate(&conn).unwrap();
    assert_eq!(backfill(&conn).unwrap(), 0);
}

#[test]
fn backfill_never_overwrites_typed_rows() {
    let conn = legacy_db();
    migrate(&conn).unwrap();
    let typed = Failure::model(&robroz_model::ModelError::from_http_status(503));
    conn.execute(
        "UPDATE tasks SET failure = ?1, failure_data = ?2 WHERE id = 't1'",
        rusqlite::params![typed.text, typed.json()],
    )
    .unwrap();
    assert_eq!(backfill(&conn).unwrap(), 0);
    assert_eq!(data_of(&conn, "tasks", "t1").unwrap(), typed.data);
}
