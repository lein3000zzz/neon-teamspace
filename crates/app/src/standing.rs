//! Who acts in a workspace, as the permission rules need to know it: the principal's role
//! there, as far as its token reaches, and, for an agent or integration, its owner's role
//! too. Use cases read it, then ask the rules about a resource as the principal stands to
//! it and as the owner does, which for people is the same resource.

use neon_domain::auth::Principal;
use neon_domain::workspaces::permissions::members_hidden;
use neon_domain::workspaces::{
    Action, Actor, Denied, OwnerStanding, Resource, WorkspaceRole, authorize,
};
use neon_store::workspaces;
use sqlx::PgConnection;
use uuid::Uuid;

/// The principal in one workspace, or outside every workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Standing {
    instance_admin: bool,
    via_session: bool,
    role: Option<WorkspaceRole>,
    owner: Option<Owner>,
}

/// An agent's or integration's owner in the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Owner {
    id: Uuid,
    role: Option<WorkspaceRole>,
}

/// Whose eyes an agent's or integration's resources are also seen through: its owner's.
/// An owner whose account is gone is the nil id, which no row names, so they stand
/// nowhere and the agent with them.
pub(crate) fn owner_of(principal: &Principal) -> Option<Uuid> {
    (!principal.user.kind.is_human()).then(|| principal.user.owner_id.unwrap_or_default())
}

impl Standing {
    /// From roles read already: the principal's own in `workspace_id` and, for an agent or
    /// integration, its owner's there. A token bound elsewhere stands nowhere in it.
    pub(crate) fn new(
        principal: &Principal,
        workspace_id: Uuid,
        role: Option<WorkspaceRole>,
        owner_role: Option<WorkspaceRole>,
    ) -> Self {
        Self {
            instance_admin: principal.is_admin(),
            via_session: principal.via_session(),
            role: principal.role_in(workspace_id, role),
            owner: owner_of(principal).map(|id| Owner {
                id,
                role: owner_role,
            }),
        }
    }

    /// Outside any workspace: for what the rules decide on the instance.
    pub(crate) fn instance(principal: &Principal) -> Self {
        Self {
            instance_admin: principal.is_admin(),
            via_session: principal.via_session(),
            role: None,
            owner: owner_of(principal).map(|id| Owner { id, role: None }),
        }
    }

    /// Reads the roles in `workspace_id` on `conn`, which may hold its lock.
    pub(crate) async fn read(
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> sqlx::Result<Self> {
        let role = workspaces::member_role(&mut *conn, workspace_id, principal.user.id).await?;
        let owner_role = match owner_of(principal) {
            Some(owner) => workspaces::member_role(&mut *conn, workspace_id, owner).await?,
            None => None,
        };
        Ok(Self::new(principal, workspace_id, role, owner_role))
    }

    /// The principal's role, as the rules see it.
    pub(crate) fn role(&self) -> Option<WorkspaceRole> {
        self.role
    }

    /// An agent's or integration's owner, through whose eyes resources are seen too.
    pub(crate) fn owner_id(&self) -> Option<Uuid> {
        self.owner.map(|owner| owner.id)
    }

    /// Whether what is shown leaves out who else is in the workspace: for a guest, and for
    /// an agent of a guest.
    pub(crate) fn members_hidden(&self) -> bool {
        members_hidden(self.role, self.owner.map(|owner| owner.role))
    }

    /// The actor for the rules, its owner standing to the resource as `theirs`.
    fn actor(&self, theirs: Resource) -> Actor {
        Actor {
            instance_admin: self.instance_admin,
            via_session: self.via_session,
            role: self.role,
            on_behalf_of: self.owner.map(|owner| OwnerStanding {
                role: owner.role,
                resource: theirs,
            }),
        }
    }

    /// Whether the principal may do `action` to `mine`, the resource as it stands to it,
    /// its owner standing to it as `theirs`.
    pub(crate) fn authorize(
        &self,
        action: Action,
        mine: Resource,
        theirs: Resource,
    ) -> Result<(), Denied> {
        authorize(self.actor(theirs), action, mine)
    }

    /// As [`authorize`](Self::authorize), for a resource nobody stands to otherwise: the
    /// instance, a workspace as a whole, its agents.
    pub(crate) fn authorize_same(&self, action: Action, resource: Resource) -> Result<(), Denied> {
        self.authorize(action, resource, resource)
    }
}
