use super::*;
use axum::body::Body;
use axum::http::Request;
use axum::routing::{get, post};
use axum::Router;
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum Kind {
    Text,
    Image,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Body1 {
    name: String,
    kind: Kind,
    count: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Page {
    limit: u32,
}

fn app() -> Router {
    Router::new()
        .route(
            "/body",
            post(|Json(_): Json<Body1>| async { Json(json!({"ok": true})) }),
        )
        .route(
            "/optional",
            post(|body: Option<Json<Body1>>| async move { Json(json!({"some": body.is_some()})) }),
        )
        .route(
            "/n/{n}",
            get(|Path(n): Path<u32>| async move { Json(json!({ "n": n })) }),
        )
        .route(
            "/s/{s}",
            get(|Path(s): Path<String>| async move { Json(json!({ "s": s })) }),
        )
        .route(
            "/page",
            get(|Query(p): Query<Page>| async move { Json(json!({ "limit": p.limit })) }),
        )
}

async fn send(request: Request<axum::body::Body>) -> (StatusCode, String, Value) {
    let response = app().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&bytes)));
    (status, content_type, value)
}

fn post_raw(uri: &str, content_type: Option<&str>, body: &str) -> Request<axum::body::Body> {
    let mut builder = Request::builder().method("POST").uri(uri);
    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn get_req(uri: &str) -> Request<axum::body::Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn assert_coded(request: Request<axum::body::Body>, status: StatusCode, code: &str) {
    let (got, content_type, body) = send(request).await;
    assert_eq!(got, status, "{body}");
    assert!(
        content_type.starts_with("application/json"),
        "{content_type}"
    );
    assert_eq!(body["code"], code, "{body}");
    assert!(
        body["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{body}"
    );
}

const JSON: Option<&str> = Some("application/json");

#[tokio::test]
async fn invalid_json_is_400_invalid_json() {
    assert_coded(
        post_raw("/body", JSON, "{not json"),
        StatusCode::BAD_REQUEST,
        "invalid_json",
    )
    .await;
    // Cut short is a syntax error too.
    assert_coded(
        post_raw("/body", JSON, r#"{"name":"#),
        StatusCode::BAD_REQUEST,
        "invalid_json",
    )
    .await;
}

#[tokio::test]
async fn missing_field_is_422_missing_field() {
    assert_coded(
        post_raw("/body", JSON, r#"{"kind":"text"}"#),
        StatusCode::UNPROCESSABLE_ENTITY,
        "missing_field",
    )
    .await;
}

#[tokio::test]
async fn unknown_enum_value_is_422_invalid_enum() {
    assert_coded(
        post_raw("/body", JSON, r#"{"name":"a","kind":"video"}"#),
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_enum",
    )
    .await;
}

#[tokio::test]
async fn unknown_field_is_422_unknown_field() {
    assert_coded(
        post_raw("/body", JSON, r#"{"name":"a","kind":"text","extra":1}"#),
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown_field",
    )
    .await;
}

#[tokio::test]
async fn wrong_type_is_422_invalid_field() {
    assert_coded(
        post_raw("/body", JSON, r#"{"name":7,"kind":"text"}"#),
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_field",
    )
    .await;
    assert_coded(
        post_raw("/body", JSON, r#"{"name":"a","kind":"text","count":-1}"#),
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_field",
    )
    .await;
}

#[tokio::test]
async fn wrong_content_type_is_415_unsupported_media_type() {
    assert_coded(
        post_raw("/body", Some("text/plain"), r#"{"name":"a","kind":"text"}"#),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
    )
    .await;
    assert_coded(
        post_raw("/body", None, r#"{"name":"a","kind":"text"}"#),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
    )
    .await;
}

#[tokio::test]
async fn optional_body_is_none_without_content_type_and_coded_when_bad() {
    let (status, _, body) = send(post_raw("/optional", None, "")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"some": false})));
    assert_coded(
        post_raw("/optional", JSON, "{"),
        StatusCode::BAD_REQUEST,
        "invalid_json",
    )
    .await;
    assert_coded(
        post_raw("/optional", Some("text/plain"), "x"),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
    )
    .await;
}

#[tokio::test]
async fn path_that_does_not_parse_is_400_invalid_path() {
    assert_coded(get_req("/n/abc"), StatusCode::BAD_REQUEST, "invalid_path").await;
    // Not UTF-8 after percent-decoding.
    assert_coded(get_req("/s/%FF"), StatusCode::BAD_REQUEST, "invalid_path").await;
}

#[tokio::test]
async fn query_that_does_not_parse_is_400_invalid_query() {
    assert_coded(
        get_req("/page?limit=lots"),
        StatusCode::BAD_REQUEST,
        "invalid_query",
    )
    .await;
    assert_coded(get_req("/page"), StatusCode::BAD_REQUEST, "invalid_query").await;
}

#[tokio::test]
async fn good_requests_still_pass_and_responses_are_plain_json() {
    let (status, content_type, body) =
        send(post_raw("/body", JSON, r#"{"name":"a","kind":"image"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("application/json"));
    assert_eq!(body, json!({"ok": true}));
    let (_, _, body) = send(get_req("/n/7")).await;
    assert_eq!(body, json!({"n": 7}));
    let (_, _, body) = send(get_req("/page?limit=5")).await;
    assert_eq!(body, json!({"limit": 5}));
}

#[test]
fn every_code_used_here_is_in_the_contract_list() {
    for code in [
        code::INVALID_JSON,
        code::MISSING_FIELD,
        code::INVALID_ENUM,
        code::UNKNOWN_FIELD,
        code::INVALID_FIELD,
        code::UNSUPPORTED_MEDIA_TYPE,
        code::BODY_TOO_LARGE,
        code::INVALID_BODY,
        code::INVALID_PATH,
        code::INVALID_QUERY,
    ] {
        assert!(robroz_contract::error_codes::rejection::ALL.contains(&code));
    }
}

#[tokio::test]
async fn body_over_the_limit_is_413_body_too_large() {
    let big = format!(
        r#"{{"name":"{}","kind":"text"}}"#,
        "x".repeat(3 * 1024 * 1024)
    );
    assert_coded(
        post_raw("/body", JSON, &big),
        StatusCode::PAYLOAD_TOO_LARGE,
        "body_too_large",
    )
    .await;
}
