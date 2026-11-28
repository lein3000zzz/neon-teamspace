use crate::mail::QueueError;
use crate::pages::PageError;

/// Why an inbox use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them.
#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("{0}")]
    InvalidInput(String),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    /// An unsubscribe link that this server did not make, or whose key is gone.
    #[error("this link does not work: open your account's notifications to change your mail")]
    InvalidToken,
    #[error("database: {0}")]
    Database(sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

from_cause!(InboxError { Database(sqlx::Error) });

impl InboxError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::InvalidToken => "invalid_token",
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }
}

impl From<PageError> for InboxError {
    fn from(error: PageError) -> Self {
        match error {
            PageError::Database(error) => Self::Database(error),
            PageError::NotFound(message) => Self::NotFound(message),
            PageError::Forbidden(message) => Self::Forbidden(message),
            other if other.is_expected() => Self::InvalidInput(other.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<QueueError> for InboxError {
    fn from(error: QueueError) -> Self {
        match error {
            QueueError::Database(error) => Self::Database(error),
            QueueError::Seal(error) => Self::Internal(format!("mail cannot be queued: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(InboxError::InvalidToken.is_expected());
        assert_eq!(InboxError::InvalidToken.code(), "invalid_token");
        assert!(!InboxError::Internal("broken".into()).is_expected());
        assert_eq!(
            InboxError::from(PageError::NotFound("no such page")).code(),
            "not_found"
        );
    }
}
