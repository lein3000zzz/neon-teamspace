//! API tokens: programmatic access as their owner, limited to the scopes they were made
//! with, and never beyond what the owner's role allows.
//!
//! A person's token works across the instance, or only in one workspace it is bound to. An
//! agent's token is always bound to the agent's workspace, and acts as the agent.

use std::fmt;

use time::OffsetDateTime;
use uuid::Uuid;

use super::user::Role;

/// Starts every API token, so a bearer token is told from other secrets at a glance.
pub const API_TOKEN_PREFIX: &str = "nw_";

/// The characters of a token kept, besides its hash, to recognise it in lists.
pub const SHOWN_PREFIX_LENGTH: usize = 10;

/// The longest token name, in characters.
pub const MAX_TOKEN_NAME: usize = 64;

/// What a token may do. A browser session may do everything its user may.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// Read what the owner can see. Every scope brings it.
    Read,
    /// Change what the owner can change.
    Write,
    /// The instance admin pages' work, for an instance admin's token across the instance;
    /// or a workspace's admin work, for an owner's or admin's token bound to it. It stops
    /// working when its owner stops being one. An agent's token never has it.
    Admin,
}

impl Scope {
    /// Every scope, in the order lists show them.
    pub const ALL: [Self; 3] = [Self::Read, Self::Write, Self::Admin];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.as_str() == text)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whom a new token acts for, as its scopes need to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenHolder {
    /// A person, across the instance, whose instance role is `role`.
    Account { role: Role },
    /// A person, in one workspace only, which they manage or not: own or administer.
    Workspace { manages: bool },
    /// An agent, in its workspace.
    Agent,
}

/// Scopes a new token cannot have.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidScopes {
    #[error("a token needs at least one scope")]
    None,
    #[error("only an admin's token may have the admin scope")]
    AdminOnly,
    #[error("a token bound to a workspace has the admin scope only for its owners and admins")]
    WorkspaceAdminOnly,
    #[error("an agent's token reads and writes, and never has the admin scope")]
    AgentAdmin,
}

impl InvalidScopes {
    /// Why the holder may not have what they asked for, when it is the admin scope they
    /// may not have, rather than no scope at all.
    pub fn refusal(&self) -> Option<&'static str> {
        match self {
            Self::AdminOnly => Some("only an admin's token may have the admin scope"),
            Self::WorkspaceAdminOnly => Some(
                "a token bound to a workspace has the admin scope only for its owners and \
                 admins",
            ),
            Self::AgentAdmin => {
                Some("an agent's token reads and writes, and never has the admin scope")
            }
            Self::None => None,
        }
    }
}

/// The scopes a new token of `holder` gets: each once, in [`Scope::ALL`] order. At least
/// one, and admin only for an instance admin's token across the instance, or a
/// workspace owner's or admin's token bound to it.
pub fn token_scopes(requested: &[Scope], holder: TokenHolder) -> Result<Vec<Scope>, InvalidScopes> {
    if requested.is_empty() {
        return Err(InvalidScopes::None);
    }
    if requested.contains(&Scope::Admin) {
        match holder {
            TokenHolder::Account { role: Role::Admin }
            | TokenHolder::Workspace { manages: true } => {}
            TokenHolder::Account { .. } => return Err(InvalidScopes::AdminOnly),
            TokenHolder::Workspace { manages: false } => {
                return Err(InvalidScopes::WorkspaceAdminOnly);
            }
            TokenHolder::Agent => return Err(InvalidScopes::AgentAdmin),
        }
    }
    let mut scopes = requested.to_vec();
    scopes.sort();
    scopes.dedup();
    Ok(scopes)
}

/// What a token's scopes allow now: an owner who is no longer an admin takes the admin
/// scope away from their tokens.
pub fn effective_scopes(scopes: &[Scope], role: Role) -> Vec<Scope> {
    scopes
        .iter()
        .copied()
        .filter(|scope| *scope != Scope::Admin || role == Role::Admin)
        .collect()
}

/// A token name that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a token's name has 1 to {MAX_TOKEN_NAME} characters, and no control characters")]
pub struct InvalidTokenName;

/// The name to keep, trimmed.
pub fn check_token_name(raw: &str) -> Result<String, InvalidTokenName> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_TOKEN_NAME
        || name.chars().any(char::is_control)
    {
        Err(InvalidTokenName)
    } else {
        Ok(name.to_owned())
    }
}

/// An API token. Its secret is shown once, when it is made; after that only its
/// [`prefix`](Self::prefix) identifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    pub id: Uuid,
    /// Whom it acts as: a person, or an agent.
    pub user_id: Uuid,
    /// The one workspace it works in; unset for a person's token that works across the
    /// instance. An agent's is always its workspace.
    pub workspace_id: Option<Uuid>,
    pub name: String,
    /// The token's first [`SHOWN_PREFIX_LENGTH`] characters.
    pub prefix: String,
    pub scopes: Vec<Scope>,
    pub created_at: OffsetDateTime,
    /// Unset for a token that never expires.
    pub expires_at: Option<OffsetDateTime>,
    pub last_used_at: Option<OffsetDateTime>,
}

impl ApiToken {
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at.is_some_and(|expires| now >= expires)
    }

    /// What its scopes allow now, for an owner whose instance role is `role`: a token
    /// across the instance loses the admin scope once its owner is no longer an instance
    /// admin. A bound token keeps it, as it answers to the workspace role each use reads.
    pub fn scopes_now(&self, role: Role) -> Vec<Scope> {
        if self.workspace_id.is_some() {
            self.scopes.clone()
        } else {
            effective_scopes(&self.scopes, role)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const ADMIN: TokenHolder = TokenHolder::Account { role: Role::Admin };
    const USER: TokenHolder = TokenHolder::Account { role: Role::User };

    #[test]
    fn scopes_are_sorted_and_deduplicated() {
        assert_eq!(
            token_scopes(&[Scope::Write, Scope::Read, Scope::Write], USER),
            Ok(vec![Scope::Read, Scope::Write])
        );
        assert_eq!(
            token_scopes(&[Scope::Admin, Scope::Read], ADMIN),
            Ok(vec![Scope::Read, Scope::Admin])
        );
    }

    #[test]
    fn scopes_must_be_some_and_fit_the_role() {
        assert_eq!(token_scopes(&[], ADMIN), Err(InvalidScopes::None));
        assert_eq!(
            token_scopes(&[Scope::Admin], USER),
            Err(InvalidScopes::AdminOnly)
        );
        assert_eq!(InvalidScopes::None.refusal(), None);
    }

    #[test]
    fn the_admin_scope_follows_whom_the_token_acts_for() {
        let cases = [
            (ADMIN, true),
            (USER, false),
            (TokenHolder::Workspace { manages: true }, true),
            (TokenHolder::Workspace { manages: false }, false),
            (TokenHolder::Agent, false),
        ];
        for (holder, admin) in cases {
            assert_eq!(
                token_scopes(&[Scope::Read, Scope::Admin], holder).is_ok(),
                admin,
                "{holder:?}"
            );
            assert_eq!(
                token_scopes(&[Scope::Read, Scope::Write], holder),
                Ok(vec![Scope::Read, Scope::Write]),
                "{holder:?}"
            );
            if let Err(refused) = token_scopes(&[Scope::Admin], holder) {
                assert_eq!(refused.refusal(), Some(refused.to_string()).as_deref());
            }
        }
        assert_eq!(
            token_scopes(&[Scope::Admin], TokenHolder::Agent),
            Err(InvalidScopes::AgentAdmin)
        );
    }

    #[test]
    fn a_demoted_owner_takes_the_admin_scope_away() {
        let scopes = [Scope::Read, Scope::Admin];

        assert_eq!(effective_scopes(&scopes, Role::Admin), scopes);
        assert_eq!(effective_scopes(&scopes, Role::User), [Scope::Read]);
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(check_token_name("  CI deploys "), Ok("CI deploys".into()));
        assert_eq!(check_token_name("   "), Err(InvalidTokenName));
        assert_eq!(
            check_token_name(&"x".repeat(MAX_TOKEN_NAME + 1)),
            Err(InvalidTokenName)
        );
        assert_eq!(check_token_name("tab\there"), Err(InvalidTokenName));
    }

    #[test]
    fn tokens_without_an_expiry_never_expire() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut token = ApiToken {
            id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            workspace_id: None,
            name: "ci".into(),
            prefix: "nw_abcdefg".into(),
            scopes: vec![Scope::Read],
            created_at: now,
            expires_at: None,
            last_used_at: None,
        };

        assert!(!token.is_expired(now + Duration::from_secs(1 << 30)));
        token.expires_at = Some(now + Duration::from_secs(60));
        assert!(!token.is_expired(now + Duration::from_secs(59)));
        assert!(token.is_expired(now + Duration::from_secs(60)));
    }

    #[test]
    fn a_bound_token_keeps_the_admin_scope_for_the_workspace_role_to_decide() {
        let mut token = ApiToken {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            workspace_id: None,
            name: "ci".into(),
            prefix: "nw_abcdefg".into(),
            scopes: vec![Scope::Read, Scope::Admin],
            created_at: OffsetDateTime::UNIX_EPOCH,
            expires_at: None,
            last_used_at: None,
        };

        assert_eq!(token.scopes_now(Role::User), [Scope::Read]);
        assert_eq!(token.scopes_now(Role::Admin), [Scope::Read, Scope::Admin]);
        token.workspace_id = Some(Uuid::nil());
        assert_eq!(token.scopes_now(Role::User), [Scope::Read, Scope::Admin]);
    }
}
