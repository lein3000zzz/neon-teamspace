//! Who a request acts as, and what that lets it do.

use uuid::Uuid;

use super::token::Scope;
use super::user::User;

/// How a request proved who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// A browser session's cookie. The user themself, with every scope.
    Session { id: Uuid },
    /// An API token, limited to its scopes as its owner's role allows them now.
    Token {
        id: Uuid,
        name: String,
        scopes: Vec<Scope>,
    },
}

/// An authenticated user, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub user: User,
    pub credential: Credential,
}

impl Principal {
    /// Signed in at a browser, not with a token. Managing passwords, sessions and tokens
    /// needs this.
    pub fn via_session(&self) -> bool {
        matches!(self.credential, Credential::Session { .. })
    }

    /// The session's id, for a browser session.
    pub fn session_id(&self) -> Option<Uuid> {
        match self.credential {
            Credential::Session { id } => Some(id),
            Credential::Token { .. } => None,
        }
    }

    /// Whether the principal's scopes allow `scope`. A session has every scope. Every
    /// scope brings read, so a token with any scope reads.
    pub fn can(&self, scope: Scope) -> bool {
        match &self.credential {
            Credential::Session { .. } => true,
            Credential::Token { scopes, .. } => {
                scope == Scope::Read && !scopes.is_empty() || scopes.contains(&scope)
            }
        }
    }

    /// An admin, through a session or a token with the admin scope.
    pub fn is_admin(&self) -> bool {
        self.user.is_admin() && self.can(Scope::Admin)
    }

    /// The scopes the principal has, in [`Scope::ALL`] order.
    pub fn scopes(&self) -> Vec<Scope> {
        Scope::ALL
            .into_iter()
            .filter(|scope| self.can(*scope))
            .filter(|scope| *scope != Scope::Admin || self.user.is_admin())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;
    use crate::auth::{Email, Role};

    fn user(role: Role) -> User {
        let now = OffsetDateTime::UNIX_EPOCH;
        User {
            id: Uuid::now_v7(),
            email: Email::parse("ada@example.com").expect("valid"),
            display_name: "Ada".into(),
            role,
            email_verified_at: Some(now),
            disabled_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn token(role: Role, scopes: &[Scope]) -> Principal {
        Principal {
            user: user(role),
            credential: Credential::Token {
                id: Uuid::now_v7(),
                name: "ci".into(),
                scopes: scopes.to_vec(),
            },
        }
    }

    fn session(role: Role) -> Principal {
        Principal {
            user: user(role),
            credential: Credential::Session { id: Uuid::now_v7() },
        }
    }

    #[test]
    fn a_session_has_every_scope_its_role_allows() {
        let admin = session(Role::Admin);
        let member = session(Role::User);

        assert!(admin.via_session() && admin.session_id().is_some());
        assert!(Scope::ALL.iter().all(|scope| admin.can(*scope)));
        assert!(admin.is_admin());
        assert!(!member.is_admin());
        assert_eq!(member.scopes(), [Scope::Read, Scope::Write]);
        assert_eq!(admin.scopes(), Scope::ALL);
    }

    #[test]
    fn a_token_has_its_scopes_and_read_with_any() {
        let writer = token(Role::User, &[Scope::Write]);
        let reader = token(Role::User, &[Scope::Read]);
        let nothing = token(Role::User, &[]);

        assert!(!writer.via_session() && writer.session_id().is_none());
        assert!(writer.can(Scope::Read) && writer.can(Scope::Write));
        assert!(reader.can(Scope::Read) && !reader.can(Scope::Write));
        assert!(!nothing.can(Scope::Read));
        assert_eq!(writer.scopes(), [Scope::Read, Scope::Write]);
    }

    #[test]
    fn admin_needs_both_the_role_and_the_scope() {
        assert!(token(Role::Admin, &[Scope::Admin]).is_admin());
        assert!(!token(Role::Admin, &[Scope::Read, Scope::Write]).is_admin());
        assert!(!token(Role::User, &[Scope::Admin]).is_admin());
        assert_eq!(
            token(Role::Admin, &[Scope::Admin]).scopes(),
            [Scope::Read, Scope::Admin]
        );
    }
}
