//! The one error shape of the API: `{"error": {"code": string, "message": string}}`, with
//! the problems of each field when there were several.

use std::any::Any;
use std::borrow::Cow;
use std::time::Duration;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use neon_app::auth::AuthError;
use neon_app::databases::DatabaseError;
use neon_app::oauth::OAuthError;
use neon_app::pages::PageError;
use neon_app::settings::UpdateError;
use neon_app::sync::ContentError;
use neon_app::transfer::TransferError;
use neon_app::webhooks::{IncomingError, WebhookError};
use neon_app::workspaces::WorkspaceError;
use neon_domain::settings::Problem;
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
    /// Each field that is wrong and why, when a request had several, such as settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ProblemDetail>,
}

/// One field that is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProblemDetail {
    /// The field, such as a setting's key.
    #[schema(example = "auth.min_password_length")]
    pub key: String,
    #[schema(example = "must be between 8 and 128")]
    pub message: String,
}

/// An error answer: a status, a stable code and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: Cow<'static, str>,
    problems: Vec<Problem>,
    /// Sent as `Retry-After`, in whole seconds rounded up.
    retry_after: Option<Duration>,
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
            problems: Vec::new(),
            retry_after: None,
        }
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn unauthenticated(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthenticated", message)
    }

    pub fn forbidden(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    /// The same answer, telling the client how long to wait before it tries again.
    #[must_use]
    pub fn with_retry_after(mut self, wait: Duration) -> Self {
        self.retry_after = Some(wait);
        self
    }

    /// The same answer, naming each field it refuses.
    #[must_use]
    pub fn with_problems(mut self, problems: Vec<Problem>) -> Self {
        self.problems = problems;
        self
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
                problems: self
                    .problems
                    .into_iter()
                    .map(|problem| ProblemDetail {
                        key: problem.key,
                        message: problem.message,
                    })
                    .collect(),
            },
        };
        let mut response = (self.status, axum::Json(body)).into_response();
        if let Some(wait) = self.retry_after {
            let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(seconds.max(1)));
        }
        response
    }
}

impl From<AuthError> for ApiError {
    /// Expected errors answer with their message, which never holds a secret. Others are
    /// logged with their cause, and answer as internal errors.
    fn from(error: AuthError) -> Self {
        let status = match &error {
            AuthError::InvalidInput(_)
            | AuthError::InvalidEmail(_)
            | AuthError::WeakPassword(_)
            | AuthError::InvalidInvite
            | AuthError::InviteForOtherEmail
            | AuthError::InvalidLink => StatusCode::BAD_REQUEST,
            AuthError::InvalidCredentials | AuthError::Unauthenticated => StatusCode::UNAUTHORIZED,
            AuthError::UserDisabled
            | AuthError::EmailNotVerified
            | AuthError::RegistrationClosed
            | AuthError::Forbidden(_)
            | AuthError::PasswordResetOff
            | AuthError::InviteEmailOff
            | AuthError::RequestsOff => StatusCode::FORBIDDEN,
            AuthError::NotFound(_) => StatusCode::NOT_FOUND,
            AuthError::InviteCodeTaken
            | AuthError::EmailTaken
            | AuthError::LastAdmin
            | AuthError::MailNotSetUp(_)
            | AuthError::CodeUnavailable(_) => StatusCode::CONFLICT,
            AuthError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            AuthError::Database(_) | AuthError::Internal(_) => {
                tracing::error!(error = %error, "an auth use case failed");
                return Self::internal();
            }
        };
        let mut api = Self::new(status, error.code(), error.to_string());
        if let AuthError::RateLimited { retry_after } = error {
            api.retry_after = Some(retry_after);
        }
        api
    }
}

impl From<WorkspaceError> for ApiError {
    /// Expected errors answer with their message, which never holds a secret. Others are
    /// logged with their cause, and answer as internal errors.
    fn from(error: WorkspaceError) -> Self {
        let status = match &error {
            WorkspaceError::InvalidInput(_)
            | WorkspaceError::ConfirmationMismatch
            | WorkspaceError::InvalidInviteLink => StatusCode::BAD_REQUEST,
            WorkspaceError::Forbidden(_) => StatusCode::FORBIDDEN,
            WorkspaceError::NotFound(_) | WorkspaceError::NoAccount => StatusCode::NOT_FOUND,
            WorkspaceError::AlreadyMember
            | WorkspaceError::AlreadyInvited
            | WorkspaceError::MemberLimit { .. }
            | WorkspaceError::AgentLimit { .. }
            | WorkspaceError::LastOwner(_)
            | WorkspaceError::LastTeamspaceOwner(_)
            | WorkspaceError::SlugTaken
            | WorkspaceError::TokenUnavailable(_) => StatusCode::CONFLICT,
            WorkspaceError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            WorkspaceError::Database(_) | WorkspaceError::Internal(_) => {
                tracing::error!(error = %error, "a workspace use case failed");
                return Self::internal();
            }
        };
        let mut api = Self::new(status, error.code(), error.to_string());
        if let WorkspaceError::RateLimited { retry_after } = error {
            api.retry_after = Some(retry_after);
        }
        api
    }
}

impl From<PageError> for ApiError {
    /// Expected errors answer with their message, which never holds a secret. Others are
    /// logged with their cause, and answer as internal errors.
    fn from(error: PageError) -> Self {
        let status = match &error {
            PageError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            PageError::Forbidden(_) => StatusCode::FORBIDDEN,
            PageError::NotFound(_) => StatusCode::NOT_FOUND,
            PageError::Cycle
            | PageError::TooDeep
            | PageError::AlreadyShared
            | PageError::AlreadyFavorite
            | PageError::TooManyRows => StatusCode::CONFLICT,
            PageError::Database(_) | PageError::Internal(_) => {
                tracing::error!(error = %error, "a page use case failed");
                return Self::internal();
            }
        };
        Self::new(status, error.code(), error.to_string())
    }
}

impl From<DatabaseError> for ApiError {
    /// Expected errors answer with their message, which never holds a secret or a value.
    /// Others are logged with their cause, and answer as internal errors.
    fn from(error: DatabaseError) -> Self {
        match error {
            DatabaseError::Page(error) => error.into(),
            DatabaseError::Database(_) | DatabaseError::Internal(_) => {
                tracing::error!(error = %error, "a database use case failed");
                Self::internal()
            }
            expected => {
                let status = match &expected {
                    DatabaseError::Forbidden(_) => StatusCode::FORBIDDEN,
                    DatabaseError::NotFound(_) => StatusCode::NOT_FOUND,
                    DatabaseError::NameTaken(_)
                    | DatabaseError::LossyChange { .. }
                    | DatabaseError::TooMany(_)
                    | DatabaseError::AlreadyClaimed(_) => StatusCode::CONFLICT,
                    _ => StatusCode::BAD_REQUEST,
                };
                Self::new(status, expected.code(), expected.to_string())
            }
        }
    }
}

impl From<ContentError> for ApiError {
    /// Expected errors answer with their message, which never quotes the page. Others are
    /// logged with their cause, and answer as internal errors, or as unavailable while the
    /// server stops.
    fn from(error: ContentError) -> Self {
        match error {
            ContentError::Page(error) => error.into(),
            failed @ ContentError::Load(_) => {
                tracing::error!(error = %failed, "a page's content use case failed");
                Self::internal()
            }
            ContentError::RateLimited { retry_after } => Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "too many edits of pages' content: slow down",
            )
            .with_retry_after(retry_after),
            other => {
                let status = match &other {
                    ContentError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
                    ContentError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
                    _ => StatusCode::BAD_REQUEST,
                };
                Self::new(status, other.code(), other.to_string())
            }
        }
    }
}

impl From<TransferError> for ApiError {
    /// Expected errors answer with their message, which never holds a secret. Others are
    /// logged with their cause, and answer as internal errors.
    fn from(error: TransferError) -> Self {
        let (code, message) = (error.code(), error.to_string());
        let (status, retry_after) = match error {
            TransferError::Page(error) => return error.into(),
            TransferError::Database(error) => return error.into(),
            TransferError::Content { .. } | TransferError::Internal(_) => {
                tracing::error!(error = %message, "an import or export failed");
                return Self::internal();
            }
            TransferError::InvalidInput(_) => (StatusCode::BAD_REQUEST, None),
            TransferError::TooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, None),
            TransferError::RateLimited { retry_after } => {
                (StatusCode::TOO_MANY_REQUESTS, Some(retry_after))
            }
            TransferError::Busy => (
                StatusCode::SERVICE_UNAVAILABLE,
                Some(TransferError::BUSY_RETRY),
            ),
            TransferError::TimedOut(_) => (StatusCode::SERVICE_UNAVAILABLE, None),
        };
        let mut api = Self::new(status, code, message);
        api.retry_after = retry_after;
        api
    }
}

impl From<OAuthError> for ApiError {
    /// The consent page's and connected apps' errors, in the API's shape. Expected errors
    /// answer with their message, which never holds a code or token. Others are logged with
    /// their cause, and answer as internal errors.
    fn from(error: OAuthError) -> Self {
        let status = match &error {
            OAuthError::Request(_) | OAuthError::Protocol(_) | OAuthError::Scopes(_) => {
                StatusCode::BAD_REQUEST
            }
            OAuthError::IdentityUnavailable(_)
            | OAuthError::Forbidden(_)
            | OAuthError::RegistrationOff => StatusCode::FORBIDDEN,
            OAuthError::NotFound(_) => StatusCode::NOT_FOUND,
            OAuthError::ClientLimit => StatusCode::SERVICE_UNAVAILABLE,
            OAuthError::RateLimited { .. } | OAuthError::SourceLimit => {
                StatusCode::TOO_MANY_REQUESTS
            }
            OAuthError::Database(_) | OAuthError::Internal(_) => {
                tracing::error!(error = %error, "an OAuth use case failed");
                return Self::internal();
            }
        };
        let mut api = Self::new(status, error.code(), error.to_string());
        if let OAuthError::RateLimited { retry_after } = error {
            api.retry_after = Some(retry_after);
        }
        api
    }
}

impl From<WebhookError> for ApiError {
    /// Expected errors answer with their message, which never holds a secret or a URL.
    /// Others are logged with their cause, and answer as internal errors.
    fn from(error: WebhookError) -> Self {
        let status = match &error {
            WebhookError::InvalidInput(_)
            | WebhookError::InvalidUrl(_)
            | WebhookError::AddressRefused(_) => StatusCode::BAD_REQUEST,
            WebhookError::Forbidden(_) => StatusCode::FORBIDDEN,
            WebhookError::NotFound(_) => StatusCode::NOT_FOUND,
            WebhookError::NoSecretKey
            | WebhookError::AgentLimit { .. }
            | WebhookError::WebhookLimit { .. }
            | WebhookError::SetterLapsed(_) => StatusCode::CONFLICT,
            WebhookError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            WebhookError::Database(_) | WebhookError::Internal(_) => {
                tracing::error!(error = %error, "a webhook use case failed");
                return Self::internal();
            }
        };
        let mut api = Self::new(status, error.code(), error.to_string());
        if let WebhookError::RateLimited { retry_after } = error {
            api.retry_after = Some(retry_after);
        }
        api
    }
}

impl From<IncomingError> for ApiError {
    /// Expected errors answer with their message, which never holds the URL's token, a
    /// secret or the body. Others are logged with their cause, and answer as internal
    /// errors.
    fn from(error: IncomingError) -> Self {
        if !error.is_expected() {
            tracing::error!(error = %error, "an incoming webhook's request failed");
            return Self::internal();
        }
        let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::BAD_REQUEST);
        let mut api = Self::new(status, error.code(), error.to_string());
        if let IncomingError::RateLimited { retry_after } = error {
            api.retry_after = Some(retry_after);
        }
        api
    }
}

impl From<UpdateError> for ApiError {
    fn from(error: UpdateError) -> Self {
        match error {
            UpdateError::Invalid(problems) => Self {
                problems,
                ..Self::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_settings",
                    "nothing was saved: some settings are invalid",
                )
            },
            UpdateError::Database(error) => {
                tracing::error!(%error, "saving settings failed");
                Self::internal()
            }
        }
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
