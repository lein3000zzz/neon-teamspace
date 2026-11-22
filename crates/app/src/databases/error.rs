use neon_domain::databases::filter::InvalidFilter;
use neon_domain::databases::{InvalidUniqueId, InvalidValue, InvalidView, SchemaError};

use crate::pages::PageError;

/// Why a database use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database), [`Internal`](Self::Internal) and the
/// failures of [`Page`](Self::Page) is expected: it is the caller's to fix, and its message
/// is safe to show them. No message holds a property's value.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("{0}")]
    InvalidInput(String),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    /// Another property, or option, has the name.
    #[error("{0}")]
    NameTaken(String),
    /// A change of a property's type that loses values, asked without confirming it.
    #[error(
        "{lost} rows would lose their value of this property: send confirm: true to change it \
         anyway"
    )]
    LossyChange { lost: usize },
    /// Past one of the bounds of a database: its rows, properties or views.
    #[error("{0}")]
    TooMany(String),
    /// A refusal or failure of the pages the database and its rows are.
    #[error(transparent)]
    Page(#[from] PageError),
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl DatabaseError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::NameTaken(_) => "name_taken",
            Self::LossyChange { .. } => "lossy_change",
            Self::TooMany(_) => "limit_reached",
            Self::Page(error) => error.code(),
            Self::Database(_) | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        match self {
            Self::Page(error) => error.is_expected(),
            Self::Database(_) | Self::Internal(_) => false,
            _ => true,
        }
    }
}

impl From<SchemaError> for DatabaseError {
    fn from(error: SchemaError) -> Self {
        match error {
            SchemaError::Invalid(message) => Self::InvalidInput(message),
            SchemaError::NameTaken(message) => Self::NameTaken(message),
            SchemaError::TooMany(message) => Self::TooMany(message),
            SchemaError::NoProperty => Self::NotFound(super::NO_PROPERTY),
        }
    }
}

impl From<InvalidValue> for DatabaseError {
    fn from(error: InvalidValue) -> Self {
        Self::InvalidInput(error.0)
    }
}

impl From<InvalidView> for DatabaseError {
    fn from(error: InvalidView) -> Self {
        Self::InvalidInput(error.0)
    }
}

impl From<InvalidFilter> for DatabaseError {
    fn from(error: InvalidFilter) -> Self {
        Self::InvalidInput(error.0)
    }
}

impl From<InvalidUniqueId> for DatabaseError {
    fn from(error: InvalidUniqueId) -> Self {
        Self::InvalidInput(error.0)
    }
}

impl From<neon_domain::pages::InvalidPosition> for DatabaseError {
    /// Positions come from keys the database holds, so one that cannot be ordered is the
    /// server's problem.
    fn from(error: neon_domain::pages::InvalidPosition) -> Self {
        Self::Internal(format!("positions: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(DatabaseError::LossyChange { lost: 3 }.is_expected());
        assert!(DatabaseError::Page(PageError::TooManyRows).is_expected());
        assert!(!DatabaseError::Page(PageError::Internal("broken".into())).is_expected());
        assert!(!DatabaseError::Internal("broken".into()).is_expected());
        assert!(!DatabaseError::Database(sqlx::Error::PoolTimedOut).is_expected());
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(
            DatabaseError::LossyChange { lost: 3 }.code(),
            "lossy_change"
        );
        assert_eq!(
            DatabaseError::LossyChange { lost: 3 }.to_string(),
            "3 rows would lose their value of this property: send confirm: true to change it \
             anyway"
        );
        assert_eq!(
            DatabaseError::Page(PageError::TooManyRows).code(),
            "too_many_rows"
        );
        assert_eq!(
            DatabaseError::from(SchemaError::NameTaken("taken".into())).code(),
            "name_taken"
        );
        assert_eq!(
            DatabaseError::from(SchemaError::NoProperty).code(),
            "not_found"
        );
        assert_eq!(
            DatabaseError::TooMany("views".into()).code(),
            "limit_reached"
        );
    }
}
