//! Every actor, action and resource, against tables written out by hand.
//!
//! Each table has a row per resource and a column per workspace role: owner, admin,
//! member, guest. `y` is allowed, `f` forbidden, `h` hidden. The tables are what a browser
//! session gets; an API token gets the same but for what only a session may do, which it
//! is forbidden. Outside the tables, every test also checks that someone outside the
//! workspace is told nothing, whether or not they are an instance admin, and that being an
//! instance admin changes nothing inside a workspace.

use super::*;

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

/// What only a browser session may do: what cannot be undone.
fn session_only(action: Action) -> bool {
    matches!(
        action,
        Action::DeleteWorkspace | Action::TransferOwnership | Action::DeleteAnyWorkspace
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

/// Rows for a member of each role who is not the actor, then `self`, the actor.
fn member_table(action: Action, table: &str) {
    let rows = rows(table);
    let labels: Vec<&str> = rows.iter().map(|row| row[0]).collect();
    assert_eq!(
        labels,
        ["owner", "admin", "member", "guest", "self"],
        "{action:?}"
    );
    for row in &rows[..4] {
        let target = WorkspaceRole::parse(row[0]).expect("a role");
        let resource = Resource::Member {
            role: target,
            is_actor: false,
        };
        check_members(action, resource, expected(&row[1..]), row[0]);
    }
    // The actor is a member of their own role.
    let want = expected(&rows[4][1..]);
    for instance_admin in [false, true] {
        for via_session in [true, false] {
            for (role, want) in ROLES.into_iter().zip(want) {
                let actor = Actor {
                    instance_admin,
                    via_session,
                    role: Some(role),
                };
                let resource = Resource::Member {
                    role,
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
    ];
    for role in ROLES {
        actions.push(Action::Invite { role });
        actions.push(Action::CreateInviteLink { role });
        actions.push(Action::ChangeRole { to: role });
        actions.push(Action::AddTeamspaceMember {
            workspace_role: role,
        });
    }
    for access in TeamspaceAccess::ALL {
        actions.push(Action::ChangeTeamspaceAccess { to: *access });
    }
    actions
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Instance,
    Workspace,
    Member,
    Teamspace,
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
        | Action::CreateTeamspace => Kind::Workspace,
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
        for is_actor in [false, true] {
            resources.push(Resource::Member { role, is_actor });
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
    resources
}

fn resource_kind(resource: Resource) -> Kind {
    match resource {
        Resource::Instance { .. } => Kind::Instance,
        Resource::Workspace { .. } => Kind::Workspace,
        Resource::Member { .. } => Kind::Member,
        Resource::Teamspace { .. } => Kind::Teamspace,
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
                workspace_role: R::Owner,
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                workspace_role: R::Admin,
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                workspace_role: R::Member,
            },
            NOT_THE_DEFAULT,
        ),
        (
            Action::AddTeamspaceMember {
                workspace_role: R::Guest,
            },
            ADD_A_GUEST,
        ),
    ]
}

/// Everyone knows who is in the workspace, but guests, who know only themselves.
const VIEW_MEMBER: &str = "
    owner  y y y h
    admin  y y y h
    member y y y h
    guest  y y y h
    self   y y y y
";

/// Only owners make owners, of anyone, themselves included.
const MAKE_OWNER: &str = "
    owner  y f f h
    admin  y f f h
    member y f f h
    guest  y f f h
    self   y f f f
";

/// Owners change anyone's role; admins anyone's but an owner's, their own too.
const CHANGE_ROLE: &str = "
    owner  y f f h
    admin  y y f h
    member y y f h
    guest  y y f h
    self   y y f f
";

/// Nobody removes themselves: they leave.
const REMOVE_MEMBER: &str = "
    owner  y f f h
    admin  y y f h
    member y y f h
    guest  y y f h
    self   f f f f
";

/// An owner hands ownership to anyone who is not an owner yet.
const TRANSFER: &str = "
    owner  f f f h
    admin  y f f h
    member y f f h
    guest  y f f h
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

#[test]
fn every_action_has_exactly_one_table() {
    let actions = all_actions();
    let tables = tables();
    let workspace_and_below = actions
        .iter()
        .filter(|action| kind(**action) != Kind::Instance)
        .count();

    assert_eq!(actions.len(), 44, "every action and parameter is listed");
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
                actors.push(Actor {
                    instance_admin,
                    via_session,
                    role,
                });
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
    };
    let workspace = Resource::Workspace {
        members_can_create_teamspaces: true,
    };
    let member = Resource::Member {
        role: WorkspaceRole::Member,
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
    };
    let owner_target = Resource::Member {
        role: WorkspaceRole::Owner,
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
    assert_eq!(check_owner_remains(1, true, false), Err(LastOwner));
    assert_eq!(check_owner_remains(2, true, false), Ok(()));
    assert_eq!(check_owner_remains(1, true, true), Ok(()));
    assert_eq!(check_owner_remains(1, false, false), Ok(()));
    assert_eq!(check_owner_remains(0, false, true), Ok(()));
}

#[test]
fn a_teamspace_keeps_an_owner_unless_the_workspace_managers_take_over() {
    assert_eq!(
        check_teamspace_owner_remains(1, true, false, false),
        Err(LastTeamspaceOwner)
    );
    assert_eq!(check_teamspace_owner_remains(1, true, false, true), Ok(()));
    assert_eq!(check_teamspace_owner_remains(2, true, false, false), Ok(()));
    assert_eq!(check_teamspace_owner_remains(1, true, true, false), Ok(()));
    assert_eq!(
        check_teamspace_owner_remains(1, false, false, false),
        Ok(())
    );
}
