use std::time::Duration;

use neon_domain::duration;
use neon_domain::workspaces::Denied;
use neon_domain::workspaces::permissions::{LastOwner, LastTeamspaceOwner};

/// Why a workspace use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them. Messages
/// never hold an invite link's token.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    InvalidInput(String),
    /// Something that does not exist, or that the caller may not know of.
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("no account here has this address: invitations only go to existing accounts")]
    NoAccount,
    #[error("this account is in the workspace already")]
    AlreadyMember,
    #[error("this account has a pending invitation to the workspace already")]
    AlreadyInvited,
    #[error("this workspace has reached its limit of {limit} members")]
    MemberLimit { limit: u64 },
    #[error(transparent)]
    LastOwner(#[from] LastOwner),
    #[error(transparent)]
    LastTeamspaceOwner(#[from] LastTeamspaceOwner),
    #[error("another workspace has this slug")]
    SlugTaken,
    #[error("the name typed does not match the workspace's name")]
    ConfirmationMismatch,
    #[error("invalid or expired invite link")]
    InvalidInviteLink,
    #[error("{0}")]
    TokenUnavailable(String),
    #[error("too many attempts: try again in {}", duration::format(*retry_after))]
    RateLimited { retry_after: Duration },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl WorkspaceError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Forbidden(_) => "forbidden",
            Self::NoAccount => "no_account",
            Self::AlreadyMember => "already_member",
            Self::AlreadyInvited => "already_invited",
            Self::MemberLimit { .. } => "member_limit",
            Self::LastOwner(_) => "last_owner",
            Self::LastTeamspaceOwner(_) => "last_teamspace_owner",
            Self::SlugTaken => "slug_taken",
            Self::ConfirmationMismatch => "confirmation_mismatch",
            Self::InvalidInviteLink => "invalid_invite_link",
            Self::TokenUnavailable(_) => "token_unavailable",
            Self::RateLimited { .. } => "rate_limited",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(WorkspaceError::NoAccount.is_expected());
        assert!(WorkspaceError::MemberLimit { limit: 3 }.is_expected());
        assert!(!WorkspaceError::Internal("sealing".into()).is_expected());
        assert!(!WorkspaceError::Database(sqlx::Error::PoolTimedOut).is_expected());
        assert_eq!(
            WorkspaceError::Database(sqlx::Error::PoolTimedOut).code(),
            "internal"
        );
    }

    #[test]
    fn what_is_hidden_is_not_found() {
        let hidden = WorkspaceError::denied(Denied::Hidden, "no such teamspace");
        let forbidden = WorkspaceError::denied(Denied::Forbidden("owners only"), "unused");

        assert_eq!(hidden.code(), "not_found");
        assert_eq!(hidden.to_string(), "no such teamspace");
        assert_eq!(forbidden.code(), "forbidden");
        assert_eq!(forbidden.to_string(), "owners only");
    }

    #[test]
    fn limits_and_owners_say_what_stops_them() {
        assert_eq!(
            WorkspaceError::MemberLimit { limit: 5 }.to_string(),
            "this workspace has reached its limit of 5 members"
        );
        assert_eq!(WorkspaceError::from(LastOwner).code(), "last_owner");
        assert_eq!(
            WorkspaceError::from(LastTeamspaceOwner).code(),
            "last_teamspace_owner"
        );
    }
}
