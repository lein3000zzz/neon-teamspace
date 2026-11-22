//! API tokens: programmatic access as their owner, limited to the scopes they were made
//! with, and never beyond what the owner's role allows.

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
    /// The admin pages' work. Only an admin's token may have it, and it stops working
    /// when its owner stops being an admin.
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

/// Scopes a new token cannot have.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidScopes {
    #[error("a token needs at least one scope")]
    None,
    #[error("there is no scope {0:?}: want read, write or admin")]
    Unknown(String),
    #[error("only an admin's token may have the admin scope")]
    AdminOnly,
}

/// The scopes a new token of an owner with `role` gets: each once, in [`Scope::ALL`]
/// order. At least one, and admin only for an admin.
pub fn token_scopes<S: AsRef<str>>(
    requested: &[S],
    role: Role,
) -> Result<Vec<Scope>, InvalidScopes> {
    if requested.is_empty() {
        return Err(InvalidScopes::None);
    }
    let mut scopes = requested
        .iter()
        .map(|text| {
            let text = text.as_ref();
            Scope::parse(text).ok_or_else(|| InvalidScopes::Unknown(text.to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if scopes.contains(&Scope::Admin) && role != Role::Admin {
        return Err(InvalidScopes::AdminOnly);
    }
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
    pub user_id: Uuid,
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
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn scopes_are_sorted_and_deduplicated() {
        assert_eq!(
            token_scopes(&["write", "read", "write"], Role::User),
            Ok(vec![Scope::Read, Scope::Write])
        );
        assert_eq!(
            token_scopes(&["admin", "read"], Role::Admin),
            Ok(vec![Scope::Read, Scope::Admin])
        );
    }

    #[test]
    fn scopes_must_exist_and_fit_the_role() {
        let none: [&str; 0] = [];

        assert_eq!(token_scopes(&none, Role::Admin), Err(InvalidScopes::None));
        assert_eq!(
            token_scopes(&["read", "delete"], Role::Admin),
            Err(InvalidScopes::Unknown("delete".into()))
        );
        assert_eq!(
            token_scopes(&["Read"], Role::Admin),
            Err(InvalidScopes::Unknown("Read".into()))
        );
        assert_eq!(
            token_scopes(&["admin"], Role::User),
            Err(InvalidScopes::AdminOnly)
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
}
