use neon_domain::workspaces::Denied;

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
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }

    /// A refusal of the rules. What the caller may not know of is `missing`, as if it did
    /// not exist.
    pub(crate) fn denied(denied: Denied, missing: &'static str) -> Self {
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
}
