//! The people in a workspace and its teamspaces, agents and integrations among them, as
//! their lists show them.

use time::OffsetDateTime;
use uuid::Uuid;

use super::roles::{TeamspaceRole, WorkspaceRole};
use crate::agents::AgentOwner;
use crate::auth::UserKind;
use crate::hue::hue_or_hashed;

/// An identity as the people it works with see it: a person, or an agent or integration
/// with the person it acts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: Uuid,
    pub kind: UserKind,
    pub display_name: String,
    /// A person's address; agents and integrations have none.
    pub email: Option<String>,
    pub hue_override: Option<u16>,
    /// An agent's or integration's owner; unset for a person, and once the owner's account
    /// is gone.
    pub owner: Option<AgentOwner>,
}

impl Person {
    pub fn hue(&self) -> u16 {
        hue_or_hashed(self.hue_override, self.id)
    }
}

/// A member of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub person: Person,
    pub role: WorkspaceRole,
    pub joined_at: OffsetDateTime,
    /// An agent or integration whose owner is out of the workspace, a guest there or
    /// disabled, whose tokens are refused meanwhile; never a person.
    pub suspended: bool,
}

/// A member of a teamspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamspaceMember {
    pub person: Person,
    pub role: TeamspaceRole,
    pub joined_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_are_drawn_in_their_own_hue() {
        let id = Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").expect("valid");
        let mut ada = Person {
            id,
            kind: UserKind::Human,
            display_name: "Ada".into(),
            email: Some("ada@example.com".into()),
            hue_override: None,
            owner: None,
        };

        assert_eq!(ada.hue(), 168);
        ada.hue_override = Some(300);
        assert_eq!(ada.hue(), 300);
    }
}
