use std::time::Duration;

use neon_domain::attachments::{FileKind, MediaType};

use super::blobs::BlobError;
use crate::pages::PageError;

/// Why an attachment use case refused, or failed.
///
/// Every variant but [`Store`](Self::Store) and [`Database`](Self::Database) is expected: it
/// is the caller's to fix, and its message is safe to show them. None names a file.
#[derive(Debug, thiserror::Error)]
pub enum AttachmentError {
    /// The page's rules refused, or it does not exist for the caller.
    #[error(transparent)]
    Page(#[from] PageError),
    /// No such file, or none the caller may see.
    #[error("no such file")]
    NotFound,
    /// The file's bytes are not of a type its block takes.
    #[error("{} files take {}; this one is {found}", kind.as_str(), kind.allowed())]
    UnsupportedType { kind: FileKind, found: MediaType },
    /// The file is larger than one upload takes.
    #[error("a file is at most {max} bytes")]
    TooLarge { max: u64 },
    /// The workspace has no room for the file.
    #[error("this workspace has reached its storage limit of {limit} bytes")]
    StorageLimit { limit: u64 },
    #[error("too many uploads: slow down")]
    RateLimited { retry_after: Duration },
    /// As many uploads run, the caller's or the server's, as may at once.
    #[error("too many uploads are running: try again once one ends")]
    Busy,
    /// The upload stopped before its end, such as when its client went away.
    #[error("the upload could not be read: {0}")]
    Body(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Store(#[from] BlobError),
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

impl AttachmentError {
    /// When to try again after [`Busy`](Self::Busy).
    pub const BUSY_RETRY: Duration = Duration::from_secs(2);

    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Page(error) => error.code(),
            Self::NotFound => "not_found",
            Self::UnsupportedType { .. } => "unsupported_type",
            Self::TooLarge { .. } => "file_too_large",
            Self::StorageLimit { .. } => "storage_limit",
            Self::RateLimited { .. } => "rate_limited",
            Self::Busy => "busy",
            Self::Body(_) => "invalid_body",
            Self::InvalidInput(_) => "invalid_input",
            Self::Store(BlobError::NotFound) => "file_missing",
            Self::Store(_) => "storage_unavailable",
            Self::Database(_) => "internal",
        }
    }

    /// Part of normal operation: the caller's to fix, not the server's.
    pub fn is_expected(&self) -> bool {
        match self {
            Self::Page(error) => error.is_expected(),
            Self::Store(_) | Self::Database(_) => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_the_caller_s_and_failures_the_server_s() {
        let refused = AttachmentError::UnsupportedType {
            kind: FileKind::Image,
            found: MediaType::Html,
        };
        assert!(refused.is_expected());
        assert_eq!(refused.code(), "unsupported_type");
        assert_eq!(
            refused.to_string(),
            "image files take PNG, JPEG, GIF, WebP, AVIF or SVG; this one is text/html"
        );
        assert_eq!(
            AttachmentError::StorageLimit { limit: 10 }.to_string(),
            "this workspace has reached its storage limit of 10 bytes"
        );
        assert!(AttachmentError::TooLarge { max: 1 }.is_expected());
        assert!(AttachmentError::NotFound.is_expected());
        assert!(!AttachmentError::Store(BlobError::Store("down".into())).is_expected());
        assert_eq!(
            AttachmentError::Store(BlobError::NotFound).code(),
            "file_missing"
        );
        assert!(!AttachmentError::Database(sqlx::Error::PoolTimedOut).is_expected());
        assert!(AttachmentError::Page(PageError::NotFound("no such page")).is_expected());
    }
}
