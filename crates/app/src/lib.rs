//! Use cases.
//!
//! Use cases call the repositories directly and own their transactions. Traits exist only
//! for side effects that tests replace: mail, blob storage, realtime broadcast, rate limits.

/// Implements `From` for the variants of an error that wrap their cause and show it in their
/// own message, such as `#[error("database: {0}")] Database(sqlx::Error)`, so that `?`
/// converts the cause. thiserror's `#[from]` would make the cause the error's `source` too,
/// and every report that walks the sources, such as a span's exception, would show it twice.
macro_rules! from_cause {
    ($error:ident { $($variant:ident($cause:ty)),+ $(,)? }) => {
        $(
            impl From<$cause> for $error {
                fn from(cause: $cause) -> Self {
                    Self::$variant(cause)
                }
            }
        )+
    };
}

pub mod attachments;
pub mod auth;
pub mod clock;
pub mod comments;
pub mod crypto;
pub mod databases;
pub mod history;
pub mod jobs;
pub mod mail;
pub mod notifications;
pub mod oauth;
pub mod pages;
pub mod periodic;
pub mod ratelimit;
pub mod search;
pub mod settings;
mod standing;
pub mod sync;
pub mod transfer;
pub mod webhooks;
pub mod workspaces;

/// How a use case reads the row it is about to change: as it stands, or locked until the
/// transaction ends, so that the changes to it run one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    Plain,
    Locked,
}

#[cfg(test)]
mod tests {
    use crate::jobs::JobError;
    use crate::sync::{ContentError, LoadError};

    #[test]
    fn a_cause_in_the_message_is_not_a_source_too() {
        let error = ContentError::from(LoadError::from(sqlx::Error::PoolClosed));

        assert_eq!(
            JobError::from(error).message(),
            format!(
                "the page's document does not load: database: {}",
                sqlx::Error::PoolClosed
            ),
        );
    }
}
