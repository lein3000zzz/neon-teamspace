use std::time::Duration;

use neon_domain::duration;
use neon_domain::search::QueryError;

use crate::pages::PageError;

/// Why a search refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and the failures of [`Page`](Self::Page)
/// is expected: it is the caller's to fix, and its message is safe to show them. No message
/// quotes the query.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error(transparent)]
    Page(#[from] PageError),
    /// A query that cannot be run.
    #[error(transparent)]
    Query(QueryError),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("too many searches: try again in {}", duration::format(*retry_after))]
    RateLimited { retry_after: Duration },
    #[error("too many searches are running: try again in a moment")]
    Busy,
    #[error("the search took longer than {}: try more words or filters", duration::format(*.0))]
    TimedOut(Duration),
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

impl SearchError {
    /// When to try again after [`Busy`](Self::Busy).
    pub const BUSY_RETRY: Duration = Duration::from_secs(1);

    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Page(error) => error.code(),
            Self::Query(_) => "invalid_search",
            Self::InvalidInput(_) => "invalid_input",
            Self::Forbidden(_) => "forbidden",
            Self::RateLimited { .. } => "rate_limited",
            Self::Busy => "busy",
            Self::TimedOut(_) => "timeout",
            Self::Database(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        match self {
            Self::Page(error) => error.is_expected(),
            Self::Database(_) => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_the_caller_s_and_failures_the_server_s() {
        let refused = SearchError::Query(QueryError::TooLong);
        assert_eq!(refused.code(), "invalid_search");
        assert!(refused.is_expected());
        assert_eq!(refused.to_string(), "a search is at most 200 characters");
        assert!(!SearchError::Database(sqlx::Error::PoolTimedOut).is_expected());
        // What the server holds back is the caller's to wait out, not a failure.
        for held in [
            SearchError::RateLimited {
                retry_after: Duration::from_secs(3),
            },
            SearchError::Busy,
            SearchError::TimedOut(Duration::from_secs(5)),
        ] {
            assert!(held.is_expected(), "{held}");
        }
        assert_eq!(
            SearchError::TimedOut(Duration::from_secs(5)).to_string(),
            "the search took longer than 5s: try more words or filters"
        );
        assert_eq!(
            SearchError::Query(QueryError::ControlCharacter).code(),
            "invalid_search"
        );
        assert_eq!(
            SearchError::from(PageError::NotFound("no such workspace")).code(),
            "not_found"
        );
    }
}
