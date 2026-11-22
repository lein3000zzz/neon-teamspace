//! The people in a workspace and its teamspaces, as their lists show them.

use time::OffsetDateTime;
use uuid::Uuid;

use super::roles::{TeamspaceRole, WorkspaceRole};
use crate::hue::hue_or_hashed;

/// An account as the people it works with see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: Uuid,
    pub display_name: String,
    pub email: String,
    pub hue_override: Option<u16>,
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
            display_name: "Ada".into(),
            email: "ada@example.com".into(),
            hue_override: None,
        };

        assert_eq!(ada.hue(), 168);
        ada.hue_override = Some(300);
        assert_eq!(ada.hue(), 300);
    }
}
