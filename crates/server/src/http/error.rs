//! The one error shape of the API: `{"error": {"code": string, "message": string}}`.

use std::any::Any;
use std::borrow::Cow;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Every error response's body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ErrorDetail {
    /// Stable and meant for programs, such as `not_found` or `invalid_json`.
    #[schema(example = "not_found")]
    pub code: String,
    /// What went wrong, for people.
    #[schema(example = "no such route")]
    pub message: String,
}

/// An error answer: a status, a stable code and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: Cow<'static, str>,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// A failure the client cannot fix. Its cause belongs in the logs, never in the
    /// answer, so the message is always the same.
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// An extractor's rejection. Server-side ones are bugs in a route, not the client's
    /// doing, so they are logged and answered as internal errors.
    fn rejected(status: StatusCode, code: &'static str, message: String) -> Self {
        if status.is_server_error() {
            tracing::error!(%status, message, "request rejected by a misconfigured extractor");
            Self::internal()
        } else {
            Self::new(status, code, message)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code.to_owned(),
                message: self.message.into_owned(),
            },
        };
        (self.status, axum::Json(body)).into_response()
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        let status = rejection.status();
        let code = match &rejection {
            JsonRejection::JsonSyntaxError(_) | JsonRejection::JsonDataError(_) => "invalid_json",
            JsonRejection::MissingJsonContentType(_) => "unsupported_media_type",
            _ if status == StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
            _ => "invalid_body",
        };
        Self::rejected(status, code, rejection.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::rejected(rejection.status(), "invalid_path", rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::rejected(rejection.status(), "invalid_query", rejection.body_text())
    }
}

/// Answers requests no route matches.
pub(super) async fn no_route() -> ApiError {
    ApiError::not_found("no such route")
}

/// Answers requests whose route exists but takes other methods.
pub(super) async fn wrong_method() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this route does not take that method",
    )
}

/// Turns a panic in a handler into an internal error, so the client still gets the
/// error shape and the connection stays usable.
pub(super) fn panicked(panic: Box<dyn Any + Send + 'static>) -> Response {
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message");
    tracing::error!(panic = message, "a request handler panicked");
    ApiError::internal().into_response()
}
