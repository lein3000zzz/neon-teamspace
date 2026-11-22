//! Every actor, action and resource, against tables written out by hand.
//!
//! Each table has a row per resource and a column per workspace role: owner, admin,
//! member, guest. `y` is allowed, `f` forbidden, `h` hidden. Outside the tables, every
//! test also checks that someone outside the workspace is told nothing, whether or not
//! they are an instance admin, and that being an instance admin changes nothing inside a
//! workspace.

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

fn rows(table: &str) -> Vec<Vec<&str>> {
    table
        .lines()
        .map(str::split_whitespace)
        .map(Iterator::collect::<Vec<_>>)
        .filter(|cells| !cells.is_empty())
        .collect()
}

/// Checks every role, instance admin or not, and someone outside the workspace, on
/// `resource`.
fn check_members(action: Action, resource: Resource, want: [char; 4], row: &str) {
    for instance_admin in [false, true] {
        for (role, want) in ROLES.into_iter().zip(want) {
            let actor = Actor {
                instance_admin,
                role: Some(role),
            };
            assert_eq!(
                outcome(authorize(actor, action, resource)),
                want,
                "{action:?} on {row} by {role} (instance admin: {instance_admin})"
            );
        }
        let outsider = Actor {
            instance_admin,
            role: None,
        };
        assert_eq!(
            authorize(outsider, action, resource),
            Err(Denied::Hidden),
            "{action:?} on {row} by someone outside (instance admin: {instance_admin})"
        );
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
        for (role, want) in ROLES.into_iter().zip(want) {
            let actor = Actor {
                instance_admin,
                role: Some(role),
            };
            let resource = Resource::Member {
                role,
                is_actor: true,
            };
            assert_eq!(
                outcome(authorize(actor, action, resource)),
                want,
                "{action:?} on themselves by {role}"
            );
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
        Action::RemoveMember,
        Action::TransferOwnership,
        Action::ViewTeamspace,
        Action::SeeTeamspace,
        Action::JoinTeamspace,
        Action::LeaveTeamspace,
        Action::UpdateTeamspace,
        Action::DeleteTeamspace,
        Action::ListTeamspaceMembers,
        Action::AddTeamspaceMember,
        Action::RemoveTeamspaceMember,
        Action::ChangeTeamspaceRole,
    ];
    for role in ROLES {
        actions.push(Action::Invite { role });
        actions.push(Action::CreateInviteLink { role });
        actions.push(Action::ChangeRole { to: role });
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
        Action::ChangeRole { .. } | Action::RemoveMember | Action::TransferOwnership => {
            Kind::Member
        }
        Action::ViewTeamspace
        | Action::SeeTeamspace
        | Action::JoinTeamspace
        | Action::LeaveTeamspace
        | Action::UpdateTeamspace
        | Action::ChangeTeamspaceAccess { .. }
        | Action::DeleteTeamspace
        | Action::ListTeamspaceMembers
        | Action::AddTeamspaceMember
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
        (Action::ChangeRole { to: R::Owner }, MAKE_OWNER),
        (Action::ChangeRole { to: R::Admin }, CHANGE_ROLE),
        (Action::ChangeRole { to: R::Member }, CHANGE_ROLE),
        (Action::ChangeRole { to: R::Guest }, CHANGE_ROLE),
        (Action::RemoveMember, REMOVE_MEMBER),
        (Action::TransferOwnership, TRANSFER),
        // Teamspace actions.
        (Action::SeeTeamspace, SEE),
        (Action::ListTeamspaceMembers, SEE),
        (Action::ViewTeamspace, VIEW),
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
        (Action::DeleteTeamspace, NOT_THE_DEFAULT),
        (Action::AddTeamspaceMember, NOT_THE_DEFAULT),
        (Action::RemoveTeamspaceMember, NOT_THE_DEFAULT),
    ]
}

/// Only owners make owners, of anyone, themselves included.
const MAKE_OWNER: &str = "
    owner  y f f f
    admin  y f f f
    member y f f f
    guest  y f f f
    self   y f f f
";

/// Owners change anyone's role; admins anyone's but an owner's, their own too.
const CHANGE_ROLE: &str = "
    owner  y f f f
    admin  y y f f
    member y y f f
    guest  y y f f
    self   y y f f
";

/// Nobody removes themselves: they leave.
const REMOVE_MEMBER: &str = "
    owner  y f f f
    admin  y y f f
    member y y f f
    guest  y y f f
    self   f f f f
";

/// An owner hands ownership to anyone who is not an owner yet.
const TRANSFER: &str = "
    owner  f f f f
    admin  y f f f
    member y f f f
    guest  y f f f
    self   f f f f
";

/// That a teamspace exists, and who is in it: everyone but guests, except a private one,
/// which members outside it may not know of.
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

/// Everything about a teamspace: a private one only for its members, even owners and
/// admins of the workspace seeing its name only.
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
    private -      f f h h
    private member y y y h
    private owner  y y y h
";

/// The teamspace's owners and the workspace's owners and admins.
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
    private -      y y h h
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
    private -      y y h h
    private member y y f h
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
    private -      f f h h
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

    assert_eq!(actions.len(), 40, "every action and parameter is listed");
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

#[test]
fn instance_actions_need_an_instance_admin_whatever_their_workspace_role() {
    let actors = || {
        [false, true].into_iter().flat_map(|instance_admin| {
            [None]
                .into_iter()
                .chain(ROLES.into_iter().map(Some))
                .map(move |role| Actor {
                    instance_admin,
                    role,
                })
        })
    };
    for creation in WorkspaceCreation::ALL {
        let instance = Resource::Instance {
            creation: *creation,
        };
        for actor in actors() {
            let admin = actor.instance_admin;
            for action in [
                Action::ListAllWorkspaces,
                Action::SetLimits,
                Action::DeleteAnyWorkspace,
            ] {
                assert_eq!(
                    authorize(actor, action, instance).is_ok(),
                    admin,
                    "{action:?} by {actor:?}"
                );
            }
            let creates = *creation == WorkspaceCreation::Anyone || admin;
            assert_eq!(
                authorize(actor, Action::CreateWorkspace, instance).is_ok(),
                creates,
                "creating with {creation} by {actor:?}"
            );
        }
    }
}

#[test]
fn actions_on_the_wrong_kind_of_resource_are_refused() {
    for action in all_actions() {
        for resource in all_resources() {
            if resource_kind(resource) == kind(action) {
                continue;
            }
            for instance_admin in [false, true] {
                for role in [None].into_iter().chain(ROLES.into_iter().map(Some)) {
                    let actor = Actor {
                        instance_admin,
                        role,
                    };
                    assert!(
                        authorize(actor, action, resource).is_err(),
                        "{action:?} on {resource:?} by {actor:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn refusals_say_why() {
    let admin = Actor {
        instance_admin: false,
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
