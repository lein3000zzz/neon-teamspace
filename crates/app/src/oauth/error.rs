use std::time::Duration;

use neon_domain::duration;
use neon_domain::oauth::{ErrorCode, InvalidChoice, Problem, RequestProblem};
use neon_domain::workspaces::Denied;

/// Why an OAuth use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them. Messages
/// never hold a code, a token or a verifier.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// The authorization request does not work. The consent page shows it; it is never
    /// sent to the client's redirect URI on its own.
    #[error("{}", .0.description())]
    Request(RequestProblem),
    /// An error of the token, registration and revocation endpoints, as OAuth has them.
    #[error(transparent)]
    Protocol(Problem),
    /// The scopes a person chose on the consent page.
    #[error(transparent)]
    Scopes(#[from] InvalidChoice),
    /// What the client would act as is not on offer: switched off, or not the person's.
    #[error("{0}")]
    IdentityUnavailable(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("clients do not register themselves on this server")]
    RegistrationOff,
    #[error("too many clients are registered that nobody connected: try again later")]
    ClientLimit,
    /// One source's share of the clients nobody connected is full; others may still
    /// register.
    #[error(
        "too many clients registered from this address wait for someone to connect them: \
         connect one, or try again later"
    )]
    SourceLimit,
    #[error("too many attempts: try again in {}", duration::format(*retry_after))]
    RateLimited { retry_after: Duration },
    #[error("database: {0}")]
    Database(sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

from_cause!(OAuthError { Database(sqlx::Error) });

impl OAuthError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Request(problem) => problem.as_str(),
            Self::Protocol(problem) => problem.code.as_str(),
            Self::Scopes(_) => ErrorCode::InvalidScope.as_str(),
            Self::IdentityUnavailable(_) => "identity_unavailable",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::RegistrationOff => "registration_off",
            Self::ClientLimit => ErrorCode::TemporarilyUnavailable.as_str(),
            Self::SourceLimit => "too_many_clients",
            Self::RateLimited { .. } => "rate_limited",
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }

    pub(super) fn protocol(code: ErrorCode, description: impl Into<String>) -> Self {
        Self::Protocol(Problem::new(code, description))
    }

    pub(super) fn denied(denied: Denied, missing: &'static str) -> Self {
        match denied {
            Denied::Hidden => Self::NotFound(missing),
            Denied::Forbidden(reason) => Self::Forbidden(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(OAuthError::Request(RequestProblem::PkceRequired).is_expected());
        assert!(OAuthError::protocol(ErrorCode::InvalidGrant, "used").is_expected());
        assert!(OAuthError::ClientLimit.is_expected());
        assert!(!OAuthError::Internal("broken".into()).is_expected());
        assert!(!OAuthError::Database(sqlx::Error::PoolTimedOut).is_expected());
    }

    #[test]
    fn codes_are_the_oauth_ones_where_oauth_has_one() {
        assert_eq!(
            OAuthError::Request(RequestProblem::RedirectUriMismatch).code(),
            "redirect_uri_mismatch"
        );
        assert_eq!(
            OAuthError::protocol(ErrorCode::InvalidGrant, "used").code(),
            "invalid_grant"
        );
        assert_eq!(
            OAuthError::Scopes(InvalidChoice::None).code(),
            "invalid_scope"
        );
        assert_eq!(OAuthError::ClientLimit.code(), "temporarily_unavailable");
        assert_eq!(OAuthError::SourceLimit.code(), "too_many_clients");
        assert_eq!(OAuthError::Internal("x".into()).code(), "internal");
    }
}
