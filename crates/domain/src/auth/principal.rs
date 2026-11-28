//! Who a request acts as, and what that lets it do.

use uuid::Uuid;

use super::token::Scope;
use super::user::User;
use crate::workspaces::WorkspaceRole;

/// How a request proved who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// A browser session's cookie. The user themself, with every scope.
    Session { id: Uuid },
    /// An API token, limited to its scopes as its owner's role allows them now, and to the
    /// workspace it is bound to, if any.
    Token {
        id: Uuid,
        name: String,
        scopes: Vec<Scope>,
        workspace_id: Option<Uuid>,
    },
}

/// An authenticated identity, and how: a person, or an agent acting for its owner.
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

    /// The one workspace its token works in: a bound token's, an agent's always. Unset
    /// for a session and for a person's token across the instance.
    pub fn workspace(&self) -> Option<Uuid> {
        match self.credential {
            Credential::Session { .. } => None,
            Credential::Token { workspace_id, .. } => workspace_id,
        }
    }

    /// Whether it may act in `workspace_id` at all: anywhere, unless its token is bound to
    /// another workspace.
    pub fn reaches(&self, workspace_id: Uuid) -> bool {
        self.workspace().is_none_or(|bound| bound == workspace_id)
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

    /// An instance admin, through a session or a token across the instance with the admin
    /// scope. A token bound to a workspace never is.
    pub fn is_admin(&self) -> bool {
        self.user.is_admin() && self.can(Scope::Admin) && self.workspace().is_none()
    }

    /// The scopes the principal has, in [`Scope::ALL`] order. Admin shows for an instance
    /// admin, and for a token bound to a workspace, whose role there decides it.
    pub fn scopes(&self) -> Vec<Scope> {
        let bound = self.workspace().is_some();
        Scope::ALL
            .iter()
            .copied()
            .filter(|scope| self.can(*scope))
            .filter(|scope| *scope != Scope::Admin || self.user.is_admin() || bound)
            .collect()
    }

    /// The role the rules see the principal in, in `workspace_id`, where its identity's
    /// role is `role`: none in a workspace its token is not bound to, and, for a bound
    /// token without the admin scope, member at most, so that it manages nothing.
    pub fn role_in(
        &self,
        workspace_id: Uuid,
        role: Option<WorkspaceRole>,
    ) -> Option<WorkspaceRole> {
        if !self.reaches(workspace_id) {
            return None;
        }
        match role {
            Some(role)
                if role.manages() && self.workspace().is_some() && !self.can(Scope::Admin) =>
            {
                Some(WorkspaceRole::Member)
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::*;
    use crate::auth::{Email, Role, UserKind};

    fn user(role: Role) -> User {
        let now = OffsetDateTime::UNIX_EPOCH;
        User {
            id: Uuid::now_v7(),
            kind: UserKind::Human,
            email: Some(Email::parse("ada@example.com").expect("valid")),
            display_name: "Ada".into(),
            role,
            hue_override: None,
            email_verified_at: Some(now),
            disabled_at: None,
            created_at: now,
            updated_at: now,
            workspace_id: None,
            owner_id: None,
        }
    }

    fn bound_token(role: Role, scopes: &[Scope], workspace_id: Option<Uuid>) -> Principal {
        Principal {
            user: user(role),
            credential: Credential::Token {
                id: Uuid::now_v7(),
                name: "ci".into(),
                scopes: scopes.to_vec(),
                workspace_id,
            },
        }
    }

    fn token(role: Role, scopes: &[Scope]) -> Principal {
        bound_token(role, scopes, None)
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
        assert_eq!(admin.workspace(), None);
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

    #[test]
    fn a_bound_token_works_in_its_workspace_alone_and_is_never_an_instance_admin() {
        let here = Uuid::from_u128(1);
        let there = Uuid::from_u128(2);
        let bound = bound_token(Role::Admin, &[Scope::Admin], Some(here));
        let across = token(Role::User, &[Scope::Read]);

        assert_eq!(bound.workspace(), Some(here));
        assert!(bound.reaches(here) && !bound.reaches(there));
        assert!(
            !bound.is_admin(),
            "an instance admin's bound token stays out"
        );
        assert_eq!(bound.scopes(), [Scope::Read, Scope::Admin]);
        assert!(across.reaches(here) && across.reaches(there));
        assert_eq!(
            bound.role_in(there, Some(WorkspaceRole::Owner)),
            None,
            "another workspace is out of its reach"
        );
        assert_eq!(
            across.role_in(there, Some(WorkspaceRole::Owner)),
            Some(WorkspaceRole::Owner)
        );
        assert_eq!(session(Role::User).role_in(there, None), None);
    }

    #[test]
    fn a_bound_token_manages_only_with_the_admin_scope() {
        let here = Uuid::from_u128(1);
        let writer = bound_token(Role::User, &[Scope::Read, Scope::Write], Some(here));
        let admin = bound_token(Role::User, &[Scope::Write, Scope::Admin], Some(here));
        let across = token(Role::User, &[Scope::Write]);

        for role in WorkspaceRole::ALL {
            let capped = if role.manages() {
                WorkspaceRole::Member
            } else {
                *role
            };
            assert_eq!(writer.role_in(here, Some(*role)), Some(capped), "{role}");
            assert_eq!(admin.role_in(here, Some(*role)), Some(*role), "{role}");
            assert_eq!(across.role_in(here, Some(*role)), Some(*role), "{role}");
        }
        assert_eq!(writer.role_in(here, None), None);
    }
}
