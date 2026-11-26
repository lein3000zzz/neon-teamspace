//! Who may do what in a workspace, in one place: [`authorize`]. Every workspace and page use
//! case asks it before it acts, so the rules are read and tested here alone.
//!
//! A refusal is [`Denied::Hidden`] when the actor may not even know the thing exists, such
//! as a workspace they are not in, a private teamspace, a page nobody shared with them, or
//! to a guest another member, so the answer is the same as for one that does not exist;
//! otherwise it is [`Denied::Forbidden`], with the reason.
//!
//! A page gives what its space gives the actor's role, or what it, or a page above it, is
//! shared with them for, whichever is more:
//!
//! - A teamspace's members edit its pages. Everyone else but guests views the pages of a
//!   teamspace they may see, the default, open and closed ones; a private teamspace's
//!   pages are for its members alone, the workspace's owners and admins included.
//! - A private page is its owner's alone.
//! - A share, to view or to edit, holds for the page and every page under it.
//! - Guests reach pages only through shares.
//! - A page leaves its space, for another teamspace or someone's private pages, only by
//!   those its space lets edit it: a teamspace's members, a private page's owner. A share
//!   to edit lets its holder move the page within its space, under pages they edit, never
//!   out of it, so a share never hands a private page or a private teamspace's page to
//!   others.
//!
//! A page in the trash shows only to those who could edit it, to restore it or delete it
//! for good.
//!
//! An agent or an integration acts for its owner, a person in the same workspace, and may
//! do only what both may: what its own role, teamspaces and shares allow it, and what its
//! owner's allow them, asked of the same resource as the owner stands to it
//! ([`OwnerStanding`]). What one of the two may not know of stays hidden; what one may not
//! do is forbidden. Joining a teamspace asks the owner to see it in full, and leaving one
//! asks nothing of them. Agents and integrations work in the workspace and never manage
//! it: they change no member, teamspace setting or agent, invite nobody and make nothing
//! that would make them an owner.
//!
//! Webhooks: the workspace's owners and admins make its incoming webhooks, and register
//! outgoing ones for anyone in it; everyone else but guests registers them for themselves
//! and for their own agents and integrations, and manages those alone.
//!
//! Instance admins have no part inside a workspace: being one counts only for the
//! instance's own actions, listing workspaces, setting their limits and deleting them.
//! In a workspace they are whatever their membership makes them, which may be nothing.

use super::roles::{TeamspaceAccess, TeamspaceRole, WorkspaceCreation, WorkspaceRole};
use crate::agents::AgentCreation;
use crate::auth::UserKind;
use crate::pages::PageAccess;

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
    /// Set when an agent or an integration acts: its owner, whose access bounds its own.
    pub on_behalf_of: Option<OwnerStanding>,
}

impl Actor {
    /// A person acting for themselves.
    pub fn person(instance_admin: bool, via_session: bool, role: Option<WorkspaceRole>) -> Self {
        Self {
            instance_admin,
            via_session,
            role,
            on_behalf_of: None,
        }
    }

    /// Whether what is shown to the actor leaves out who else is in the workspace: see
    /// [`members_hidden`].
    pub fn members_hidden(&self) -> bool {
        members_hidden(self.role, self.on_behalf_of.map(|owner| owner.role))
    }
}

/// Whether what is shown leaves out who else is in the workspace, for someone with `role`
/// there, acting for an owner with `owner_role` if any: for a guest, and for an agent or
/// integration of a guest.
pub fn members_hidden(
    role: Option<WorkspaceRole>,
    owner_role: Option<Option<WorkspaceRole>>,
) -> bool {
    let guest = |role: Option<WorkspaceRole>| role.is_some_and(|role| !role.joins_teamspaces());
    guest(role) || owner_role.is_some_and(guest)
}

/// An agent's or integration's owner as the rules need to know them: their role in the
/// workspace, and the resource acted on as they stand to it, with their own teamspace
/// role, shares and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerStanding {
    /// `None` once they are out of the workspace.
    pub role: Option<WorkspaceRole>,
    /// The same resource the actor acts on, of the same kind, as the owner stands to it.
    pub resource: Resource,
}

/// What an action is done to, with what the rules need to know of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// The instance, for creating workspaces and the instance admins' work.
    Instance { creation: WorkspaceCreation },
    /// A workspace as a whole.
    Workspace { members_can_create_teamspaces: bool },
    /// One member of the workspace: their role, what they are, and whether they are the
    /// actor.
    Member {
        role: WorkspaceRole,
        kind: UserKind,
        is_actor: bool,
    },
    /// A teamspace of the workspace, and the actor's role in it, if any.
    Teamspace {
        access: TeamspaceAccess,
        membership: Option<TeamspaceRole>,
    },
    /// The top of a space's pages, where the tree is listed from and pages are added.
    Pages { space: PageSpace },
    /// One page: its space, the most that it or a page above it is shared with the actor
    /// for, and whether it is in the trash.
    Page {
        space: PageSpace,
        shared: Option<PageAccess>,
        trashed: bool,
    },
    /// The workspace's agents as a whole, for making them: whether agents are on, on the
    /// instance and in the workspace, and who makes them there.
    Agents {
        instance: bool,
        workspace: bool,
        creation: AgentCreation,
    },
    /// One agent or integration of the workspace, and whether the actor owns it.
    Agent { kind: UserKind, own: bool },
    /// The outgoing webhooks of one identity of the workspace, as the actor stands to it.
    Webhook { subject: WebhookSubject },
}

/// Whose events an outgoing webhook delivers, as the actor stands to that identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookSubject {
    /// The actor themself.
    Itself,
    /// An agent or integration the actor owns.
    OwnDelegate,
    /// Anyone else of the workspace: another person, or someone else's agent or
    /// integration.
    Other,
}

/// Someone added to a teamspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newcomer {
    /// A person, whose role in the workspace is this.
    Person(WorkspaceRole),
    /// An agent or integration; `own` when the actor owns it.
    Delegate { own: bool },
}

/// Where pages are, as the rules need to know of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSpace {
    /// A teamspace, and the actor's role in it, if any.
    Teamspace {
        access: TeamspaceAccess,
        membership: Option<TeamspaceRole>,
    },
    /// A member's private pages; `own` when that member is the actor.
    Private { own: bool },
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
    /// Adding a member of the workspace: a person, or an agent or integration, which its
    /// owner adds only to teamspaces they are in themselves.
    AddTeamspaceMember {
        newcomer: Newcomer,
    },
    RemoveTeamspaceMember,
    ChangeTeamspaceRole,

    // On a space's pages.
    /// The tree of its pages.
    ListPages,
    /// Adding a page at its top, new or moved there.
    CreatePage,

    // On a page.
    /// Its title, icon, cover, style, content and place, and the pages under it.
    ViewPage,
    /// Changing what it holds, adding subpages under it, and moving it within its space,
    /// or another page under it.
    EditPage,
    /// Moving it, with the pages under it, out of its space: to another teamspace, or to
    /// private pages. For those its space lets edit it, whatever is shared with them.
    ChangePageSpace,
    /// Moving it to the trash, with its subpages.
    TrashPage,
    RestorePage,
    /// Deleting it for good, from the trash.
    DeletePage,
    /// Who it is shared with, on it and on the pages above it.
    ListShares,
    /// Sharing it with a member or guest, changing what a share gives, or removing one.
    SharePage,

    // On the workspace's agents.
    /// Making an agent, which the maker owns.
    CreateAgent,

    // On an agent or integration.
    /// Its name, owner and status.
    ViewAgent,
    /// Its name and hue, which only its owner changes.
    UpdateAgent,
    /// Handing it to another person of the workspace, who owns it from then on.
    TransferAgent,
    DeleteAgent,
    /// Its tokens, without their secrets.
    ListAgentTokens,
    /// From a browser session, by its owner alone.
    CreateAgentToken,
    /// From a browser session.
    RevokeAgentToken,

    // On the workspace's incoming webhooks, on the workspace as a whole.
    /// Listing, making, changing and deleting incoming webhooks, each of which acts as an
    /// integration its maker owns.
    ManageIncomingWebhooks,

    // On an identity's outgoing webhooks.
    /// Making an outgoing webhook that delivers the identity's events.
    RegisterWebhook,
    /// Reading, changing, testing and deleting one, with its delivery log.
    ManageWebhook,
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
    match actor.on_behalf_of {
        None => decide(actor, action, resource),
        Some(owner) => on_behalf(actor, owner, action, resource),
    }
}

/// Whether a person with `role` in the workspace may see a page out of the trash in
/// `space`, shared with them, on it or a page above it, for `shared`: what [`authorize`]
/// answers a person for [`Action::ViewPage`].
///
/// Queries that list the pages someone may see among many, such as search, filter them in
/// the database rather than one at a time, and they lean on what the tests prove of this:
///
/// - A page shows when its space alone shows it (`shared` being `None`), or when it is
///   shared with them at all and a share alone shows a page: what the space gives and
///   what a share gives add up, and any share shows what any other does.
/// - An agent or integration sees a page when it sees it and its owner does, each as a
///   person with their own role, standing to the page as each does.
///
/// So such a query asks this of each teamspace, of the person's own private pages and of
/// shares once, for the actor and for an agent's owner, and keeps the pages both see.
pub fn person_sees_page(
    role: Option<WorkspaceRole>,
    space: PageSpace,
    shared: Option<PageAccess>,
) -> bool {
    decide(
        Actor::person(false, false, role),
        Action::ViewPage,
        Resource::Page {
            space,
            shared,
            trashed: false,
        },
    )
    .is_ok()
}

const DELEGATES_MANAGE_NOTHING: Denied = Forbidden(
    "agents and integrations work in the workspace and never manage it, its members, its \
     teamspaces or its agents",
);

const OWNER_MAY_NOT: Denied = Forbidden(
    "an agent or integration does only what its owner may, and its owner may not do this",
);

/// What an agent's owner must be allowed for the agent to do an action.
enum OwnerCheck {
    /// The same action, on the resource as they stand to it.
    Same,
    /// Another action instead.
    Instead(Action),
    /// Only that they are in the workspace.
    InWorkspace,
}

fn owner_check(action: Action) -> OwnerCheck {
    match action {
        // To join a teamspace, the owner must see it in full.
        Action::JoinTeamspace => OwnerCheck::Instead(Action::ViewTeamspace),
        // Leaving one only narrows what the agent may do.
        Action::LeaveTeamspace => OwnerCheck::InWorkspace,
        _ => OwnerCheck::Same,
    }
}

/// What agents and integrations never do, whatever their owner may.
fn delegates_never(action: Action) -> bool {
    matches!(
        action,
        Action::UpdateWorkspace
            | Action::DeleteWorkspace
            | Action::Leave
            | Action::Invite { .. }
            | Action::ManageInvitations
            | Action::CreateInviteLink { .. }
            | Action::ManageInviteLinks
            | Action::CreateTeamspace
            | Action::ChangeRole { .. }
            | Action::RemoveMember
            | Action::TransferOwnership
            | Action::UpdateTeamspace
            | Action::ChangeTeamspaceAccess { .. }
            | Action::DeleteTeamspace
            | Action::AddTeamspaceMember { .. }
            | Action::RemoveTeamspaceMember
            | Action::ChangeTeamspaceRole
            | Action::CreateAgent
            | Action::UpdateAgent
            | Action::TransferAgent
            | Action::DeleteAgent
            | Action::ListAgentTokens
            | Action::CreateAgentToken
            | Action::RevokeAgentToken
            | Action::ManageIncomingWebhooks
            | Action::RegisterWebhook
            | Action::ManageWebhook
    )
}

/// An agent or integration acting for `owner`: what it may do as a member itself, and
/// what its owner may, both. Hidden from either stays hidden, so that what an agent may not
/// do never tells it what it may not know.
fn on_behalf(
    actor: Actor,
    owner: OwnerStanding,
    action: Action,
    resource: Resource,
) -> Result<(), Denied> {
    if matches!(resource, Resource::Instance { .. }) {
        return Err(Forbidden(
            "an agent or integration works inside its own workspace only",
        ));
    }
    if std::mem::discriminant(&resource) != std::mem::discriminant(&owner.resource) {
        return Err(NOT_APPLICABLE);
    }
    // Either out of the workspace, it may know nothing of it.
    let (Some(_), Some(owner_role)) = (actor.role, owner.role) else {
        return Err(Hidden);
    };
    if let Resource::Pages {
        space: PageSpace::Private { own: true },
    } = resource
    {
        return Err(Forbidden(
            "an agent or integration has no private pages: its owner could not see them",
        ));
    }
    let itself = Actor::person(false, actor.via_session, actor.role);
    let mine = decide(itself, action, resource);
    let them = Actor::person(false, actor.via_session, Some(owner_role));
    let theirs = match owner_check(action) {
        OwnerCheck::Same => decide(them, action, owner.resource),
        OwnerCheck::Instead(instead) => decide(them, instead, owner.resource),
        OwnerCheck::InWorkspace => Ok(()),
    }
    .map_err(|denied| match denied {
        Hidden => Hidden,
        Forbidden(_) => OWNER_MAY_NOT,
    });
    let both = match (mine, theirs) {
        (Err(Hidden), _) | (_, Err(Hidden)) => Err(Hidden),
        (Err(denied), _) | (_, Err(denied)) => Err(denied),
        (Ok(()), Ok(())) => Ok(()),
    };
    if delegates_never(action) && both != Err(Hidden) {
        Err(DELEGATES_MANAGE_NOTHING)
    } else {
        both
    }
}

/// The rules for a person, or for an agent's or integration's own part.
fn decide(actor: Actor, action: Action, resource: Resource) -> Result<(), Denied> {
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
        Resource::Member {
            role,
            kind,
            is_actor,
        } => on_member(
            member(actor)?,
            actor.via_session,
            action,
            role,
            kind,
            is_actor,
        ),
        Resource::Teamspace { access, membership } => {
            on_teamspace(member(actor)?, action, access, membership)
        }
        Resource::Pages { space } => on_pages(member(actor)?, action, space),
        Resource::Page {
            space,
            shared,
            trashed,
        } => on_page(member(actor)?, action, space, shared, trashed),
        Resource::Agents {
            instance,
            workspace,
            creation,
        } => on_agents(member(actor)?, action, instance, workspace, creation),
        Resource::Agent { kind, own } => {
            on_agent(member(actor)?, actor.via_session, action, kind, own)
        }
        Resource::Webhook { subject } => on_webhook(member(actor)?, action, subject),
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
        Action::UpdateWorkspace
        | Action::ManageInvitations
        | Action::ManageInviteLinks
        | Action::ManageIncomingWebhooks => allow_if(role.manages(), MANAGERS_ONLY),
        Action::DeleteWorkspace => {
            allow_if(
                role == Owner,
                Forbidden("only an owner may delete the workspace"),
            )?;
            needs_session(via_session, DELETE_NEEDS_SESSION)
        }
        Action::ListMembers => allow_if(role != Guest, GUESTS_SEE_NO_MEMBER),
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
    kind: UserKind,
    is_actor: bool,
) -> Result<(), Denied> {
    on_person(role, via_session, action, target, is_actor)?;
    if kind.is_human() {
        return Ok(());
    }
    match action {
        Action::ChangeRole { .. } => Err(Forbidden(
            "an agent's or integration's role in its workspace is always member",
        )),
        Action::RemoveMember => Err(Forbidden(
            "an agent or integration is deleted, not removed: it belongs to this workspace \
             alone",
        )),
        Action::TransferOwnership => Err(Forbidden("ownership goes to a person")),
        _ => Ok(()),
    }
}

/// The rules on a member as a person, whatever they are.
fn on_person(
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
        Action::AddTeamspaceMember { newcomer } => match (access, newcomer) {
            (TeamspaceAccess::Default, _) => Err(IN_THE_DEFAULT),
            (_, Newcomer::Delegate { own: true }) => allow_if(
                membership.is_some(),
                Forbidden("you add your agent only to teamspaces you are in yourself"),
            ),
            _ if !owns => Err(OWNERS_ONLY),
            (_, Newcomer::Delegate { own: false }) => Ok(()),
            (_, Newcomer::Person(workspace_role)) => allow_if(
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

const GUESTS_SEE_NO_MEMBER: Denied = Forbidden("guests see no member but themselves");

/// What a space's pages give a role, before any share.
fn space_access(role: WorkspaceRole, space: PageSpace) -> Option<PageAccess> {
    // Guests reach pages only through shares.
    if !role.joins_teamspaces() {
        return None;
    }
    match space {
        PageSpace::Teamspace {
            membership: Some(_),
            ..
        } => Some(PageAccess::Edit),
        PageSpace::Teamspace {
            access: TeamspaceAccess::Private,
            membership: None,
        } => None,
        PageSpace::Teamspace {
            membership: None, ..
        } => Some(PageAccess::View),
        PageSpace::Private { own } => own.then_some(PageAccess::Edit),
    }
}

fn on_pages(role: WorkspaceRole, action: Action, space: PageSpace) -> Result<(), Denied> {
    match space {
        PageSpace::Private { own: false } => Err(Hidden),
        PageSpace::Private { own: true } => {
            if !role.joins_teamspaces() {
                return Err(Forbidden(
                    "guests have no private pages: they see the pages shared with them",
                ));
            }
            match action {
                Action::ListPages | Action::CreatePage => Ok(()),
                _ => Err(NOT_APPLICABLE),
            }
        }
        PageSpace::Teamspace { access, membership } => {
            // As for the teamspace itself: hidden from guests, and a private one from those
            // outside it.
            if !role.joins_teamspaces()
                || (access == TeamspaceAccess::Private && membership.is_none())
            {
                return Err(Hidden);
            }
            match action {
                Action::ListPages => Ok(()),
                Action::CreatePage => allow_if(
                    membership.is_some(),
                    Forbidden("only the teamspace's members add pages to it: join it first"),
                ),
                _ => Err(NOT_APPLICABLE),
            }
        }
    }
}

fn on_page(
    role: WorkspaceRole,
    action: Action,
    space: PageSpace,
    shared: Option<PageAccess>,
    trashed: bool,
) -> Result<(), Denied> {
    const VIEW_ONLY: Denied = Forbidden("you may view this page, not edit it");
    const IN_THE_TRASH: Denied = Forbidden("this page is in the trash: restore it first");

    let access = space_access(role, space).max(shared);
    let edits = access == Some(PageAccess::Edit);
    // A page in the trash shows only to those who could edit it.
    if access.is_none() || (trashed && !edits) {
        return Err(Hidden);
    }
    let guest = !role.joins_teamspaces();
    match action {
        Action::ViewPage => Ok(()),
        Action::EditPage | Action::TrashPage => {
            if trashed {
                Err(IN_THE_TRASH)
            } else {
                allow_if(edits, VIEW_ONLY)
            }
        }
        Action::ChangePageSpace => {
            if trashed {
                Err(IN_THE_TRASH)
            } else if !edits {
                Err(VIEW_ONLY)
            } else {
                allow_if(
                    space_access(role, space) == Some(PageAccess::Edit),
                    Forbidden(
                        "a page shared with you moves only within its space: those in its \
                         teamspace, or its owner, may move it elsewhere",
                    ),
                )
            }
        }
        Action::RestorePage => allow_if(trashed, Forbidden("this page is not in the trash")),
        Action::DeletePage => allow_if(
            trashed,
            Forbidden("only a page in the trash is deleted for good: trash it first"),
        ),
        Action::ListShares => allow_if(!guest, GUESTS_SEE_NO_MEMBER),
        Action::SharePage => {
            if guest {
                Err(GUESTS_SEE_NO_MEMBER)
            } else if trashed {
                Err(IN_THE_TRASH)
            } else {
                allow_if(edits, VIEW_ONLY)
            }
        }
        _ => Err(NOT_APPLICABLE),
    }
}

fn on_agents(
    role: WorkspaceRole,
    action: Action,
    instance: bool,
    workspace: bool,
    creation: AgentCreation,
) -> Result<(), Denied> {
    match action {
        Action::CreateAgent => {
            if !role.joins_teamspaces() {
                Err(Forbidden("guests make no agents"))
            } else if !instance {
                Err(Forbidden("agents are turned off on this instance"))
            } else if !workspace {
                Err(Forbidden("agents are turned off in this workspace"))
            } else {
                allow_if(
                    creation == AgentCreation::Members || role.manages(),
                    Forbidden("in this workspace only owners and admins make agents"),
                )
            }
        }
        _ => Err(NOT_APPLICABLE),
    }
}

fn on_agent(
    role: WorkspaceRole,
    via_session: bool,
    action: Action,
    kind: UserKind,
    own: bool,
) -> Result<(), Denied> {
    const OWNER_OR_MANAGERS: Denied =
        Forbidden("only its owner, and the workspace's owners and admins, may do this");
    const TOKENS_NEED_SESSION: &str =
        "managing an agent's tokens needs a browser session: API tokens cannot";

    // Guests know of no member, agents included, theirs too.
    if !role.joins_teamspaces() {
        return Err(Hidden);
    }
    match action {
        Action::ViewAgent => Ok(()),
        Action::UpdateAgent => allow_if(own, Forbidden("only its owner renames or recolours it")),
        Action::TransferAgent => allow_if(
            role.manages(),
            Forbidden("only the workspace's owners and admins hand an agent to someone else"),
        ),
        Action::DeleteAgent | Action::ListAgentTokens => {
            allow_if(own || role.manages(), OWNER_OR_MANAGERS)
        }
        Action::CreateAgentToken => {
            if kind == UserKind::Integration {
                Err(Forbidden(
                    "an integration works through its webhook, and has no tokens",
                ))
            } else if !own {
                Err(Forbidden("only its owner makes an agent's tokens"))
            } else {
                needs_session(via_session, TOKENS_NEED_SESSION)
            }
        }
        Action::RevokeAgentToken => {
            allow_if(own || role.manages(), OWNER_OR_MANAGERS)?;
            needs_session(via_session, TOKENS_NEED_SESSION)
        }
        _ => Err(NOT_APPLICABLE),
    }
}

/// An identity's outgoing webhooks: the workspace's owners and admins register and manage
/// anyone's; everyone else but guests their own and their agents' and integrations'. A
/// guest registers none, and knows of none.
fn on_webhook(role: WorkspaceRole, action: Action, subject: WebhookSubject) -> Result<(), Denied> {
    let mine = matches!(
        subject,
        WebhookSubject::Itself | WebhookSubject::OwnDelegate
    );
    match action {
        Action::RegisterWebhook => {
            if role.manages() {
                Ok(())
            } else if !role.joins_teamspaces() {
                if mine {
                    Err(Forbidden("guests register no webhooks"))
                } else {
                    Err(Hidden)
                }
            } else {
                allow_if(
                    mine,
                    Forbidden(
                        "only the workspace's owners and admins register webhooks for others",
                    ),
                )
            }
        }
        Action::ManageWebhook => {
            if role.manages() || (mine && role.joins_teamspaces()) {
                Ok(())
            } else {
                Err(Hidden)
            }
        }
        _ => Err(NOT_APPLICABLE),
    }
}

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
mod delegate_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod visibility_tests;
