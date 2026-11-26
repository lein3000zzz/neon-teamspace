use std::time::Duration;

use neon_domain::comments::{
    InvalidAnchor, InvalidComment, MAX_PAGE_COMMENTS, MAX_REPLIES, MAX_THREADS,
};

use crate::pages::PageError;
use crate::sync::LoadError;

/// Why a comment use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them.
#[derive(Debug, thiserror::Error)]
pub enum CommentError {
    #[error("{0}")]
    InvalidInput(String),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error(
        "a page holds at most {MAX_THREADS} open threads of comments: resolve or delete some \
         first"
    )]
    TooManyThreads,
    #[error("a thread holds at most {MAX_REPLIES} comments: start another")]
    TooManyReplies,
    #[error(
        "a page holds at most {MAX_PAGE_COMMENTS} comments: those who edit it delete some first"
    )]
    TooManyComments,
    #[error("too many comments: slow down")]
    RateLimited { retry_after: Duration },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl CommentError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::TooManyThreads => "too_many_threads",
            Self::TooManyReplies => "too_many_replies",
            Self::TooManyComments => "too_many_comments",
            Self::RateLimited { .. } => "rate_limited",
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }
}

impl From<PageError> for CommentError {
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

impl From<LoadError> for CommentError {
    fn from(error: LoadError) -> Self {
        match error {
            LoadError::Database(error) => Self::Database(error),
            other => Self::Internal(format!("the page's document: {other}")),
        }
    }
}

impl From<InvalidComment> for CommentError {
    fn from(error: InvalidComment) -> Self {
        Self::InvalidInput(error.to_string())
    }
}

impl From<InvalidAnchor> for CommentError {
    fn from(error: InvalidAnchor) -> Self {
        Self::InvalidInput(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_only_failures_of_the_server_unexpected() {
        assert_eq!(CommentError::TooManyThreads.code(), "too_many_threads");
        assert_eq!(CommentError::TooManyReplies.code(), "too_many_replies");
        assert_eq!(CommentError::TooManyComments.code(), "too_many_comments");
        let limited = CommentError::RateLimited {
            retry_after: Duration::from_secs(2),
        };
        assert_eq!(limited.code(), "rate_limited");
        assert!(limited.is_expected());
        assert!(CommentError::from(InvalidComment::Empty).is_expected());
        assert!(!CommentError::Internal("broken".into()).is_expected());
        assert_eq!(
            CommentError::from(PageError::Forbidden("view only")).code(),
            "forbidden"
        );
    }
}
