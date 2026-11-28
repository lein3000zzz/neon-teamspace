use crate::pages::PageError;
use crate::sync::ContentError;

/// Why a history use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database), [`Internal`](Self::Internal) and the
/// failures of the page and content errors is expected: it is the caller's to fix, and its
/// message is safe to show them. No message quotes a page.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error(transparent)]
    Page(#[from] PageError),
    #[error(transparent)]
    Content(#[from] ContentError),
    /// A version that does not exist, or not of that page.
    #[error("no such version of this page")]
    NoVersion,
    #[error("{0}")]
    InvalidInput(String),
    #[error("database: {0}")]
    Database(sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

from_cause!(HistoryError { Database(sqlx::Error) });

impl HistoryError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Page(error) => error.code(),
            Self::Content(error) => error.code(),
            Self::NoVersion => "not_found",
            Self::InvalidInput(_) => "invalid_input",
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        match self {
            Self::Page(error) => error.is_expected(),
            Self::Content(error) => error.is_expected(),
            Self::NoVersion | Self::InvalidInput(_) => true,
            Self::Database(_) | Self::Internal(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_the_caller_s_and_failures_the_server_s() {
        assert_eq!(HistoryError::NoVersion.code(), "not_found");
        assert!(HistoryError::NoVersion.is_expected());
        assert_eq!(
            HistoryError::from(ContentError::Conflict).code(),
            "edit_conflict"
        );
        assert!(HistoryError::from(ContentError::Conflict).is_expected());
        assert!(!HistoryError::Internal("broken".into()).is_expected());
        assert!(!HistoryError::Database(sqlx::Error::PoolTimedOut).is_expected());
        assert_eq!(
            HistoryError::from(PageError::NotFound("no such page")).code(),
            "not_found"
        );
    }
}
