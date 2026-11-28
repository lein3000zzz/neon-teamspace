//! The roles people have in a workspace and in its teamspaces, the kinds of teamspace, and
//! who may create workspaces.

text_enum! {
    /// What a person may do in a workspace. A workspace always keeps at least one owner.
    WorkspaceRole {
        /// Everything, the workspace itself included: deleting it, and making owners.
        Owner => "owner",
        /// Manages members, guests, invitations, invite links and teamspaces, but never an
        /// owner.
        Admin => "admin",
        /// Works in the workspace and its teamspaces.
        Member => "member",
        /// Sees only the pages shared with them: no teamspaces, and no member but themselves.
        Guest => "guest",
    }
}

impl WorkspaceRole {
    /// Owners and admins, who manage the workspace.
    pub fn manages(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Everyone but guests is in the default teamspace, and may be in others.
    pub fn joins_teamspaces(self) -> bool {
        self != Self::Guest
    }
}

text_enum! {
    /// What a person may do in a teamspace.
    TeamspaceRole {
        /// Manages the teamspace and its members.
        Owner => "owner",
        Member => "member",
    }
}

text_enum! {
    /// Who sees a teamspace and who may join it.
    TeamspaceAccess {
        /// The workspace's own teamspace, one per workspace, which everyone but guests is in.
        /// It cannot be deleted, left, or given another access.
        Default => "default",
        /// Every member sees it, and joins or leaves it at will.
        Open => "open",
        /// Every member sees it; its owners, or the workspace's owners and admins, add who
        /// joins.
        Closed => "closed",
        /// Only its members see it. The workspace's owners and admins see its name, to
        /// manage or delete it.
        Private => "private",
    }
}

text_enum! {
    /// Who may create a workspace, the `workspaces.creation` setting.
    WorkspaceCreation {
        /// Every account.
        Anyone => "anyone",
        /// Instance admins only.
        Admins => "admins",
    }
}

impl WorkspaceCreation {
    pub const DEFAULT: Self = Self::Anyone;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_round_trips_through_its_text() {
        for role in WorkspaceRole::ALL {
            assert_eq!(WorkspaceRole::parse(role.as_str()), Some(*role));
        }
        for role in TeamspaceRole::ALL {
            assert_eq!(TeamspaceRole::parse(&role.to_string()), Some(*role));
        }
        for access in TeamspaceAccess::ALL {
            assert_eq!(TeamspaceAccess::parse(access.as_str()), Some(*access));
        }
        for creation in WorkspaceCreation::ALL {
            assert_eq!(WorkspaceCreation::parse(creation.as_str()), Some(*creation));
        }
        assert_eq!(WorkspaceRole::parse("Owner"), None);
        assert_eq!(TeamspaceRole::parse("admin"), None);
        assert_eq!(TeamspaceAccess::parse("public"), None);
    }

    #[test]
    fn anyone_creates_workspaces_by_default() {
        assert_eq!(WorkspaceCreation::DEFAULT, WorkspaceCreation::Anyone);
    }

    #[test]
    fn owners_and_admins_manage_and_guests_stay_out_of_teamspaces() {
        let managing: Vec<WorkspaceRole> = WorkspaceRole::ALL
            .iter()
            .copied()
            .filter(|role| role.manages())
            .collect();
        let joining: Vec<WorkspaceRole> = WorkspaceRole::ALL
            .iter()
            .copied()
            .filter(|role| role.joins_teamspaces())
            .collect();

        assert_eq!(managing, [WorkspaceRole::Owner, WorkspaceRole::Admin]);
        assert_eq!(
            joining,
            [
                WorkspaceRole::Owner,
                WorkspaceRole::Admin,
                WorkspaceRole::Member
            ]
        );
    }
}
