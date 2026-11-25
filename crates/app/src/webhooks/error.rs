use std::time::Duration;

use neon_domain::duration;
use neon_domain::workspaces::Denied;

use super::signature::SignatureError;
use crate::databases::DatabaseError;
use crate::pages::PageError;
use crate::workspaces::WorkspaceError;

/// Why a webhook use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them. Messages
/// never hold a secret, a token or a URL.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("{0}")]
    InvalidInput(String),
    /// A URL that is no webhook URL.
    #[error("{0}")]
    InvalidUrl(String),
    /// A URL whose host does not resolve, or resolves to an address webhooks may not reach.
    #[error("{0}")]
    AddressRefused(String),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("secrets.keys is not set: webhooks keep their secrets encrypted with it")]
    NoSecretKey,
    #[error("this workspace has reached its limit of {limit} agents and integrations")]
    AgentLimit { limit: u64 },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl WebhookError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidUrl(_) => "invalid_url",
            Self::AddressRefused(_) => "address_refused",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::NoSecretKey => "no_secret_key",
            Self::AgentLimit { .. } => "agent_limit",
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }

    /// A refusal of the rules. What the caller may not know of is `missing`, as if it did
    /// not exist.
    pub(crate) fn invalid(error: impl std::fmt::Display) -> Self {
        Self::InvalidInput(error.to_string())
    }

    pub(crate) fn denied(denied: Denied, missing: &'static str) -> Self {
        match denied {
            Denied::Hidden => Self::NotFound(missing),
            Denied::Forbidden(reason) => Self::Forbidden(reason),
        }
    }
}

impl From<WorkspaceError> for WebhookError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::InvalidInput(message) => Self::InvalidInput(message),
            WorkspaceError::NotFound(missing) => Self::NotFound(missing),
            WorkspaceError::Forbidden(reason) => Self::Forbidden(reason),
            WorkspaceError::AgentLimit { limit } => Self::AgentLimit { limit },
            WorkspaceError::Database(error) => Self::Database(error),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<PageError> for WebhookError {
    fn from(error: PageError) -> Self {
        match error {
            PageError::InvalidInput(message) => Self::InvalidInput(message),
            PageError::NotFound(missing) => Self::NotFound(missing),
            PageError::Forbidden(reason) => Self::Forbidden(reason),
            PageError::Database(error) => Self::Database(error),
            other => Self::Internal(other.to_string()),
        }
    }
}

/// How a refusal of the service an incoming webhook acts through is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BadRequest,
    Forbidden,
    NotFound,
    Conflict,
}

/// Why an incoming webhook's request was refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected, and its message is safe to send back: it never holds the URL's token, a
/// secret or what the body held.
#[derive(Debug, thiserror::Error)]
pub enum IncomingError {
    #[error("no such webhook")]
    UnknownHook,
    #[error("too many requests: try again in {}", duration::format(*retry_after))]
    RateLimited { retry_after: Duration },
    #[error("this webhook is turned off")]
    Disabled,
    #[error(
        "this webhook's integration is suspended while its owner is out of the workspace, a \
         guest there or disabled"
    )]
    Suspended,
    #[error("this webhook's integration was deleted")]
    Deleted,
    #[error("the body is JSON: send it with Content-Type: application/json")]
    NotJson,
    #[error("the body is larger than {} KiB", super::MAX_INCOMING_BODY / 1024)]
    TooLarge,
    #[error("the body could not be read to its end")]
    Unreadable,
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error("{0}")]
    InvalidPayload(String),
    /// What the target's service refused, with its own code.
    #[error("{message}")]
    Refused {
        refusal: Refusal,
        code: &'static str,
        message: String,
    },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl IncomingError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownHook => "not_found",
            Self::RateLimited { .. } => "rate_limited",
            Self::Disabled => "hook_disabled",
            Self::Suspended => "integration_suspended",
            Self::Deleted => "integration_deleted",
            Self::NotJson => "unsupported_media_type",
            Self::TooLarge => "payload_too_large",
            Self::Unreadable => "invalid_body",
            Self::Signature(_) => "invalid_signature",
            Self::InvalidPayload(_) => "invalid_payload",
            Self::Refused { code, .. } => code,
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// The HTTP status the sender gets.
    pub fn status(&self) -> u16 {
        match self {
            Self::UnknownHook => 404,
            Self::RateLimited { .. } => 429,
            Self::Disabled | Self::Suspended | Self::Deleted => 403,
            Self::NotJson => 415,
            Self::TooLarge => 413,
            Self::Signature(_) => 401,
            Self::InvalidPayload(_) | Self::Unreadable => 400,
            Self::Refused { refusal, .. } => match refusal {
                Refusal::BadRequest => 400,
                Refusal::Forbidden => 403,
                Refusal::NotFound => 404,
                Refusal::Conflict => 409,
            },
            Self::Database(_) | Self::Internal(_) => 500,
        }
    }

    /// Part of normal operation: the sender's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }
}

impl From<DatabaseError> for IncomingError {
    fn from(error: DatabaseError) -> Self {
        let code = error.code();
        let refusal = match &error {
            DatabaseError::Database(_) | DatabaseError::Internal(_) => None,
            DatabaseError::Page(page) => match page {
                PageError::Database(_) | PageError::Internal(_) => None,
                PageError::NotFound(_) => Some(Refusal::NotFound),
                PageError::Forbidden(_) => Some(Refusal::Forbidden),
                PageError::InvalidInput(_) => Some(Refusal::BadRequest),
                _ => Some(Refusal::Conflict),
            },
            DatabaseError::NotFound(_) => Some(Refusal::NotFound),
            DatabaseError::Forbidden(_) => Some(Refusal::Forbidden),
            DatabaseError::NameTaken(_)
            | DatabaseError::LossyChange { .. }
            | DatabaseError::TooMany(_) => Some(Refusal::Conflict),
            _ => Some(Refusal::BadRequest),
        };
        match (refusal, error) {
            (Some(refusal), error) => Self::Refused {
                refusal,
                code,
                message: error.to_string(),
            },
            (
                None,
                DatabaseError::Database(error) | DatabaseError::Page(PageError::Database(error)),
            ) => Self::Database(error),
            (None, other) => Self::Internal(other.to_string()),
        }
    }
}

impl From<PageError> for IncomingError {
    fn from(error: PageError) -> Self {
        DatabaseError::Page(error).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(WebhookError::AddressRefused("loopback".into()).is_expected());
        assert!(WebhookError::NoSecretKey.is_expected());
        assert!(!WebhookError::Internal("sealing".into()).is_expected());
        assert_eq!(
            WebhookError::Database(sqlx::Error::PoolTimedOut).code(),
            "internal"
        );
        assert_eq!(
            WebhookError::denied(Denied::Hidden, "no such webhook").to_string(),
            "no such webhook"
        );
        assert_eq!(
            WebhookError::denied(Denied::Forbidden("no"), "unused").code(),
            "forbidden"
        );
    }

    #[test]
    fn refusals_of_the_target_keep_their_codes() {
        let missing = IncomingError::from(DatabaseError::NotFound("no such row"));
        assert_eq!((missing.status(), missing.code()), (404, "not_found"));
        let view_only = IncomingError::from(PageError::Forbidden("view only"));
        assert_eq!((view_only.status(), view_only.code()), (403, "forbidden"));
        let invalid = IncomingError::from(DatabaseError::InvalidInput("no such property".into()));
        assert_eq!((invalid.status(), invalid.code()), (400, "invalid_input"));
        let full = IncomingError::from(PageError::TooManyRows);
        assert_eq!((full.status(), full.code()), (409, "too_many_rows"));
        let broken = IncomingError::from(DatabaseError::Internal("oops".into()));
        assert!(!broken.is_expected());
        assert_eq!(broken.status(), 500);
        assert_eq!(
            (
                IncomingError::Signature(SignatureError::Stale).status(),
                "invalid_signature"
            ),
            (401, IncomingError::Signature(SignatureError::Stale).code())
        );
    }
}
