use std::time::Duration;

use neon_domain::duration;

use crate::pages::PageError;
use crate::sync::ContentError;

/// Why an import or export refused, or failed.
///
/// Every variant but [`Content`](Self::Content), [`Internal`](Self::Internal) and the
/// failures of [`Page`](Self::Page) is expected: it is the caller's to fix, and its message
/// is safe to show them. Messages name files of the caller's upload, never anything of a
/// page they may not see.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{0}")]
    InvalidInput(String),
    /// Past one of the [`Limits`](super::Limits).
    #[error("{0}")]
    TooLarge(String),
    #[error("it took longer than {}: import or export less at once", duration::format(*.0))]
    TimedOut(Duration),
    #[error("too many imports and exports are running: try again in a moment")]
    Busy,
    #[error("too many imports or exports: try again in {}", duration::format(*retry_after))]
    RateLimited { retry_after: Duration },
    #[error(transparent)]
    Page(#[from] PageError),
    /// A page's document could not be read.
    #[error("page {page_id}: {error}")]
    Content {
        page_id: uuid::Uuid,
        error: ContentError,
    },
    #[error("{0}")]
    Internal(String),
}

impl TransferError {
    /// How long a refusal for being busy asks the caller to wait.
    pub const BUSY_RETRY: Duration = Duration::from_secs(5);

    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::TooLarge(_) => "too_large",
            Self::TimedOut(_) => "timeout",
            Self::Busy => "busy",
            Self::RateLimited { .. } => "rate_limited",
            Self::Page(error) => error.code(),
            Self::Content { .. } | Self::Internal(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, or to try again later, not the
    /// server's.
    pub fn is_expected(&self) -> bool {
        match self {
            Self::Page(error) => error.is_expected(),
            Self::Content { .. } | Self::Internal(_) => false,
            _ => true,
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    pub(crate) fn too_large(message: impl Into<String>) -> Self {
        Self::TooLarge(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(TransferError::Busy.is_expected());
        assert!(TransferError::TimedOut(Duration::from_secs(20)).is_expected());
        assert!(TransferError::Page(PageError::TooDeep).is_expected());
        assert!(!TransferError::Page(PageError::Internal("broken".into())).is_expected());
        assert!(!TransferError::Internal("broken".into()).is_expected());
        assert_eq!(TransferError::Page(PageError::TooDeep).code(), "too_deep");
        assert_eq!(
            TransferError::TimedOut(Duration::from_secs(20)).to_string(),
            "it took longer than 20s: import or export less at once"
        );
    }
}
