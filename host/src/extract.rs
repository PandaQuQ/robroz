//! Extractors whose rejections are JSON error bodies with a stable `code`
//! (#179).
//!
//! axum's own `Json`, `Path` and `Query` reject a bad request before the
//! handler runs, with a plain-text body and no `code`. These wrappers run the
//! same axum extractor and turn its rejection into the host's usual
//! `{"error": …, "code": …}` body, keeping axum's status. `error` is axum's
//! English text, for logs only; a client picks its copy from `code`
//! ([`robroz_contract::error_codes::rejection`]).
//!
//! `Json` is also the response type: it serializes exactly like `axum::Json`,
//! so modules import it in place of `axum::Json`.

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequest, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use robroz_contract::error_codes::rejection as code;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// [`axum::Json`] with a coded JSON rejection.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

/// [`axum::extract::Path`] with a coded JSON rejection.
#[derive(Debug)]
pub struct Path<T>(pub T);

/// [`axum::extract::Query`] with a coded JSON rejection.
#[derive(Debug)]
pub struct Query<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match <axum::Json<T> as FromRequest<S>>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(json_rejection(&rejection)),
        }
    }
}

/// `Option<Json<T>>`: `None` without a `Content-Type`, as with `axum::Json`.
impl<T, S> OptionalFromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        match <axum::Json<T> as OptionalFromRequest<S>>::from_request(req, state).await {
            Ok(value) => Ok(value.map(|axum::Json(value)| Self(value))),
            Err(rejection) => Err(json_rejection(&rejection)),
        }
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

impl<T> From<T> for Json<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Self(value)),
            Err(rejection) => Err(path_rejection(&rejection)),
        }
    }
}

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(query_rejection(&rejection)),
        }
    }
}

fn coded(status: StatusCode, code: &str, error: String) -> Response {
    (
        status,
        axum::Json(robroz_contract::CodedError {
            error,
            code: code.to_string(),
        }),
    )
        .into_response()
}

/// The code for a JSON body rejection.
pub(crate) fn json_rejection_code(rejection: &JsonRejection) -> &'static str {
    match rejection {
        JsonRejection::JsonSyntaxError(_) => code::INVALID_JSON,
        JsonRejection::MissingJsonContentType(_) => code::UNSUPPORTED_MEDIA_TYPE,
        JsonRejection::JsonDataError(err) => serde_data_code(&err.body_text()),
        JsonRejection::BytesRejection(_) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            code::BODY_TOO_LARGE
        }
        _ => code::INVALID_BODY,
    }
}

/// serde's data errors by kind. The texts are serde's own `de::Error`
/// messages (`missing_field`, `unknown_variant`, `unknown_field`), which
/// serde has kept stable; anything else is a field of the wrong type or value.
fn serde_data_code(text: &str) -> &'static str {
    if text.contains("missing field `") {
        code::MISSING_FIELD
    } else if text.contains("unknown variant `") {
        code::INVALID_ENUM
    } else if text.contains("unknown field `") {
        code::UNKNOWN_FIELD
    } else {
        code::INVALID_FIELD
    }
}

fn json_rejection(rejection: &JsonRejection) -> Response {
    coded(
        rejection.status(),
        json_rejection_code(rejection),
        rejection.body_text(),
    )
}

fn path_rejection(rejection: &PathRejection) -> Response {
    coded(
        rejection.status(),
        code::INVALID_PATH,
        rejection.body_text(),
    )
}

fn query_rejection(rejection: &QueryRejection) -> Response {
    coded(
        rejection.status(),
        code::INVALID_QUERY,
        rejection.body_text(),
    )
}

#[cfg(test)]
mod tests;
