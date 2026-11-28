//! Every actor, action and resource, against tables written out by hand.
//!
//! Each table has a row per resource and a column per workspace role: owner, admin,
//! member, guest. `y` is allowed, `f` forbidden, `h` hidden. A page's tables have a row per
//! access the actor ends up with on it, live and in the trash; [`SPACE_ACCESS`] says what
//! each space gives each role before shares, and the tests try every share on every
//! space. The tables are what a browser
//! session gets; an API token gets the same but for what only a session may do, which it
//! is forbidden. Outside the tables, every test also checks that someone outside the
//! workspace is told nothing, whether or not they are an instance admin, and that being an
//! instance admin changes nothing inside a workspace.

use super::*;
use crate::agents::AgentCreation;
use crate::auth::UserKind;
use crate::pages::PageAccess;

const ROLES: [WorkspaceRole; 4] = [
    WorkspaceRole::Owner,
    WorkspaceRole::Admin,
    WorkspaceRole::Member,
    WorkspaceRole::Guest,
];

fn outcome(result: Result<(), Denied>) -> char {
    match result {
        Ok(()) => 'y',
        Err(Denied::Hidden) => 'h',
        Err(Denied::Forbidden(_)) => 'f',
    }
}

/// The four expected outcomes at the end of a row, by role.
fn expected(cells: &[&str]) -> [char; 4] {
    assert_eq!(cells.len(), 4, "a row ends with four cells: {cells:?}");
    let mut out = ['?'; 4];
    for (slot, cell) in out.iter_mut().zip(cells) {
        let mut chars = cell.chars();
        let (Some(c @ ('y' | 'f' | 'h')), None) = (chars.next(), chars.next()) else {
            panic!("a cell is y, f or h: {cell:?}");
        };
        *slot = c;
    }
    out
}

/// What only a browser session may do: what cannot be undone, and managing tokens.
fn session_only(action: Action) -> bool {
    matches!(
        action,
        Action::DeleteWorkspace
            | Action::TransferOwnership
            | Action::DeleteAnyWorkspace
            | Action::CreateAgentToken
            | Action::RevokeAgentToken
    )
}

/// The outcome for an API token of an action whose outcome for a session is `want`.
fn by_token(action: Action, want: char) -> char {
    if session_only(action) && want == 'y' {
        'f'
    } else {
        want
    }
}

fn rows(table: &str) -> Vec<Vec<&str>> {
    table
        .lines()
        .map(str::split_whitespace)
        .map(Iterator::collect::<Vec<_>>)
        .filter(|cells| !cells.is_empty())
        .collect()
}

/// Checks every role, instance admin or not, at a browser or with a token, and someone
/// outside the workspace, on `resource`.
fn check_members(action: Action, resource: Resource, want: [char; 4], row: &str) {
    for instance_admin in [false, true] {
        for via_session in [true, false] {
            for (role, want) in ROLES.into_iter().zip(want) {
                let actor = Actor {
                    instance_admin,
                    via_session,
                    role: Some(role),
                    on_behalf_of: None,
                };
                let want = if via_session {
                    want
                } else {
                    by_token(action, want)
                };
                assert_eq!(
                    outcome(authorize(actor, action, resource)),
                    want,
                    "{action:?} on {row} by {role} (instance admin: {instance_admin}, \
                     session: {via_session})"
                );
            }
            let outsider = Actor {
                instance_admin,
                via_session,
                role: None,
                on_behalf_of: None,
            };
            assert_eq!(
                authorize(outsider, action, resource),
                Err(Denied::Hidden),
                "{action:?} on {row} by someone outside (instance admin: {instance_admin}, \
                 session: {via_session})"
            );
        }
    }
}

/// Rows `open`, where members may create teamspaces, and `locked`, where they may not.
fn workspace_table(action: Action, table: &str) {
    let rows = rows(table);
    let labels: Vec<&str> = rows.iter().map(|row| row[0]).collect();
    assert_eq!(labels, ["open", "locked"], "{action:?}");
    for row in rows {
        let resource = Resource::Workspace {
            members_can_create_teamspaces: row[0] == "open",
        };
        check_members(action, resource, expected(&row[1..]), row[0]);
    }
}

/// Rows for a person of each role who is not the actor, an agent and an integration, whose
/// role is always member, then `self`, the actor.
fn member_table(action: Action, table: &str) {
    let rows = rows(table);
    let labels: Vec<&str> = rows.iter().map(|row| row[0]).collect();
    assert_eq!(
        labels,
        [
            "owner",
            "admin",
            "member",
            "guest",
            "agent",
            "integration",
            "self"
        ],
        "{action:?}"
    );
    for row in &rows[..6] {
        let (role, kind) = match UserKind::parse(row[0]) {
            Some(kind) => (WorkspaceRole::Member, kind),
            None => (
                WorkspaceRole::parse(row[0]).expect("a role"),
                UserKind::Human,
            ),
        };
        let resource = Resource::Member {
            role,
            kind,
            is_actor: false,
        };
        check_members(action, resource, expected(&row[1..]), row[0]);
    }
    // The actor is a member of their own role.
    let want = expected(&rows[6][1..]);
    for instance_admin in [false, true] {
        for via_session in [true, false] {
            for (role, want) in ROLES.into_iter().zip(want) {
                let actor = Actor {
                    instance_admin,
                    via_session,
                    role: Some(role),
                    on_behalf_of: None,
                };
                let resource = Resource::Member {
                    role,
                    kind: UserKind::Human,
                    is_actor: true,
                };
                let want = if via_session {
                    want
                } else {
                    by_token(action, want)
                };
                assert_eq!(
                    outcome(authorize(actor, action, resource)),
                    want,
                    "{action:?} on themselves by {role} (session: {via_session})"
                );
            }
        }
    }
}

/// A row per access and the actor's role in the teamspace, `-` for none: all twelve.
fn teamspace_table(action: Action, table: &str) {
    let rows = rows(table);
    let mut seen = Vec::new();
    for row in &rows {
        let access = TeamspaceAccess::parse(row[0]).expect("an access");
        let membership = match row[1] {
            "-" => None,
            role => Some(TeamspaceRole::parse(role).expect("a teamspace role")),
        };
        seen.push((access, membership));
        let label = format!("{} teamspace, actor {}", row[0], row[1]);
        check_members(
            action,
            Resource::Teamspace { access, membership },
            expected(&row[2..]),
            &label,
        );
    }
    let mut all = Vec::new();
    for access in TeamspaceAccess::ALL {
        for membership in [
            None,
            Some(TeamspaceRole::Member),
            Some(TeamspaceRole::Owner),
        ] {
            all.push((*access, membership));
        }
    }
    assert_eq!(
        seen, all,
        "{action:?}: one row for each access and membership, in order"
    );
}

/// Every action, with every value of its parameter. The match in [`kind`] has no
/// wildcard, so a new action does not compile until it is placed here and in a table.
fn all_actions() -> Vec<Action> {
    let mut actions = vec![
        Action::CreateWorkspace,
        Action::ListAllWorkspaces,
        Action::SetLimits,
        Action::DeleteAnyWorkspace,
        Action::ViewWorkspace,
        Action::UpdateWorkspace,
        Action::DeleteWorkspace,
        Action::ListMembers,
        Action::Leave,
        Action::ManageInvitations,
        Action::ManageInviteLinks,
        Action::CreateTeamspace,
        Action::ViewMember,
        Action::RemoveMember,
        Action::TransferOwnership,
        Action::ViewTeamspace,
        Action::SeeTeamspace,
        Action::JoinTeamspace,
        Action::LeaveTeamspace,
        Action::UpdateTeamspace,
        Action::DeleteTeamspace,
        Action::ListTeamspaceMembers,
        Action::RemoveTeamspaceMember,
        Action::ChangeTeamspaceRole,
        Action::ListPages,
        Action::CreatePage,
        Action::ViewPage,
        Action::EditPage,
        Action::ChangePageSpace,
        Action::TrashPage,
        Action::CommentPage,
        Action::RestorePage,
        Action::DeletePage,
        Action::ListShares,
        Action::SharePage,
        Action::CreateAgent,
        Action::ViewAgent,
        Action::UpdateAgent,
        Action::TransferAgent,
        Action::DeleteAgent,
        Action::ListAgentTokens,
        Action::CreateAgentToken,
        Action::RevokeAgentToken,
        Action::ManageIncomingWebhooks,
        Action::RegisterWebhook,
        Action::ManageWebhook,
    ];
    for role in ROLES {
        actions.push(Action::Invite { role });
        actions.push(Action::CreateInviteLink { role });
        actions.push(Action::ChangeRole { to: role });
    }
    for newcomer in all_newcomers() {
        actions.push(Action::AddTeamspaceMember { newcomer });
    }
    for access in TeamspaceAccess::ALL {
        actions.push(Action::ChangeTeamspaceAccess { to: *access });
    }
    actions
}

/// Everyone who may be added to a teamspace: a person of each role, and an agent the
/// actor owns or not.
fn all_newcomers() -> Vec<Newcomer> {
    ROLES
        .into_iter()
        .map(Newcomer::Person)
        .chain([
            Newcomer::Delegate { own: false },
            Newcomer::Delegate { own: true },
        ])
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Instance,
    Workspace,
    Member,
    Teamspace,
    Pages,
    Page,
    Agents,
    Agent,
    Webhook,
}

fn kind(action: Action) -> Kind {
    match action {
        Action::CreateWorkspace
        | Action::ListAllWorkspaces
        | Action::SetLimits
        | Action::DeleteAnyWorkspace => Kind::Instance,
        Action::ViewWorkspace
        | Action::UpdateWorkspace
        | Action::DeleteWorkspace
        | Action::ListMembers
        | Action::Leave
        | Action::Invite { .. }
        | Action::ManageInvitations
        | Action::CreateInviteLink { .. }
        | Action::ManageInviteLinks
        | Action::CreateTeamspace
        | Action::ManageIncomingWebhooks => Kind::Workspace,
        Action::ViewMember
        | Action::ChangeRole { .. }
        | Action::RemoveMember
        | Action::TransferOwnership => Kind::Member,
        Action::ViewTeamspace
        | Action::SeeTeamspace
        | Action::JoinTeamspace
        | Action::LeaveTeamspace
        | Action::UpdateTeamspace
        | Action::ChangeTeamspaceAccess { .. }
        | Action::DeleteTeamspace
        | Action::ListTeamspaceMembers
        | Action::AddTeamspaceMember { .. }
        | Action::RemoveTeamspaceMember
        | Action::ChangeTeamspaceRole => Kind::Teamspace,
        Action::ListPages | Action::CreatePage => Kind::Pages,
        Action::ViewPage
        | Action::EditPage
        | Action::ChangePageSpace
        | Action::TrashPage
        | Action::CommentPage
        | Action::RestorePage
        | Action::DeletePage
        | Action::ListShares
        | Action::SharePage => Kind::Page,
        Action::CreateAgent => Kind::Agents,
        Action::ViewAgent
        | Action::UpdateAgent
        | Action::TransferAgent
        | Action::DeleteAgent
        | Action::ListAgentTokens
        | Action::CreateAgentToken
        | Action::RevokeAgentToken => Kind::Agent,
        Action::RegisterWebhook | Action::ManageWebhook => Kind::Webhook,
    }
}

/// One of each resource of every kind.
fn all_resources() -> Vec<Resource> {
    let mut resources = Vec::new();
    for creation in WorkspaceCreation::ALL {
        resources.push(Resource::Instance {
            creation: *creation,
        });
    }
    for members_can_create_teamspaces in [true, false] {
        resources.push(Resource::Workspace {
            members_can_create_teamspaces,
        });
    }
    for role in ROLES {
        for kind in UserKind::ALL {
            for is_actor in [false, true] {
                resources.push(Resource::Member {
                    role,
                    kind: *kind,
                    is_actor,
                });
            }
        }
    }
    for access in TeamspaceAccess::ALL {
        for membership in [
            None,
            Some(TeamspaceRole::Member),
            Some(TeamspaceRole::Owner),
        ] {
            resources.push(Resource::Teamspace {
                access: *access,
                membership,
            });
        }
    }
    for space in all_page_spaces() {
        resources.push(Resource::Pages { space });
        for shared in SHARES {
            for trashed in [false, true] {
                resources.push(Resource::Page {
                    space,
                    shared,
                    trashed,
                });
            }
        }
    }
    resources.extend(all_agents_resources());
    for kind in [UserKind::Agent, UserKind::Integration] {
        for own in [false, true] {
            resources.push(Resource::Agent { kind, own });
        }
    }
    for subject in WEBHOOK_SUBJECTS {
        resources.push(Resource::Webhook { subject });
    }
    resources
}

const WEBHOOK_SUBJECTS: [WebhookSubject; 3] = [
    WebhookSubject::Itself,
    WebhookSubject::OwnDelegate,
    WebhookSubject::Other,
];

/// The workspace's agents with agents on or off, on the instance and in the workspace, and
/// each who makes them.
fn all_agents_resources() -> Vec<Resource> {
    let mut resources = Vec::new();
    for instance in [true, false] {
        for workspace in [true, false] {
            for creation in AgentCreation::ALL {
                resources.push(Resource::Agents {
                    instance,
                    workspace,
                    creation: *creation,
                });
            }
        }
    }
    resources
}

fn resource_kind(resource: Resource) -> Kind {
    match resource {
        Resource::Instance { .. } => Kind::Instance,
        Resource::Workspace { .. } => Kind::Workspace,
        Resource::Member { .. } => Kind::Member,
        Resource::Teamspace { .. } => Kind::Teamspace,
        Resource::Pages { .. } => Kind::Pages,
        Resource::Page { .. } => Kind::Page,
        Resource::Agents { .. } => Kind::Agents,
        Resource::Agent { .. } => Kind::Agent,
        Resource::Webhook { .. } => Kind::Webhook,
    }
}

/// Every action's table, by the kind of resource it applies to.
fn tables() -> Vec<(Action, &'static str)> {
    use TeamspaceAccess as T;
    use WorkspaceRole as R;

    vec![
        // Workspace actions.
        (Action::ViewWorkspace, "open y y y y \n locked y y y y"),
        (Action::Leave, "open y y y y \n locked y y y y"),
        (Action::UpdateWorkspace, "open y y f f \n locked y y f f"),
        (Action::DeleteWorkspace, "open y f f f \n locked y f f f"),
        (Action::ListMembers, "open y y y f \n locked y y y f"),
        (Action::ManageInvitations, "open y y f f \n locked y y f f"),
        (Action::ManageInviteLinks, "open y y f f \n locked y y f f"),
        (Action::CreateTeamspace, "open y y y f \n locked y y f f"),
        (
            Action::ManageIncomingWebhooks,
            "open y y f f \n locked y y f f",
        ),
        (
            Action::Invite { role: R::Owner },
            "open f f f f \n locked f f f f",
        ),
        (
            Action::Invite { role: R::Admin },
            "open y y f f \n locked y y f f",
        ),
        (
            Action::Invite { role: R::Member },
            "open y y f f \n locked y y f f",
        ),
        (
            Action::Invite { role: R::Guest },
            "open y y f f \n locked y y f f",
        ),
        (
            Action::CreateInviteLink { role: R::Owner },
            "open f f f f \n locked f f f f",
        ),
        (
            Action::CreateInviteLink { role: R::Admin },
            "open f f f f \n locked f f f f",
        ),
        (
            Action::CreateInviteLink { role: R::Member },
            "open y y f f \n locked y y f f",
        ),
        (
            Action::CreateInviteLink { role: R::Guest },
            "open y y f f \n locked y y f f",
        ),
        // Member actions.
        (Action::ViewMember, VIEW_MEMBER),
        (Action::ChangeRole { to: R::Owner }, MAKE_OWNER),
        (Action::ChangeRole { to: R::Admin }, CHANGE_ROLE),
        (Action::ChangeRole { to: R::Member }, CHANGE_ROLE),
        (Action::ChangeRole { to: R::Guest }, CHANGE_ROLE),
        (Action::RemoveMember, REMOVE_MEMBER),
        (Action::TransferOwnership, TRANSFER),
        // Teamspace actions.
        (Action::SeeTeamspace, SEE),
        (Action::ViewTeamspace, VIEW),
        (Action::ListTeamspaceMembers, VIEW),
        (Action::JoinTeamspace, JOIN),
        (Action::LeaveTeamspace, LEAVE),
        (Action::UpdateTeamspace, OWNERS),
        (Action::ChangeTeamspaceRole, OWNERS),
        (
            Action::ChangeTeamspaceAccess { to: T::Default },
            NEVER_DEFAULT,
        ),
        (
            Action::ChangeTeamspaceAccess { to: T::Open },
            NOT_THE_DEFAULT,
        ),
        (
            Action::ChangeTeamspaceAccess { to: T::Closed },
            NOT_THE_DEFAULT,
        ),
        (
            Action::ChangeTeamspaceAccess { to: T::Private },
            NOT_THE_DEFAULT,
        ),
        (Action::DeleteTeamspace, DELETE),
        (Action::RemoveTeamspaceMember, NOT_THE_DEFAULT),
        (
            Action::AddTeamspaceMember {
                newcomer: Newcomer::Person(R::Owner),
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                newcomer: Newcomer::Person(R::Admin),
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                newcomer: Newcomer::Person(R::Member),
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                newcomer: Newcomer::Person(R::Guest),
            },
            ADD_A_GUEST,
        ),
        (
            Action::AddTeamspaceMember {
                newcomer: Newcomer::Delegate { own: false },
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                newcomer: Newcomer::Delegate { own: true },
            },
            ADD_OWN_AGENT,
        ),
        // Actions on a space's pages.
        (Action::ListPages, LIST_PAGES),
        (Action::CreatePage, CREATE_PAGE),
        // Actions on a page.
        (Action::ViewPage, VIEW_PAGE),
        (Action::EditPage, EDIT_PAGE),
        (Action::ChangePageSpace, CHANGE_SPACE),
        (Action::TrashPage, EDIT_PAGE),
        (Action::CommentPage, COMMENT_PAGE),
        (Action::RestorePage, FROM_THE_TRASH),
        (Action::DeletePage, FROM_THE_TRASH),
        (Action::ListShares, LIST_SHARES),
        (Action::SharePage, SHARE_PAGE),
        // Making agents.
        (Action::CreateAgent, CREATE_AGENT),
        // Actions on an agent or integration.
        (Action::ViewAgent, VIEW_AGENT),
        (Action::UpdateAgent, OWNER_ALONE),
        (Action::TransferAgent, MANAGERS),
        (Action::DeleteAgent, OWNER_OR_MANAGERS),
        (Action::ListAgentTokens, OWNER_OR_MANAGERS),
        (Action::CreateAgentToken, MAKE_TOKEN),
        (Action::RevokeAgentToken, OWNER_OR_MANAGERS),
        // Actions on an identity's outgoing webhooks.
        (Action::RegisterWebhook, REGISTER_WEBHOOK),
        (Action::ManageWebhook, MANAGE_WEBHOOK),
    ]
}

/// Everyone knows who is in the workspace, agents and integrations too, but guests, who
/// know only themselves.
const VIEW_MEMBER: &str = "
    owner  y y y h
    admin  y y y h
    member y y y h
    guest  y y y h
    agent       y y y h
    integration y y y h
    self   y y y y
";

/// Only owners make owners, of anyone, themselves included.
const MAKE_OWNER: &str = "
    owner  y f f h
    admin  y f f h
    member y f f h
    guest  y f f h
    agent       f f f h
    integration f f f h
    self   y f f f
";

/// Owners change anyone's role; admins anyone's but an owner's, their own too. An agent's
/// or integration's stays member.
const CHANGE_ROLE: &str = "
    owner  y f f h
    admin  y y f h
    member y y f h
    guest  y y f h
    agent       f f f h
    integration f f f h
    self   y y f f
";

/// Nobody removes themselves: they leave. Agents and integrations are deleted instead.
const REMOVE_MEMBER: &str = "
    owner  y f f h
    admin  y y f h
    member y y f h
    guest  y y f h
    agent       f f f h
    integration f f f h
    self   f f f f
";

/// An owner hands ownership to any person who is not an owner yet.
const TRANSFER: &str = "
    owner  f f f h
    admin  y f f h
    member y f f h
    guest  y f f h
    agent       f f f h
    integration f f f h
    self   f f f f
";

/// That a teamspace exists, by name: everyone but guests, except a private one, which
/// members outside it may not know of. The workspace's owners and admins know it by name.
const SEE: &str = "
    default -      y y y h
    default member y y y h
    default owner  y y y h
    open    -      y y y h
    open    member y y y h
    open    owner  y y y h
    closed  -      y y y h
    closed  member y y y h
    closed  owner  y y y h
    private -      y y h h
    private member y y y h
    private owner  y y y h
";

/// Everything about a teamspace, its members included: a private one only for its
/// members, even owners and admins of the workspace seeing its name only.
const VIEW: &str = "
    default -      y y y h
    default member y y y h
    default owner  y y y h
    open    -      y y y h
    open    member y y y h
    open    owner  y y y h
    closed  -      y y y h
    closed  member y y y h
    closed  owner  y y y h
    private -      h h h h
    private member y y y h
    private owner  y y y h
";

/// Open teamspaces take anyone; closed and private ones only those owners and admins add,
/// themselves included. Nobody joins twice, nor the default teamspace.
const JOIN: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      y y y h
    open    member f f f h
    open    owner  f f f h
    closed  -      y y f h
    closed  member f f f h
    closed  owner  f f f h
    private -      y y h h
    private member f f f h
    private owner  f f f h
";

/// Anyone in a teamspace leaves it, but for the default one.
const LEAVE: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      f f f h
    open    member y y y h
    open    owner  y y y h
    closed  -      f f f h
    closed  member y y y h
    closed  owner  y y y h
    private -      h h h h
    private member y y y h
    private owner  y y y h
";

/// The teamspace's owners and the workspace's owners and admins, who manage a private
/// teamspace once they are in it.
const OWNERS: &str = "
    default -      y y f h
    default member y y f h
    default owner  y y y h
    open    -      y y f h
    open    member y y f h
    open    owner  y y y h
    closed  -      y y f h
    closed  member y y f h
    closed  owner  y y y h
    private -      h h h h
    private member y y f h
    private owner  y y y h
";

/// As [`OWNERS`], but never on the default teamspace.
const NOT_THE_DEFAULT: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      y y f h
    open    member y y f h
    open    owner  y y y h
    closed  -      y y f h
    closed  member y y f h
    closed  owner  y y y h
    private -      h h h h
    private member y y f h
    private owner  y y y h
";

/// As [`NOT_THE_DEFAULT`], but the workspace's owners and admins also delete a private
/// teamspace they are not in.
const DELETE: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      y y f h
    open    member y y f h
    open    owner  y y y h
    closed  -      y y f h
    closed  member y y f h
    closed  owner  y y y h
    private -      y y h h
    private member y y f h
    private owner  y y y h
";

/// A guest is in no teamspace, whoever adds them.
const ADD_A_GUEST: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      f f f h
    open    member f f f h
    open    owner  f f f h
    closed  -      f f f h
    closed  member f f f h
    closed  owner  f f f h
    private -      h h h h
    private member f f f h
    private owner  f f f h
";

/// An owner adds their agent to the teamspaces they are in, whatever their role there,
/// and to no other, the default one aside, which every agent is in.
const ADD_OWN_AGENT: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      f f f h
    open    member y y y h
    open    owner  y y y h
    closed  -      f f f h
    closed  member y y y h
    closed  owner  y y y h
    private -      h h h h
    private member y y y h
    private owner  y y y h
";

/// No teamspace becomes the default one.
const NEVER_DEFAULT: &str = "
    default -      f f f h
    default member f f f h
    default owner  f f f h
    open    -      f f f h
    open    member f f f h
    open    owner  f f f h
    closed  -      f f f h
    closed  member f f f h
    closed  owner  f f f h
    private -      h h h h
    private member f f f h
    private owner  f f f h
";

/// Every share a page may have for the actor: none, to view, to comment, to edit.
const SHARES: [Option<PageAccess>; 4] = [
    None,
    Some(PageAccess::View),
    Some(PageAccess::Comment),
    Some(PageAccess::Edit),
];

/// Every space pages are in: a teamspace of each access, the actor outside it, a member or
/// an owner of it, then the actor's own private pages and another member's.
fn all_page_spaces() -> Vec<PageSpace> {
    let mut spaces = Vec::new();
    for access in TeamspaceAccess::ALL {
        for membership in [
            None,
            Some(TeamspaceRole::Member),
            Some(TeamspaceRole::Owner),
        ] {
            spaces.push(PageSpace::Teamspace {
                access: *access,
                membership,
            });
        }
    }
    spaces.push(PageSpace::Private { own: true });
    spaces.push(PageSpace::Private { own: false });
    spaces
}

/// A row per space, in [`all_page_spaces`] order: a teamspace's access and the actor's
/// role in it, `-` for none, or `mine` and `theirs` for private pages. Each row's label and
/// cells.
fn space_rows(table: &str) -> Vec<(PageSpace, String, Vec<&str>)> {
    let parsed: Vec<(PageSpace, String, Vec<&str>)> = rows(table)
        .into_iter()
        .map(|row| match row[0] {
            "mine" => (
                PageSpace::Private { own: true },
                "the actor's private pages".to_owned(),
                row[1..].to_vec(),
            ),
            "theirs" => (
                PageSpace::Private { own: false },
                "another member's private pages".to_owned(),
                row[1..].to_vec(),
            ),
            access => {
                let membership = match row[1] {
                    "-" => None,
                    role => Some(TeamspaceRole::parse(role).expect("a teamspace role")),
                };
                let space = PageSpace::Teamspace {
                    access: TeamspaceAccess::parse(access).expect("an access"),
                    membership,
                };
                (
                    space,
                    format!("{access} teamspace, actor {}", row[1]),
                    row[2..].to_vec(),
                )
            }
        })
        .collect();
    let spaces: Vec<PageSpace> = parsed.iter().map(|(space, _, _)| *space).collect();
    assert_eq!(
        spaces,
        all_page_spaces(),
        "one row for each space, in order"
    );
    parsed
}

/// Rows of [`space_rows`] for an action on a space's pages.
fn pages_table(action: Action, table: &str) {
    for (space, label, cells) in space_rows(table) {
        check_members(action, Resource::Pages { space }, expected(&cells), &label);
    }
}

/// Rows by what the actor's role gets of the page's space before shares, what the page or
/// one above it is shared with them for, `none`, `view`, `comment` or `edit` each, and `live` or
/// `trashed`, then one outcome for every role: for actions that the space's own access
/// decides, whatever is shared. Checked on every space with every share, for each role
/// with what the space gives it.
fn by_space_table(action: Action, table: &str) {
    let access = |cell: &str| match cell {
        "none" => None,
        "view" => Some(PageAccess::View),
        "comment" => Some(PageAccess::Comment),
        "edit" => Some(PageAccess::Edit),
        other => panic!("an access is none, view, comment or edit: {other:?}"),
    };
    let mut outcomes = std::collections::HashMap::new();
    for row in rows(table) {
        let [base, shared, state, cell] = row[..] else {
            panic!("a row is an access, a share, a state and an outcome: {row:?}");
        };
        let trashed = match state {
            "live" => false,
            "trashed" => true,
            other => panic!("a state is live or trashed: {other:?}"),
        };
        let want = expected(&[cell, cell, cell, cell])[0];
        let earlier = outcomes.insert((access(base), access(shared), trashed), want);
        assert!(earlier.is_none(), "one row for each case: {row:?}");
    }
    assert_eq!(
        outcomes.len(),
        24,
        "a row for every access, share and state"
    );
    let mut used = std::collections::HashSet::new();
    for (space, by_role) in space_access_table() {
        for shared in SHARES {
            for trashed in [false, true] {
                let want = by_role.map(|base| {
                    used.insert((base, shared, trashed));
                    outcomes[&(base, shared, trashed)]
                });
                let label = format!("{space:?}, shared {shared:?}, trashed {trashed}");
                check_members(
                    action,
                    Resource::Page {
                        space,
                        shared,
                        trashed,
                    },
                    want,
                    &label,
                );
            }
        }
    }
    assert_eq!(
        used.len(),
        outcomes.len(),
        "every row is a case that happens"
    );
}

/// What each space gives each role before shares: `e` edit, `v` view, `-` nothing.
fn space_access_table() -> Vec<(PageSpace, [Option<PageAccess>; 4])> {
    space_rows(SPACE_ACCESS)
        .into_iter()
        .map(|(space, _, cells)| {
            assert_eq!(cells.len(), 4, "{cells:?}");
            let mut access = [None; 4];
            for (slot, cell) in access.iter_mut().zip(cells) {
                *slot = match cell {
                    "e" => Some(PageAccess::Edit),
                    "v" => Some(PageAccess::View),
                    "-" => None,
                    other => panic!("a cell is e, v or -: {other:?}"),
                };
            }
            (space, access)
        })
        .collect()
}

/// Rows `none`, `view`, `comment` and `edit`, the access the actor ends up with, each
/// `live` and `trashed`: what each role gets on a page. Checked on every space with every
/// share.
fn page_table(action: Action, table: &str) {
    let rows = rows(table);
    let labels: Vec<String> = rows
        .iter()
        .map(|row| format!("{} {}", row[0], row[1]))
        .collect();
    assert_eq!(
        labels,
        [
            "none live",
            "none trashed",
            "view live",
            "view trashed",
            "comment live",
            "comment trashed",
            "edit live",
            "edit trashed"
        ],
        "{action:?}"
    );
    let outcome_for = |access: Option<PageAccess>, trashed: bool| {
        let index = match access {
            None => 0,
            Some(PageAccess::View) => 2,
            Some(PageAccess::Comment) => 4,
            Some(PageAccess::Edit) => 6,
        } + usize::from(trashed);
        expected(&rows[index][2..])
    };
    for (space, by_role) in space_access_table() {
        for shared in SHARES {
            for trashed in [false, true] {
                let mut want = ['?'; 4];
                for (i, base) in by_role.into_iter().enumerate() {
                    want[i] = outcome_for(base.max(shared), trashed)[i];
                }
                let label = format!("{space:?}, shared {shared:?}, trashed {trashed}");
                check_members(
                    action,
                    Resource::Page {
                        space,
                        shared,
                        trashed,
                    },
                    want,
                    &label,
                );
            }
        }
    }
}

/// A teamspace's members edit its pages; everyone else but guests views those of a
/// teamspace they see, which a private one is not. Private pages are their owner's alone.
/// Guests get nothing but their shares.
const SPACE_ACCESS: &str = "
    default -      v v v -
    default member e e e -
    default owner  e e e -
    open    -      v v v -
    open    member e e e -
    open    owner  e e e -
    closed  -      v v v -
    closed  member e e e -
    closed  owner  e e e -
    private -      - - - -
    private member e e e -
    private owner  e e e -
    mine           e e e -
    theirs         - - - -
";

/// The tree of a space: of every teamspace one sees, and one's own private pages, which
/// guests have none of.
const LIST_PAGES: &str = "
    default -      y y y h
    default member y y y h
    default owner  y y y h
    open    -      y y y h
    open    member y y y h
    open    owner  y y y h
    closed  -      y y y h
    closed  member y y y h
    closed  owner  y y y h
    private -      h h h h
    private member y y y h
    private owner  y y y h
    mine           y y y f
    theirs         h h h h
";

/// Pages at the top of a teamspace are added by its members, and private ones by their
/// owner.
const CREATE_PAGE: &str = "
    default -      f f f h
    default member y y y h
    default owner  y y y h
    open    -      f f f h
    open    member y y y h
    open    owner  y y y h
    closed  -      f f f h
    closed  member y y y h
    closed  owner  y y y h
    private -      h h h h
    private member y y y h
    private owner  y y y h
    mine           y y y f
    theirs         h h h h
";

/// Any access shows a live page; only edit access a trashed one.
const VIEW_PAGE: &str = "
    none live       h h h h
    none trashed    h h h h
    view live       y y y y
    view trashed    h h h h
    comment live    y y y y
    comment trashed h h h h
    edit live       y y y y
    edit trashed    y y y y
";

/// Editing, trashing, adding subpages and moving: with edit access, out of the trash.
const EDIT_PAGE: &str = "
    none live       h h h h
    none trashed    h h h h
    view live       f f f f
    view trashed    h h h h
    comment live    f f f f
    comment trashed h h h h
    edit live       y y y y
    edit trashed    f f f f
";

/// Commenting: with comment or edit access, out of the trash, guests too.
const COMMENT_PAGE: &str = "
    none live       h h h h
    none trashed    h h h h
    view live       f f f f
    view trashed    h h h h
    comment live    y y y y
    comment trashed h h h h
    edit live       y y y y
    edit trashed    f f f f
";

/// Moving a page to another teamspace or to private pages: for those its space lets edit
/// it, out of the trash. A share to edit moves it only within its space, which shares to
/// view or comment do not.
const CHANGE_SPACE: &str = "
    none none live       h
    none none trashed    h
    none view live       f
    none view trashed    h
    none comment live    f
    none comment trashed h
    none edit live       f
    none edit trashed    f
    view none live       f
    view none trashed    h
    view view live       f
    view view trashed    h
    view comment live    f
    view comment trashed h
    view edit live       f
    view edit trashed    f
    edit none live       y
    edit none trashed    f
    edit view live       y
    edit view trashed    f
    edit comment live    y
    edit comment trashed f
    edit edit live       y
    edit edit trashed    f
";

/// Restoring and deleting for good: a page in the trash, by those who could edit it.
const FROM_THE_TRASH: &str = "
    none live       h h h h
    none trashed    h h h h
    view live       f f f f
    view trashed    h h h h
    comment live    f f f f
    comment trashed h h h h
    edit live       f f f f
    edit trashed    y y y y
";

/// Who a page is shared with: for everyone who sees it but guests, who know no other
/// member.
const LIST_SHARES: &str = "
    none live       h h h h
    none trashed    h h h h
    view live       y y y f
    view trashed    h h h h
    comment live    y y y f
    comment trashed h h h h
    edit live       y y y f
    edit trashed    y y y f
";

/// Sharing a page: for its editors but guests, out of the trash.
const SHARE_PAGE: &str = "
    none live       h h h h
    none trashed    h h h h
    view live       f f f f
    view trashed    h h h h
    comment live    f f f f
    comment trashed h h h h
    edit live       y y y f
    edit trashed    f f f f
";

/// Rows by whether agents are on, on the instance and in the workspace, and who makes them
/// there: everyone but guests while agents are on, or only owners and admins.
fn agents_table(action: Action, table: &str) {
    let rows = rows(table);
    let mut seen = Vec::new();
    for row in &rows {
        let on = |cell: &str| match cell {
            "on" => true,
            "off" => false,
            other => panic!("agents are on or off: {other:?}"),
        };
        let resource = Resource::Agents {
            instance: on(row[0]),
            workspace: on(row[1]),
            creation: AgentCreation::parse(row[2]).expect("who makes agents"),
        };
        seen.push(resource);
        let label = format!(
            "agents {} on the instance, {} here, made by {}",
            row[0], row[1], row[2]
        );
        check_members(action, resource, expected(&row[3..]), &label);
    }
    assert_eq!(
        seen,
        all_agents_resources(),
        "{action:?}: one row each, in order"
    );
}

/// Rows `agent` and `integration`, each `mine`, owned by the actor, and `theirs`.
fn agent_table(action: Action, table: &str) {
    let rows = rows(table);
    let labels: Vec<String> = rows
        .iter()
        .map(|row| format!("{} {}", row[0], row[1]))
        .collect();
    assert_eq!(
        labels,
        [
            "agent mine",
            "agent theirs",
            "integration mine",
            "integration theirs"
        ],
        "{action:?}"
    );
    for (row, label) in rows.iter().zip(&labels) {
        let resource = Resource::Agent {
            kind: UserKind::parse(row[0]).expect("a kind"),
            own: row[1] == "mine",
        };
        check_members(action, resource, expected(&row[2..]), label);
    }
}

/// Everyone but guests makes agents while agents are on everywhere, unless only owners and
/// admins do there.
const CREATE_AGENT: &str = "
    on  on  admins  y y f f
    on  on  members y y y f
    on  off admins  f f f f
    on  off members f f f f
    off on  admins  f f f f
    off on  members f f f f
    off off admins  f f f f
    off off members f f f f
";

/// Everyone but guests, who know of no member, sees an agent and its owner.
const VIEW_AGENT: &str = "
    agent       mine   y y y h
    agent       theirs y y y h
    integration mine   y y y h
    integration theirs y y y h
";

/// Only its owner renames or recolours it.
const OWNER_ALONE: &str = "
    agent       mine   y y y h
    agent       theirs f f f h
    integration mine   y y y h
    integration theirs f f f h
";

/// The workspace's owners and admins hand any agent to someone else.
const MANAGERS: &str = "
    agent       mine   y y f h
    agent       theirs y y f h
    integration mine   y y f h
    integration theirs y y f h
";

/// Its owner, or the workspace's owners and admins.
const OWNER_OR_MANAGERS: &str = "
    agent       mine   y y y h
    agent       theirs y y f h
    integration mine   y y y h
    integration theirs y y f h
";

/// Only its owner makes an agent's tokens; an integration has none.
const MAKE_TOKEN: &str = "
    agent       mine   y y y h
    agent       theirs f f f h
    integration mine   f f f h
    integration theirs f f f h
";

/// Rows `itself`, `own-delegate`, an agent or integration the actor owns, and `other`.
fn webhook_table(action: Action, table: &str) {
    let rows = rows(table);
    let labels: Vec<&str> = rows.iter().map(|row| row[0]).collect();
    assert_eq!(labels, ["itself", "own-delegate", "other"], "{action:?}");
    for (row, subject) in rows.iter().zip(WEBHOOK_SUBJECTS) {
        check_members(
            action,
            Resource::Webhook { subject },
            expected(&row[1..]),
            row[0],
        );
    }
}

/// The workspace's owners and admins register webhooks for anyone; everyone else for
/// themselves and what they own, but guests, who register none.
const REGISTER_WEBHOOK: &str = "
    itself       y y y f
    own-delegate y y y f
    other        y y f h
";

/// As registering, but those who may not manage a webhook may not know of it either.
const MANAGE_WEBHOOK: &str = "
    itself       y y y h
    own-delegate y y y h
    other        y y h h
";

#[test]
fn every_action_has_exactly_one_table() {
    let actions = all_actions();
    let tables = tables();
    let workspace_and_below = actions
        .iter()
        .filter(|action| kind(**action) != Kind::Instance)
        .count();

    assert_eq!(actions.len(), 68, "every action and parameter is listed");
    assert_eq!(tables.len(), workspace_and_below);
    for action in actions.iter().filter(|a| kind(**a) != Kind::Instance) {
        let found = tables.iter().filter(|(a, _)| a == action).count();
        assert_eq!(found, 1, "{action:?} needs one table");
    }
}

#[test]
fn workspace_actions_follow_their_tables() {
    for (action, table) in tables() {
        match kind(action) {
            Kind::Workspace => workspace_table(action, table),
            Kind::Member => member_table(action, table),
            Kind::Teamspace => teamspace_table(action, table),
            Kind::Pages => pages_table(action, table),
            Kind::Page if action == Action::ChangePageSpace => by_space_table(action, table),
            Kind::Page => page_table(action, table),
            Kind::Agents => agents_table(action, table),
            Kind::Agent => agent_table(action, table),
            Kind::Webhook => webhook_table(action, table),
            Kind::Instance => unreachable!("instance actions have no table"),
        }
    }
}

/// Every actor: instance admin or not, at a browser or with a token, in any role or
/// outside the workspace.
fn all_actors() -> Vec<Actor> {
    let mut actors = Vec::new();
    for instance_admin in [false, true] {
        for via_session in [true, false] {
            for role in [None].into_iter().chain(ROLES.into_iter().map(Some)) {
                actors.push(Actor::person(instance_admin, via_session, role));
            }
        }
    }
    actors
}

#[test]
fn instance_actions_need_an_instance_admin_whatever_their_workspace_role() {
    for creation in WorkspaceCreation::ALL {
        let instance = Resource::Instance {
            creation: *creation,
        };
        for actor in all_actors() {
            let admin = actor.instance_admin;
            for action in [Action::ListAllWorkspaces, Action::SetLimits] {
                assert_eq!(
                    authorize(actor, action, instance).is_ok(),
                    admin,
                    "{action:?} by {actor:?}"
                );
            }
            assert_eq!(
                outcome(authorize(actor, Action::DeleteAnyWorkspace, instance)),
                by_token_if(
                    actor,
                    Action::DeleteAnyWorkspace,
                    if admin { 'y' } else { 'f' }
                ),
                "deleting any workspace by {actor:?}"
            );
            let creates = *creation == WorkspaceCreation::Anyone || admin;
            assert_eq!(
                authorize(actor, Action::CreateWorkspace, instance).is_ok(),
                creates,
                "creating with {creation} by {actor:?}"
            );
        }
    }
}

/// [`by_token`] for an actor with a token, `want` for one at a browser.
fn by_token_if(actor: Actor, action: Action, want: char) -> char {
    if actor.via_session {
        want
    } else {
        by_token(action, want)
    }
}

#[test]
fn actions_on_the_wrong_kind_of_resource_are_refused() {
    for action in all_actions() {
        for resource in all_resources() {
            if resource_kind(resource) == kind(action) {
                continue;
            }
            for actor in all_actors() {
                assert!(
                    authorize(actor, action, resource).is_err(),
                    "{action:?} on {resource:?} by {actor:?}"
                );
            }
        }
    }
}

#[test]
fn only_a_browser_session_deletes_or_transfers() {
    let owner = |via_session| Actor {
        instance_admin: true,
        via_session,
        role: Some(WorkspaceRole::Owner),
        on_behalf_of: None,
    };
    let workspace = Resource::Workspace {
        members_can_create_teamspaces: true,
    };
    let member = Resource::Member {
        role: WorkspaceRole::Member,
        kind: UserKind::Human,
        is_actor: false,
    };
    let instance = Resource::Instance {
        creation: WorkspaceCreation::Anyone,
    };

    for (action, resource) in [
        (Action::DeleteWorkspace, workspace),
        (Action::TransferOwnership, member),
        (Action::DeleteAnyWorkspace, instance),
    ] {
        assert_eq!(authorize(owner(true), action, resource), Ok(()));
        let refused = authorize(owner(false), action, resource).expect_err("refused");
        assert!(
            refused.to_string().contains("needs a browser session"),
            "{action:?}: {refused}"
        );
    }
    assert_eq!(
        authorize(owner(false), Action::UpdateWorkspace, workspace),
        Ok(()),
        "a token does the rest"
    );
}

#[test]
fn refusals_say_why() {
    let admin = Actor {
        instance_admin: false,
        via_session: true,
        role: Some(WorkspaceRole::Admin),
        on_behalf_of: None,
    };
    let owner_target = Resource::Member {
        role: WorkspaceRole::Owner,
        kind: UserKind::Human,
        is_actor: false,
    };

    assert_eq!(
        authorize(admin, Action::RemoveMember, owner_target)
            .expect_err("refused")
            .to_string(),
        "only an owner may change or remove an owner"
    );
    assert_eq!(Denied::Hidden.to_string(), "not found");
}

#[test]
fn a_workspace_always_keeps_an_owner() {
    use WorkspaceRole::{Admin, Guest, Member, Owner};

    assert_eq!(check_owner_remains(1, Owner, Some(Admin)), Err(LastOwner));
    assert_eq!(check_owner_remains(1, Owner, Some(Guest)), Err(LastOwner));
    assert_eq!(check_owner_remains(1, Owner, None), Err(LastOwner));
    assert_eq!(check_owner_remains(2, Owner, Some(Member)), Ok(()));
    assert_eq!(check_owner_remains(2, Owner, None), Ok(()));
    assert_eq!(check_owner_remains(1, Owner, Some(Owner)), Ok(()));
    assert_eq!(check_owner_remains(1, Admin, Some(Member)), Ok(()));
    assert_eq!(check_owner_remains(1, Member, None), Ok(()));
    assert_eq!(check_owner_remains(0, Admin, Some(Owner)), Ok(()));
}

#[test]
fn a_teamspace_keeps_an_owner_unless_the_workspace_managers_take_over() {
    use TeamspaceRole::{Member, Owner};

    assert_eq!(
        check_teamspace_owner_remains(1, Owner, Some(Member), false),
        Err(LastTeamspaceOwner)
    );
    assert_eq!(
        check_teamspace_owner_remains(1, Owner, None, false),
        Err(LastTeamspaceOwner)
    );
    assert_eq!(check_teamspace_owner_remains(1, Owner, None, true), Ok(()));
    assert_eq!(
        check_teamspace_owner_remains(1, Owner, Some(Member), true),
        Ok(())
    );
    assert_eq!(check_teamspace_owner_remains(2, Owner, None, false), Ok(()));
    assert_eq!(
        check_teamspace_owner_remains(1, Owner, Some(Owner), false),
        Ok(())
    );
    assert_eq!(
        check_teamspace_owner_remains(1, Member, None, false),
        Ok(())
    );
    assert_eq!(
        check_teamspace_owner_remains(0, Member, Some(Owner), false),
        Ok(())
    );
}

fn as_member(role: WorkspaceRole) -> Actor {
    Actor {
        instance_admin: false,
        via_session: true,
        role: Some(role),
        on_behalf_of: None,
    }
}

#[test]
fn shares_add_to_what_the_space_gives_and_never_take_it_away() {
    let in_teamspace = PageSpace::Teamspace {
        access: TeamspaceAccess::Open,
        membership: Some(TeamspaceRole::Member),
    };
    let outside = PageSpace::Teamspace {
        access: TeamspaceAccess::Open,
        membership: None,
    };
    let page = |space, shared| Resource::Page {
        space,
        shared,
        trashed: false,
    };
    let member = as_member(WorkspaceRole::Member);

    assert_eq!(
        authorize(
            member,
            Action::EditPage,
            page(in_teamspace, Some(PageAccess::View))
        ),
        Ok(()),
        "a view share leaves a teamspace member editing"
    );
    assert!(matches!(
        authorize(member, Action::EditPage, page(outside, None)),
        Err(Denied::Forbidden(_))
    ));
    assert_eq!(
        authorize(
            member,
            Action::EditPage,
            page(outside, Some(PageAccess::Edit))
        ),
        Ok(())
    );
}

#[test]
fn guests_reach_pages_only_through_shares() {
    let guest = as_member(WorkspaceRole::Guest);
    let default = PageSpace::Teamspace {
        access: TeamspaceAccess::Default,
        membership: None,
    };
    let page = |shared| Resource::Page {
        space: default,
        shared,
        trashed: false,
    };

    assert_eq!(
        authorize(guest, Action::ViewPage, page(None)),
        Err(Denied::Hidden)
    );
    assert_eq!(
        authorize(guest, Action::ViewPage, page(Some(PageAccess::View))),
        Ok(())
    );
    assert_eq!(
        authorize(guest, Action::EditPage, page(Some(PageAccess::Edit))),
        Ok(())
    );
    assert_eq!(
        authorize(guest, Action::SharePage, page(Some(PageAccess::Edit)))
            .expect_err("refused")
            .to_string(),
        "guests see no member but themselves"
    );
    assert_eq!(
        authorize(
            guest,
            Action::ListPages,
            Resource::Pages {
                space: PageSpace::Private { own: true }
            }
        )
        .expect_err("refused")
        .to_string(),
        "guests have no private pages: they see the pages shared with them"
    );
}

#[test]
fn a_share_moves_a_page_only_within_its_space() {
    let member = as_member(WorkspaceRole::Member);
    let guest = as_member(WorkspaceRole::Guest);
    let page = |space, shared| Resource::Page {
        space,
        shared,
        trashed: false,
    };
    let theirs = PageSpace::Private { own: false };
    let secret = PageSpace::Teamspace {
        access: TeamspaceAccess::Private,
        membership: None,
    };
    let in_secret = PageSpace::Teamspace {
        access: TeamspaceAccess::Private,
        membership: Some(TeamspaceRole::Member),
    };
    let edit = Some(PageAccess::Edit);

    for (actor, space) in [(member, theirs), (member, secret), (guest, secret)] {
        assert_eq!(
            authorize(actor, Action::EditPage, page(space, edit)),
            Ok(())
        );
        assert_eq!(
            authorize(actor, Action::ChangePageSpace, page(space, edit))
                .expect_err("refused")
                .to_string(),
            "a page shared with you moves only within its space: those in its teamspace, or \
             its owner, may move it elsewhere",
            "{space:?}"
        );
    }
    assert_eq!(
        authorize(member, Action::ChangePageSpace, page(in_secret, None)),
        Ok(())
    );
    assert_eq!(
        authorize(
            member,
            Action::ChangePageSpace,
            page(PageSpace::Private { own: true }, None)
        ),
        Ok(())
    );
}

#[test]
fn a_trashed_page_shows_only_to_those_who_could_edit_it() {
    let member = as_member(WorkspaceRole::Member);
    let trashed = |space, shared| Resource::Page {
        space,
        shared,
        trashed: true,
    };
    let outside = PageSpace::Teamspace {
        access: TeamspaceAccess::Closed,
        membership: None,
    };
    let mine = PageSpace::Private { own: true };

    assert_eq!(
        authorize(member, Action::ViewPage, trashed(outside, None)),
        Err(Denied::Hidden),
        "a viewer does not see it in the trash"
    );
    assert_eq!(
        authorize(member, Action::RestorePage, trashed(mine, None)),
        Ok(())
    );
    assert_eq!(
        authorize(member, Action::EditPage, trashed(mine, None))
            .expect_err("refused")
            .to_string(),
        "this page is in the trash: restore it first"
    );
    assert_eq!(
        authorize(
            member,
            Action::DeletePage,
            Resource::Page {
                space: mine,
                shared: None,
                trashed: false
            }
        )
        .expect_err("refused")
        .to_string(),
        "only a page in the trash is deleted for good: trash it first"
    );
}
