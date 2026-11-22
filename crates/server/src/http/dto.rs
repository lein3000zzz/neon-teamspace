//! The shapes the account routes share. Times are RFC 3339 strings, ids UUIDs.

use neon_domain::auth;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// What an account may do on the instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    User,
}

impl From<auth::Role> for Role {
    fn from(role: auth::Role) -> Self {
        match role {
            auth::Role::Admin => Self::Admin,
            auth::Role::User => Self::User,
        }
    }
}

impl From<Role> for auth::Role {
    fn from(role: Role) -> Self {
        match role {
            Role::Admin => Self::Admin,
            Role::User => Self::User,
        }
    }
}

/// What an API token may do: `read`, `write`, or the admin pages' work, `admin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    Write,
    Admin,
}

impl From<auth::Scope> for Scope {
    fn from(scope: auth::Scope) -> Self {
        match scope {
            auth::Scope::Read => Self::Read,
            auth::Scope::Write => Self::Write,
            auth::Scope::Admin => Self::Admin,
        }
    }
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }
}

/// An account.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct User {
    pub id: Uuid,
    #[schema(example = "ada@example.com")]
    pub email: String,
    #[schema(example = "Ada Lovelace")]
    pub display_name: String,
    pub role: Role,
    /// Unset while the address must still be verified.
    #[serde(with = "time::serde::rfc3339::option")]
    pub email_verified_at: Option<OffsetDateTime>,
    /// Set while the account is disabled.
    #[serde(with = "time::serde::rfc3339::option")]
    pub disabled_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<auth::User> for User {
    fn from(user: auth::User) -> Self {
        Self {
            id: user.id,
            email: user.email.as_str().to_owned(),
            display_name: user.display_name,
            role: user.role.into(),
            email_verified_at: user.email_verified_at,
            disabled_at: user.disabled_at,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// An invite, without its code.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Invite {
    pub id: Uuid,
    /// The address it is bound to, which alone signs up with it; absent for any address.
    pub email: Option<String>,
    pub created_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// How many sign-ups it admits, and how many it did.
    pub max_uses: i32,
    pub uses: i32,
    pub status: InviteStatus,
    #[serde(with = "time::serde::rfc3339::option")]
    pub revoked_at: Option<OffsetDateTime>,
    /// Whether its code can be copied again.
    pub code_kept: bool,
    /// Who signed up with it, oldest first.
    pub signups: Vec<InviteUse>,
}

/// Whether an invite admits sign-ups now, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InviteStatus {
    Open,
    Used,
    Expired,
    Revoked,
}

impl From<auth::InviteStatus> for InviteStatus {
    fn from(status: auth::InviteStatus) -> Self {
        match status {
            auth::InviteStatus::Open => Self::Open,
            auth::InviteStatus::UsedUp => Self::Used,
            auth::InviteStatus::Expired => Self::Expired,
            auth::InviteStatus::Revoked => Self::Revoked,
        }
    }
}

/// A sign-up with an invite.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteUse {
    /// The account's id and address, absent once it is gone.
    pub user_id: Option<Uuid>,
    pub email: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub used_at: OffsetDateTime,
}

impl Invite {
    pub fn new(invite: auth::Invite, signups: Vec<auth::InviteUse>, now: OffsetDateTime) -> Self {
        Self {
            status: invite.status(now).into(),
            id: invite.id,
            email: invite.email,
            created_by: invite.created_by,
            created_at: invite.created_at,
            expires_at: invite.expires_at,
            max_uses: invite.max_uses,
            uses: invite.uses,
            revoked_at: invite.revoked_at,
            code_kept: invite.code_kept,
            signups: signups
                .into_iter()
                .map(|signup| InviteUse {
                    user_id: signup.user_id,
                    email: signup.email,
                    used_at: signup.used_at,
                })
                .collect(),
        }
    }
}

/// A new invite with its code. The code is shown here, and again only from its sealed
/// copy when a secret key is configured.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreatedInvite {
    pub invite: Invite,
    #[schema(example = "inv_4yVd0q3oJ2...")]
    pub code: String,
    /// For an invite meant to be mailed: whether its mail was queued, with the invite. It
    /// is not while mail is not set up; the code can be handed over instead. Delivery
    /// shows in the mail status of `GET /admin/settings`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued: Option<bool>,
}

/// The body of routes that take a token from a mailed link.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    /// The token in the link.
    pub token: String,
}

/// The body of routes that take an invite code. It goes in the body, out of URLs and
/// their logs.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeRequest {
    #[schema(example = "inv_4yVd0q3oJ2...")]
    pub code: String,
}

/// An address, for the forms that answer the same whatever it is.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EmailRequest {
    #[schema(example = "ada@example.com")]
    pub email: String,
}
