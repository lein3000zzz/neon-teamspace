use std::time::Duration;

use neon_domain::agents::AgentStatus;
use neon_domain::auth::{InvalidEmail, WeakPassword};
use neon_domain::duration;

/// Why an auth use case refused, or failed.
///
/// Every variant but [`Database`](Self::Database) and [`Internal`](Self::Internal) is
/// expected: it is the caller's to fix, and its message is safe to show them. Messages
/// never hold a password, a token or an invite code.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    InvalidEmail(#[from] InvalidEmail),
    #[error(transparent)]
    WeakPassword(#[from] WeakPassword),
    #[error("invalid or expired invite")]
    InvalidInvite,
    #[error("this invite is for another address")]
    InviteForOtherEmail,
    #[error("another invite has this code")]
    InviteCodeTaken,
    #[error("{0}")]
    CodeUnavailable(String),
    #[error("an account has this address already")]
    EmailTaken,
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("not signed in")]
    Unauthenticated,
    #[error("this account is disabled")]
    UserDisabled,
    /// A token that is known, unexpired and not revoked, of an agent whose owner is out of
    /// its workspace, a guest there or disabled.
    #[error(
        "this agent is suspended while its owner is out of the workspace, a guest there or \
         disabled: its tokens work again once its owner is a member again, or once the \
         workspace's owners or admins hand it to someone else"
    )]
    AgentSuspended,
    /// As [`AgentSuspended`](Self::AgentSuspended), while agents are off.
    #[error(
        "agents are turned off in this workspace or on this instance: the agent's tokens work \
         again once agents are back on"
    )]
    AgentsOff,
    /// As [`AgentSuspended`](Self::AgentSuspended), for an agent deleted.
    #[error("this agent is deleted: its tokens never work again")]
    AgentDeleted,
    #[error("verify your email first: open the link mailed to you")]
    EmailNotVerified,
    #[error("registration is closed: ask an admin for an account")]
    RegistrationClosed,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("this link does not work any more: it was used already, or it is too old")]
    InvalidLink,
    #[error("passwords cannot be reset by email here: ask an admin")]
    PasswordResetOff,
    #[error("invites cannot be mailed here: inviting by email is off, or mail is not set up")]
    InviteEmailOff,
    #[error("requests to join are off here")]
    RequestsOff,
    /// With what is missing, for admins; the forms anyone can send say only that.
    #[error("mail is not set up{}", .0.as_deref().map(|why| format!(": {why}")).unwrap_or_default())]
    MailNotSetUp(Option<String>),
    #[error("{0}")]
    NotFound(&'static str),
    #[error("the last active admin cannot be demoted or disabled")]
    LastAdmin,
    #[error("too many attempts: try again in {}", duration::format(*retry_after))]
    RateLimited { retry_after: Duration },
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl AuthError {
    /// A stable code for programs, as the API's error shape carries it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidEmail(_) => "invalid_email",
            Self::WeakPassword(_) => "weak_password",
            Self::InvalidInvite | Self::InviteForOtherEmail => "invalid_invite",
            Self::InviteCodeTaken => "invite_code_taken",
            Self::CodeUnavailable(_) => "code_unavailable",
            Self::EmailTaken => "email_taken",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Unauthenticated => "unauthenticated",
            Self::UserDisabled => "user_disabled",
            Self::AgentSuspended => "agent_suspended",
            Self::AgentsOff => "agents_off",
            Self::AgentDeleted => "agent_deleted",
            Self::EmailNotVerified => "email_not_verified",
            Self::RegistrationClosed => "registration_closed",
            Self::Forbidden(_) => "forbidden",
            Self::InvalidLink => "invalid_link",
            Self::PasswordResetOff => "password_reset_off",
            Self::InviteEmailOff => "invite_email_off",
            Self::RequestsOff => "requests_off",
            Self::MailNotSetUp(_) => "mail_not_set_up",
            Self::NotFound(_) => "not_found",
            Self::LastAdmin => "last_admin",
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

    /// The refusal of a token whose agent has `status`, or `None` while the agent works.
    pub(crate) fn agent_stopped(status: AgentStatus) -> Option<Self> {
        match status {
            AgentStatus::Active => None,
            AgentStatus::Suspended => Some(Self::AgentSuspended),
            AgentStatus::Disabled => Some(Self::AgentsOff),
            AgentStatus::Deleted => Some(Self::AgentDeleted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_of_the_server_are_unexpected() {
        assert!(AuthError::InvalidCredentials.is_expected());
        assert!(
            AuthError::RateLimited {
                retry_after: Duration::from_secs(1)
            }
            .is_expected()
        );
        assert!(!AuthError::Internal("hashing".into()).is_expected());
        assert!(!AuthError::Database(sqlx::Error::PoolTimedOut).is_expected());
        assert_eq!(
            AuthError::Database(sqlx::Error::PoolTimedOut).code(),
            "internal"
        );
    }

    #[test]
    fn mail_not_set_up_says_why_when_told() {
        assert_eq!(
            AuthError::MailNotSetUp(None).to_string(),
            "mail is not set up"
        );
        assert_eq!(
            AuthError::MailNotSetUp(Some("mail.host is not set".into())).to_string(),
            "mail is not set up: mail.host is not set"
        );
    }

    #[test]
    fn rate_limits_say_when_to_try_again() {
        let error = AuthError::RateLimited {
            retry_after: Duration::from_secs(30),
        };

        assert_eq!(error.to_string(), "too many attempts: try again in 30s");
        assert_eq!(error.code(), "rate_limited");
    }
}
