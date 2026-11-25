//! Every 4xx and 5xx JSON error body carries a stable `code`.
//!
//! The host text in `error` (and `message`) is for logs only; a client picks
//! its own copy from `code` (and `reason` on the approval 403). Handlers that
//! know a more specific code (`approval_forbidden`, `not_bro_owner`) set it
//! themselves. Every other error body gets its code here, in one place, so no
//! handler can forget it: the code of a fixed host text from
//! [`robroz_contract::error_codes`], else the code of its HTTP status.

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::Value;

/// Error bodies are a line of text; anything larger is not rewritten.
const MAX_ERROR_BODY: usize = 64 * 1024;

pub async fn add_error_code(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return response;
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_ERROR_BODY).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let rewritten = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(mut map)) if !map.contains_key("code") => {
            let code = map
                .get("error")
                .and_then(Value::as_str)
                .and_then(robroz_contract::error_codes::code_for_message)
                .unwrap_or_else(|| robroz_contract::error_codes::fallback_code(status.as_u16()));
            map.insert("code".into(), Value::String(code.to_string()));
            serde_json::to_vec(&Value::Object(map)).ok()
        }
        _ => None,
    };
    match rewritten {
        Some(bytes) => {
            parts.headers.remove(header::CONTENT_LENGTH);
            parts.headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            Response::from_parts(parts, Body::from(bytes))
        }
        None => Response::from_parts(parts, Body::from(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route(
                "/fixed",
                get(|| async {
                    (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": robroz_contract::host_text::GROUP_NOT_FOUND })),
                    )
                }),
            )
            .route(
                "/dynamic",
                get(|| async {
                    (
                        StatusCode::BAD_GATEWAY,
                        Json(json!({ "error": "连不上 MCP 服务器：boom" })),
                    )
                }),
            )
            .route(
                "/coded",
                get(|| async {
                    (
                        StatusCode::FORBIDDEN,
                        Json(json!({ "error": "x", "code": "approval_forbidden", "reason": "decided" })),
                    )
                }),
            )
            .route(
                "/extra",
                get(|| async {
                    (
                        StatusCode::CONFLICT,
                        Json(json!({ "error": "command_not_approved", "command": "npx", "args": [] })),
                    )
                }),
            )
            .route("/ok", get(|| async { Json(json!({ "error": "not an error" })) }))
            .route(
                "/text",
                get(|| async { (StatusCode::UNPROCESSABLE_ENTITY, "plain").into_response() }),
            )
            .layer(from_fn(add_error_code))
    }

    async fn get_json(path: &str) -> (StatusCode, String, String) {
        let response = app()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (
            status,
            content_type,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    async fn body_of(path: &str) -> (StatusCode, Value) {
        let (status, _, body) = get_json(path).await;
        (status, serde_json::from_str(&body).unwrap())
    }

    #[tokio::test]
    async fn a_fixed_host_text_gets_its_own_code() {
        let (status, body) = body_of("/fixed").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            body,
            json!({ "error": "群不存在", "code": "group_not_found" })
        );
    }

    #[tokio::test]
    async fn a_dynamic_text_gets_the_status_code() {
        let (status, body) = body_of("/dynamic").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["code"], "bad_gateway");
        assert_eq!(body["error"], "连不上 MCP 服务器：boom");
    }

    #[tokio::test]
    async fn an_existing_code_and_extra_fields_are_kept() {
        let (_, body) = body_of("/coded").await;
        assert_eq!(
            body,
            json!({ "error": "x", "code": "approval_forbidden", "reason": "decided" })
        );
        let (_, body) = body_of("/extra").await;
        assert_eq!(body["code"], "command_not_approved");
        assert_eq!(body["command"], "npx");
    }

    #[tokio::test]
    async fn success_and_non_json_bodies_are_untouched() {
        let (status, body) = body_of("/ok").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "error": "not an error" }));
        let (status, content_type, body) = get_json("/text").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(content_type.starts_with("text/plain"), "{content_type}");
        assert_eq!(body, "plain");
    }
}
