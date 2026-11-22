//! Who may do what in a workspace, in one place: [`authorize`]. Every workspace use case
//! asks it before it acts, so the rules are read and tested here alone.
//!
//! A refusal is [`Denied::Hidden`] when the actor may not even know the thing exists, such
//! as a workspace they are not in, a private teamspace, or to a guest another member, so
//! the answer is the same as for one that does not exist; otherwise it is
//! [`Denied::Forbidden`], with the reason.
//!
//! Instance admins have no part inside a workspace: being one counts only for the
//! instance's own actions, listing workspaces, setting their limits and deleting them.
//! In a workspace they are whatever their membership makes them, which may be nothing.

use super::roles::{TeamspaceAccess, TeamspaceRole, WorkspaceCreation, WorkspaceRole};

/// Who acts, as far as the rules need to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    /// An instance admin, through a browser session or a token with the admin scope.
    pub instance_admin: bool,
    /// Signed in at a browser rather than with an API token. Deleting a workspace and
    /// transferring its ownership need it, so a token that leaked cannot take a team's work
    /// away.
    pub via_session: bool,
    /// Their role in the workspace concerned; `None` when they are not in it.
    pub role: Option<WorkspaceRole>,
}

/// What an action is done to, with what the rules need to know of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// The instance, for creating workspaces and the instance admins' work.
    Instance { creation: WorkspaceCreation },
    /// A workspace as a whole.
    Workspace { members_can_create_teamspaces: bool },
    /// One member of the workspace: their role, and whether they are the actor.
    Member { role: WorkspaceRole, is_actor: bool },
    /// A teamspace of the workspace, and the actor's role in it, if any.
    Teamspace {
        access: TeamspaceAccess,
        membership: Option<TeamspaceRole>,
    },
}

/// Everything the rules decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // On the instance.
    CreateWorkspace,
    /// Instance admins' list: names, owners, counts, limits, usage. No content.
    ListAllWorkspaces,
    SetLimits,
    /// Deleting any workspace, as an instance admin, from a browser session.
    DeleteAnyWorkspace,

    // On a workspace.
    ViewWorkspace,
    /// Its name, slug, icon, hue and settings.
    UpdateWorkspace,
    /// From a browser session.
    DeleteWorkspace,
    /// Every member, and how many there are. A guest knows only themselves, by
    /// [`ViewMember`](Self::ViewMember).
    ListMembers,
    Leave,
    /// Inviting an account by address, to join with `role`.
    Invite {
        role: WorkspaceRole,
    },
    /// Listing and revoking the invitations sent.
    ManageInvitations,
    /// Making an invite link that admits people with `role`.
    CreateInviteLink {
        role: WorkspaceRole,
    },
    /// Listing, copying and revoking invite links.
    ManageInviteLinks,
    CreateTeamspace,

    // On a member.
    /// That they are in the workspace, with their role.
    ViewMember,
    ChangeRole {
        to: WorkspaceRole,
    },
    RemoveMember,
    /// Making the member an owner, the actor becoming an admin, from a browser session.
    TransferOwnership,

    // On a teamspace.
    /// Everything about it: its settings, members and, later, pages.
    ViewTeamspace,
    /// That it exists, by name: for private teamspaces, what owners and admins see.
    SeeTeamspace,
    JoinTeamspace,
    LeaveTeamspace,
    /// Its name, icon and hue.
    UpdateTeamspace,
    ChangeTeamspaceAccess {
        to: TeamspaceAccess,
    },
    DeleteTeamspace,
    ListTeamspaceMembers,
    /// Adding a member of the workspace whose role there is `workspace_role`.
    AddTeamspaceMember {
        workspace_role: WorkspaceRole,
    },
    RemoveTeamspaceMember,
    ChangeTeamspaceRole,
}

/// Why an action is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    /// The actor may not know it exists: answer as for something that does not.
    #[error("not found")]
    Hidden,
    /// The actor knows it, and may not do this.
    #[error("{0}")]
    Forbidden(&'static str),
}

use Denied::{Forbidden, Hidden};

const NOT_APPLICABLE: Denied = Forbidden("this cannot be done to that");

/// Whether `actor` may do `action` to `resource`.
pub fn authorize(actor: Actor, action: Action, resource: Resource) -> Result<(), Denied> {
    match resource {
        Resource::Instance { creation } => on_instance(actor, action, creation),
        Resource::Workspace {
            members_can_create_teamspaces,
        } => on_workspace(
            member(actor)?,
            actor.via_session,
            action,
            members_can_create_teamspaces,
        ),
        Resource::Member { role, is_actor } => {
            on_member(member(actor)?, actor.via_session, action, role, is_actor)
        }
        Resource::Teamspace { access, membership } => {
            on_teamspace(member(actor)?, action, access, membership)
        }
    }
}

/// The actor's role in the workspace. Anyone else, instance admins included, may not know
/// what is in it.
fn member(actor: Actor) -> Result<WorkspaceRole, Denied> {
    actor.role.ok_or(Hidden)
}

/// Refuses an action that cannot be undone to an API token.
fn needs_session(via_session: bool, what: &'static str) -> Result<(), Denied> {
    allow_if(via_session, Forbidden(what))
}

const DELETE_NEEDS_SESSION: &str =
    "deleting a workspace needs a browser session: API tokens cannot";

fn on_instance(actor: Actor, action: Action, creation: WorkspaceCreation) -> Result<(), Denied> {
    const ADMINS_ONLY: Denied = Forbidden("only an instance admin may do this");

    match action {
        Action::CreateWorkspace => match creation {
            WorkspaceCreation::Anyone => Ok(()),
            WorkspaceCreation::Admins if actor.instance_admin => Ok(()),
            WorkspaceCreation::Admins => {
                Err(Forbidden("only instance admins create workspaces here"))
            }
        },
        Action::ListAllWorkspaces | Action::SetLimits => {
            allow_if(actor.instance_admin, ADMINS_ONLY)
        }
        Action::DeleteAnyWorkspace => {
            allow_if(actor.instance_admin, ADMINS_ONLY)?;
            needs_session(actor.via_session, DELETE_NEEDS_SESSION)
        }
        _ => Err(NOT_APPLICABLE),
    }
}

fn on_workspace(
    role: WorkspaceRole,
    via_session: bool,
    action: Action,
    members_can_create_teamspaces: bool,
) -> Result<(), Denied> {
    use WorkspaceRole::{Admin, Guest, Member, Owner};

    const MANAGERS_ONLY: Denied = Forbidden("only the workspace's owners and admins may do this");

    match action {
        Action::ViewWorkspace | Action::Leave => Ok(()),
        Action::UpdateWorkspace | Action::ManageInvitations | Action::ManageInviteLinks => {
            allow_if(role.manages(), MANAGERS_ONLY)
        }
        Action::DeleteWorkspace => {
            allow_if(
                role == Owner,
                Forbidden("only an owner may delete the workspace"),
            )?;
            needs_session(via_session, DELETE_NEEDS_SESSION)
        }
        Action::ListMembers => allow_if(
            role != Guest,
            Forbidden("guests see no member but themselves"),
        ),
        Action::Invite { role: invited } => {
            if !role.manages() {
                Err(MANAGERS_ONLY)
            } else if invited == Owner {
                Err(Forbidden(
                    "nobody is invited as an owner: an owner transfers ownership to a member",
                ))
            } else {
                Ok(())
            }
        }
        Action::CreateInviteLink { role: admitted } => {
            if !role.manages() {
                Err(MANAGERS_ONLY)
            } else if !matches!(admitted, Member | Guest) {
                Err(Forbidden("an invite link admits members or guests"))
            } else {
                Ok(())
            }
        }
        Action::CreateTeamspace => match role {
            Owner | Admin => Ok(()),
            Member if members_can_create_teamspaces => Ok(()),
            Member => Err(Forbidden(
                "in this workspace only owners and admins create teamspaces",
            )),
            Guest => Err(Forbidden("guests cannot create teamspaces")),
        },
        _ => Err(NOT_APPLICABLE),
    }
}

fn on_member(
    role: WorkspaceRole,
    via_session: bool,
    action: Action,
    target: WorkspaceRole,
    is_actor: bool,
) -> Result<(), Denied> {
    use WorkspaceRole::{Admin, Guest, Owner};

    const MANAGERS_ONLY: Denied =
        Forbidden("only the workspace's owners and admins manage its members");
    const OWNERS_ONLY: Denied = Forbidden("only an owner may change or remove an owner");

    // Guests may not know who else is in the workspace.
    if role == Guest && !is_actor {
        return Err(Hidden);
    }
    match action {
        Action::ViewMember => Ok(()),
        Action::ChangeRole { to } => match role {
            Owner => Ok(()),
            Admin if target == Owner => Err(OWNERS_ONLY),
            Admin if to == Owner => Err(Forbidden("only an owner may make an owner")),
            Admin => Ok(()),
            _ => Err(MANAGERS_ONLY),
        },
        Action::RemoveMember => match role {
            _ if is_actor => Err(Forbidden("to remove yourself, leave the workspace")),
            Owner => Ok(()),
            Admin if target == Owner => Err(OWNERS_ONLY),
            Admin => Ok(()),
            _ => Err(MANAGERS_ONLY),
        },
        Action::TransferOwnership => {
            if role != Owner {
                Err(Forbidden("only an owner may transfer ownership"))
            } else if is_actor || target == Owner {
                Err(Forbidden(
                    "ownership goes to a member who is not an owner yet",
                ))
            } else {
                needs_session(
                    via_session,
                    "transferring ownership needs a browser session: API tokens cannot",
                )
            }
        }
        _ => Err(NOT_APPLICABLE),
    }
}

fn on_teamspace(
    role: WorkspaceRole,
    action: Action,
    access: TeamspaceAccess,
    membership: Option<TeamspaceRole>,
) -> Result<(), Denied> {
    const OWNERS_ONLY: Denied = Forbidden(
        "only the teamspace's owners, and the workspace's owners and admins, may do this",
    );

    // Guests see no teamspace at all.
    if !role.joins_teamspaces() {
        return Err(Hidden);
    }
    let manager = role.manages();
    if access == TeamspaceAccess::Private && membership.is_none() {
        // Owners and admins know a private teamspace they are not in by its name alone.
        // They may delete it, or join it to manage it, which its members then see; all
        // else about it stays hidden, as it is from everyone else outside it.
        return match action {
            Action::SeeTeamspace | Action::DeleteTeamspace | Action::JoinTeamspace if manager => {
                Ok(())
            }
            _ => Err(Hidden),
        };
    }
    let owns = membership == Some(TeamspaceRole::Owner) || manager;

    match action {
        Action::SeeTeamspace | Action::ViewTeamspace | Action::ListTeamspaceMembers => Ok(()),
        Action::JoinTeamspace => match access {
            _ if membership.is_some() => Err(Forbidden("you are in this teamspace already")),
            TeamspaceAccess::Default => {
                Err(Forbidden("everyone but guests is in the default teamspace"))
            }
            TeamspaceAccess::Open => Ok(()),
            TeamspaceAccess::Closed | TeamspaceAccess::Private => allow_if(
                manager,
                Forbidden("a teamspace owner adds who joins this teamspace"),
            ),
        },
        Action::LeaveTeamspace => match access {
            TeamspaceAccess::Default => Err(Forbidden("nobody leaves the default teamspace")),
            _ => allow_if(
                membership.is_some(),
                Forbidden("you are not in this teamspace"),
            ),
        },
        Action::UpdateTeamspace | Action::ChangeTeamspaceRole => allow_if(owns, OWNERS_ONLY),
        Action::ChangeTeamspaceAccess { to } => {
            if !owns {
                Err(OWNERS_ONLY)
            } else if access == TeamspaceAccess::Default {
                Err(Forbidden("the default teamspace keeps its access"))
            } else if to == TeamspaceAccess::Default {
                Err(Forbidden("a workspace has one default teamspace"))
            } else {
                Ok(())
            }
        }
        Action::DeleteTeamspace => match access {
            TeamspaceAccess::Default => Err(Forbidden("the default teamspace cannot be deleted")),
            _ => allow_if(owns, OWNERS_ONLY),
        },
        Action::AddTeamspaceMember { workspace_role } => match access {
            TeamspaceAccess::Default => Err(IN_THE_DEFAULT),
            _ if !owns => Err(OWNERS_ONLY),
            _ => allow_if(
                workspace_role.joins_teamspaces(),
                Forbidden("guests are in no teamspace: make them a member first"),
            ),
        },
        Action::RemoveTeamspaceMember => match access {
            TeamspaceAccess::Default => Err(IN_THE_DEFAULT),
            _ => allow_if(owns, OWNERS_ONLY),
        },
        _ => Err(NOT_APPLICABLE),
    }
}

const IN_THE_DEFAULT: Denied =
    Forbidden("everyone but guests is in the default teamspace, and stays in it");

fn allow_if(allowed: bool, otherwise: Denied) -> Result<(), Denied> {
    if allowed { Ok(()) } else { Err(otherwise) }
}

/// A change that would leave a workspace without an owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a workspace keeps at least one owner: make another owner first")]
pub struct LastOwner;

/// Whether a workspace with `owners` owners keeps one when a member who is an owner now
/// (`was_owner`) is one after the change or not (`stays_owner`).
pub fn check_owner_remains(
    owners: i64,
    was_owner: bool,
    stays_owner: bool,
) -> Result<(), LastOwner> {
    if was_owner && !stays_owner && owners <= 1 {
        Err(LastOwner)
    } else {
        Ok(())
    }
}

/// A change that would leave a teamspace without an owner, made by someone who does not
/// manage the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a teamspace keeps at least one owner: make another owner first")]
pub struct LastTeamspaceOwner;

/// Like [`check_owner_remains`] for a teamspace, which the workspace's owners and admins
/// (`actor_manages`) may leave without an owner, as they manage it themselves.
pub fn check_teamspace_owner_remains(
    owners: i64,
    was_owner: bool,
    stays_owner: bool,
    actor_manages: bool,
) -> Result<(), LastTeamspaceOwner> {
    if was_owner && !stays_owner && owners <= 1 && !actor_manages {
        Err(LastTeamspaceOwner)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
