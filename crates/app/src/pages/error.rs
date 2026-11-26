use neon_domain::pages::InvalidPosition;
use neon_domain::workspaces::Denied;

/// Why a page use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them.
#[derive(Debug, thiserror::Error)]
pub enum PageError {
    #[error("{0}")]
    InvalidInput(String),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("a page cannot move under itself or one of its subpages")]
    Cycle,
    #[error("pages nest at most {} levels deep", neon_domain::pages::MAX_DEPTH)]
    TooDeep,
    #[error("the page is shared with this person already: change that share instead")]
    AlreadyShared,
    #[error("the page is among your favorites already")]
    AlreadyFavorite,
    #[error(
        "a database holds at most {} rows out of the trash",
        neon_domain::databases::MAX_ROWS
    )]
    TooManyRows,
    #[error("this workspace has reached its storage limit of {limit} bytes")]
    StorageLimit { limit: u64 },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl PageError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::Cycle => "cycle",
            Self::TooDeep => "too_deep",
            Self::AlreadyShared => "already_shared",
            Self::AlreadyFavorite => "already_favorite",
            Self::TooManyRows => "too_many_rows",
            Self::StorageLimit { .. } => "storage_limit",
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        !matches!(self, Self::Database(_) | Self::Internal(_))
    }

    pub(crate) fn invalid(error: impl std::fmt::Display) -> Self {
        Self::InvalidInput(error.to_string())
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

impl From<InvalidPosition> for PageError {
    /// Positions come from keys the database holds, so one that cannot be ordered is the
    /// server's problem.
    fn from(error: InvalidPosition) -> Self {
        Self::Internal(format!("page positions: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(PageError::Cycle.is_expected());
        assert!(PageError::AlreadyShared.is_expected());
        assert!(!PageError::Internal("broken".into()).is_expected());
        assert!(!PageError::Database(sqlx::Error::PoolTimedOut).is_expected());
        assert_eq!(
            PageError::from(InvalidPosition::Exhausted("z".into())).code(),
            "internal"
        );
    }

    #[test]
    fn what_is_hidden_is_not_found() {
        let hidden = PageError::denied(Denied::Hidden, "no such page");
        let forbidden = PageError::denied(Denied::Forbidden("view only"), "unused");

        assert_eq!(hidden.code(), "not_found");
        assert_eq!(hidden.to_string(), "no such page");
        assert_eq!(forbidden.code(), "forbidden");
        assert_eq!(forbidden.to_string(), "view only");
        assert_eq!(
            PageError::TooDeep.to_string(),
            "pages nest at most 100 levels deep"
        );
    }
}
