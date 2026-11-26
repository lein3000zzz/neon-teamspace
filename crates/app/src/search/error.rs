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
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

impl SearchError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Page(error) => error.code(),
            Self::Query(_) => "invalid_search",
            Self::InvalidInput(_) => "invalid_input",
            Self::Forbidden(_) => "forbidden",
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
        assert_eq!(
            SearchError::from(PageError::NotFound("no such workspace")).code(),
            "not_found"
        );
    }
}
