//! Every action of an agent or integration, against tables written out by hand.
//!
//! The delegate is a member of the workspace, as every agent and integration is. Each table
//! has a cell per role of its owner: owner, admin, member, guest. `y` is allowed, `f`
//! forbidden, `h` hidden. Teamspace tables have a row per access, the delegate's place in
//! the teamspace and its owner's, `-` for none; page tables a row per access the delegate
//! and its owner each end up with on the page, live and in the trash, checked on every
//! space and share that gives them. Outside the tables, every test also checks that an
//! owner out of the workspace leaves the delegate nothing, and two laws hold over every
//! action and resource: a delegate never does what it could not do as a member itself, nor
//! what its owner could not, and being made a teamspace's owner would give it nothing more.

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

const SHARES: [Option<PageAccess>; 3] = [None, Some(PageAccess::View), Some(PageAccess::Edit)];

fn outcome(result: Result<(), Denied>) -> char {
    match result {
        Ok(()) => 'y',
        Err(Denied::Hidden) => 'h',
        Err(Denied::Forbidden(_)) => 'f',
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

fn cells<const N: usize>(cells: &[&str]) -> [char; N] {
    assert_eq!(cells.len(), N, "a row ends with {N} cells: {cells:?}");
    let mut out = ['?'; N];
    for (slot, cell) in out.iter_mut().zip(cells) {
        let mut chars = cell.chars();
        let (Some(c @ ('y' | 'f' | 'h')), None) = (chars.next(), chars.next()) else {
            panic!("a cell is y, f or h: {cell:?}");
        };
        *slot = c;
    }
    out
}

/// A delegate, a member itself, acting with a token, or a session for good measure, for an
/// owner with `role` who stands to the resource as `theirs`.
fn delegate(via_session: bool, role: Option<WorkspaceRole>, theirs: Resource) -> Actor {
    Actor {
        instance_admin: false,
        via_session,
        role: Some(WorkspaceRole::Member),
        on_behalf_of: Some(OwnerStanding {
            role,
            resource: theirs,
        }),
    }
}

/// Checks `action` on `mine`, the resource as the delegate stands to it, for an owner of
/// each role standing to it as `theirs` gives for that role, and for an owner out of the
/// workspace, who leaves the delegate nothing.
fn check(
    action: Action,
    mine: Resource,
    theirs: impl Fn(WorkspaceRole) -> Resource,
    want: [char; 4],
    label: &str,
) {
    for via_session in [false, true] {
        for (role, want) in ROLES.into_iter().zip(want) {
            assert_eq!(
                outcome(authorize(
                    delegate(via_session, Some(role), theirs(role)),
                    action,
                    mine
                )),
                want,
                "{action:?} on {label} for an owner who is {role} (session: {via_session})"
            );
        }
        assert_eq!(
            authorize(
                delegate(via_session, None, theirs(WorkspaceRole::Member)),
                action,
                mine
            ),
            Err(Denied::Hidden),
            "{action:?} on {label} for an owner out of the workspace"
        );
    }
}

// Workspace actions: one row, as nothing about the workspace depends on who stands to it.

fn workspace_table(action: Action, table: &str) {
    let rows = rows(table);
    assert_eq!(rows.len(), 1, "{action:?}");
    for members_can_create_teamspaces in [true, false] {
        let workspace = Resource::Workspace {
            members_can_create_teamspaces,
        };
        check(
            action,
            workspace,
            |_| workspace,
            cells(&rows[0][1..]),
            "the workspace",
        );
    }
}

fn workspace_tables() -> Vec<(Action, &'static str)> {
    use WorkspaceRole as R;

    let mut tables = vec![
        (Action::ViewWorkspace, "all y y y y"),
        (Action::ListMembers, "all y y y f"),
        (Action::Leave, "all f f f f"),
        (Action::UpdateWorkspace, "all f f f f"),
        (Action::DeleteWorkspace, "all f f f f"),
        (Action::ManageInvitations, "all f f f f"),
        (Action::ManageInviteLinks, "all f f f f"),
        (Action::CreateTeamspace, "all f f f f"),
    ];
    for role in [R::Owner, R::Admin, R::Member, R::Guest] {
        tables.push((Action::Invite { role }, "all f f f f"));
        tables.push((Action::CreateInviteLink { role }, "all f f f f"));
    }
    tables
}

#[test]
fn delegates_see_the_workspace_and_manage_nothing_in_it() {
    for (action, table) in workspace_tables() {
        workspace_table(action, table);
    }
}

// Member actions: a row per person who is neither the delegate nor its owner, by role,
// another agent, an integration, the delegate itself, and its owner, whose role is the
// column's.

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
            "self",
            "its-owner"
        ],
        "{action:?}"
    );
    let member = |role, kind, is_actor| Resource::Member {
        role,
        kind,
        is_actor,
    };
    for row in &rows[..6] {
        let (role, kind) = match UserKind::parse(row[0]) {
            Some(kind) => (WorkspaceRole::Member, kind),
            None => (
                WorkspaceRole::parse(row[0]).expect("a role"),
                UserKind::Human,
            ),
        };
        let target = member(role, kind, false);
        check(action, target, |_| target, cells(&row[1..]), row[0]);
    }
    for kind in [UserKind::Agent, UserKind::Integration] {
        check(
            action,
            member(WorkspaceRole::Member, kind, true),
            |_| member(WorkspaceRole::Member, kind, false),
            cells(&rows[6][1..]),
            "itself",
        );
    }
    // Its owner's role is the column's: they are the member the delegate acts on.
    let want: [char; 4] = cells(&rows[7][1..]);
    for (role, want) in ROLES.into_iter().zip(want) {
        for via_session in [false, true] {
            assert_eq!(
                outcome(authorize(
                    delegate(via_session, Some(role), member(role, UserKind::Human, true)),
                    action,
                    member(role, UserKind::Human, false),
                )),
                want,
                "{action:?} on its owner, who is {role}"
            );
        }
    }
}

fn member_tables() -> Vec<(Action, &'static str)> {
    let mut tables = vec![
        (Action::ViewMember, VIEW_MEMBER),
        (Action::RemoveMember, MANAGE_MEMBER),
        (Action::TransferOwnership, MANAGE_MEMBER),
    ];
    for to in ROLES {
        tables.push((Action::ChangeRole { to }, MANAGE_MEMBER));
    }
    tables
}

/// Who is in the workspace: what its owner may know, a guest only themselves.
const VIEW_MEMBER: &str = "
    owner       y y y h
    admin       y y y h
    member      y y y h
    guest       y y y h
    agent       y y y h
    integration y y y h
    self        y y y h
    its-owner   y y y y
";

/// Never: what a guest owner may not know of stays hidden.
const MANAGE_MEMBER: &str = "
    owner       f f f h
    admin       f f f h
    member      f f f h
    guest       f f f h
    agent       f f f h
    integration f f f h
    self        f f f h
    its-owner   f f f f
";

#[test]
fn delegates_see_members_as_their_owner_does_and_change_none() {
    for (action, table) in member_tables() {
        member_table(action, table);
    }
}

// Teamspace actions: a row per access, the delegate's place in the teamspace and its
// owner's.

fn membership(cell: &str) -> Option<TeamspaceRole> {
    match cell {
        "-" => None,
        role => Some(TeamspaceRole::parse(role).expect("a teamspace role")),
    }
}

/// The teamspace rows: every access, the delegate out of it or a member, and its owner out
/// of it, a member or an owner.
fn teamspace_rows() -> Vec<(
    TeamspaceAccess,
    Option<TeamspaceRole>,
    Option<TeamspaceRole>,
)> {
    let mut all = Vec::new();
    for access in TeamspaceAccess::ALL {
        for mine in [None, Some(TeamspaceRole::Member)] {
            for theirs in [
                None,
                Some(TeamspaceRole::Member),
                Some(TeamspaceRole::Owner),
            ] {
                all.push((*access, mine, theirs));
            }
        }
    }
    all
}

fn teamspace_table(action: Action, table: &str) {
    let rows = rows(table);
    let mut seen = Vec::new();
    for row in &rows {
        let access = TeamspaceAccess::parse(row[0]).expect("an access");
        let (mine, theirs) = (membership(row[1]), membership(row[2]));
        seen.push((access, mine, theirs));
        let label = format!(
            "{} teamspace, delegate {}, owner {}",
            row[0], row[1], row[2]
        );
        check(
            action,
            Resource::Teamspace {
                access,
                membership: mine,
            },
            |_| Resource::Teamspace {
                access,
                membership: theirs,
            },
            cells(&row[3..]),
            &label,
        );
    }
    assert_eq!(seen, teamspace_rows(), "{action:?}: one row each, in order");
}

fn teamspace_tables() -> Vec<(Action, &'static str)> {
    let mut tables = vec![
        (Action::SeeTeamspace, SEE),
        (Action::ViewTeamspace, VIEW),
        (Action::ListTeamspaceMembers, VIEW),
        (Action::JoinTeamspace, JOIN),
        (Action::LeaveTeamspace, LEAVE),
        (Action::UpdateTeamspace, MANAGE),
        (Action::ChangeTeamspaceRole, MANAGE),
        (Action::RemoveTeamspaceMember, MANAGE),
        (Action::DeleteTeamspace, MANAGE_DELETE),
    ];
    for to in TeamspaceAccess::ALL {
        tables.push((Action::ChangeTeamspaceAccess { to: *to }, MANAGE));
    }
    for newcomer in ROLES.into_iter().map(Newcomer::Person).chain([
        Newcomer::Delegate { own: false },
        Newcomer::Delegate { own: true },
    ]) {
        tables.push((Action::AddTeamspaceMember { newcomer }, MANAGE));
    }
    tables
}

/// That it exists: a private one for those in it, and for its owner's owners and admins
/// only when the delegate is in it too.
const SEE: &str = "
    default -      -      y y y h
    default -      member y y y h
    default -      owner  y y y h
    default member -      y y y h
    default member member y y y h
    default member owner  y y y h
    open    -      -      y y y h
    open    -      member y y y h
    open    -      owner  y y y h
    open    member -      y y y h
    open    member member y y y h
    open    member owner  y y y h
    closed  -      -      y y y h
    closed  -      member y y y h
    closed  -      owner  y y y h
    closed  member -      y y y h
    closed  member member y y y h
    closed  member owner  y y y h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      y y h h
    private member member y y y h
    private member owner  y y y h
";

/// Everything about it: a private one only when both are in it.
const VIEW: &str = "
    default -      -      y y y h
    default -      member y y y h
    default -      owner  y y y h
    default member -      y y y h
    default member member y y y h
    default member owner  y y y h
    open    -      -      y y y h
    open    -      member y y y h
    open    -      owner  y y y h
    open    member -      y y y h
    open    member member y y y h
    open    member owner  y y y h
    closed  -      -      y y y h
    closed  -      member y y y h
    closed  -      owner  y y y h
    closed  member -      y y y h
    closed  member member y y y h
    closed  member owner  y y y h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      h h h h
    private member member y y y h
    private member owner  y y y h
";

/// An open teamspace it is not in yet, which its owner sees in full; it joins no other
/// itself, whoever its owner is.
const JOIN: &str = "
    default -      -      f f f h
    default -      member f f f h
    default -      owner  f f f h
    default member -      f f f h
    default member member f f f h
    default member owner  f f f h
    open    -      -      y y y h
    open    -      member y y y h
    open    -      owner  y y y h
    open    member -      f f f h
    open    member member f f f h
    open    member owner  f f f h
    closed  -      -      f f f h
    closed  -      member f f f h
    closed  -      owner  f f f h
    closed  member -      f f f h
    closed  member member f f f h
    closed  member owner  f f f h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      h h h h
    private member member f f f h
    private member owner  f f f h
";

/// Any teamspace it is in, but the default one, whatever its owner may.
const LEAVE: &str = "
    default -      -      f f f f
    default -      member f f f f
    default -      owner  f f f f
    default member -      f f f f
    default member member f f f f
    default member owner  f f f f
    open    -      -      f f f f
    open    -      member f f f f
    open    -      owner  f f f f
    open    member -      y y y y
    open    member member y y y y
    open    member owner  y y y y
    closed  -      -      f f f f
    closed  -      member f f f f
    closed  -      owner  f f f f
    closed  member -      y y y y
    closed  member member y y y y
    closed  member owner  y y y y
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      y y y y
    private member member y y y y
    private member owner  y y y y
";

/// Never, but hidden where either may not know of it.
const MANAGE: &str = "
    default -      -      f f f h
    default -      member f f f h
    default -      owner  f f f h
    default member -      f f f h
    default member member f f f h
    default member owner  f f f h
    open    -      -      f f f h
    open    -      member f f f h
    open    -      owner  f f f h
    open    member -      f f f h
    open    member member f f f h
    open    member owner  f f f h
    closed  -      -      f f f h
    closed  -      member f f f h
    closed  -      owner  f f f h
    closed  member -      f f f h
    closed  member member f f f h
    closed  member owner  f f f h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      h h h h
    private member member f f f h
    private member owner  f f f h
";

/// As [`MANAGE`], but an owner or admin of the workspace knows a private teamspace by name
/// to delete it.
const MANAGE_DELETE: &str = "
    default -      -      f f f h
    default -      member f f f h
    default -      owner  f f f h
    default member -      f f f h
    default member member f f f h
    default member owner  f f f h
    open    -      -      f f f h
    open    -      member f f f h
    open    -      owner  f f f h
    open    member -      f f f h
    open    member member f f f h
    open    member owner  f f f h
    closed  -      -      f f f h
    closed  -      member f f f h
    closed  -      owner  f f f h
    closed  member -      f f f h
    closed  member member f f f h
    closed  member owner  f f f h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      f f h h
    private member member f f f h
    private member owner  f f f h
";

#[test]
fn delegates_join_and_see_teamspaces_within_their_owners_reach() {
    for (action, table) in teamspace_tables() {
        teamspace_table(action, table);
    }
}

// The top of a space's pages: a row per space as the delegate and its owner each stand to
// it. For private pages, `mine` is the delegate's own, `theirs` someone else's, and
// `owners` its owner's.

/// Every pair of spaces, as the delegate and its owner stand to one, with its label.
fn space_pairs() -> Vec<(PageSpace, PageSpace, String)> {
    let mut pairs = Vec::new();
    for (access, mine, theirs) in teamspace_rows() {
        let label = |m: Option<TeamspaceRole>| m.map_or("-".to_owned(), |m| m.to_string());
        pairs.push((
            PageSpace::Teamspace {
                access,
                membership: mine,
            },
            PageSpace::Teamspace {
                access,
                membership: theirs,
            },
            format!("{access} {} {}", label(mine), label(theirs)),
        ));
    }
    let private = |own| PageSpace::Private { own };
    pairs.push((private(true), private(false), "mine".to_owned()));
    pairs.push((private(false), private(true), "owners".to_owned()));
    pairs.push((private(false), private(false), "theirs".to_owned()));
    pairs
}

fn pages_table(action: Action, table: &str) {
    let rows = rows(table);
    let pairs = space_pairs();
    assert_eq!(rows.len(), pairs.len(), "{action:?}: a row for each pair");
    for (row, (mine, theirs, label)) in rows.iter().zip(&pairs) {
        let (cells_at, row_label) =
            if row[0] == "private" || TeamspaceAccess::parse(row[0]).is_some() {
                (3, format!("{} {} {}", row[0], row[1], row[2]))
            } else {
                (1, row[0].to_owned())
            };
        assert_eq!(&row_label, label, "{action:?}: rows in order");
        let theirs = *theirs;
        check(
            action,
            Resource::Pages { space: *mine },
            |_| Resource::Pages { space: theirs },
            cells(&row[cells_at..]),
            label,
        );
    }
}

/// A teamspace's tree: as [`VIEW`]. Private pages: nobody's, as its owner may not see the
/// delegate's, which it is told, nor the delegate its owner's.
const LIST_PAGES: &str = "
    default -      -      y y y h
    default -      member y y y h
    default -      owner  y y y h
    default member -      y y y h
    default member member y y y h
    default member owner  y y y h
    open    -      -      y y y h
    open    -      member y y y h
    open    -      owner  y y y h
    open    member -      y y y h
    open    member member y y y h
    open    member owner  y y y h
    closed  -      -      y y y h
    closed  -      member y y y h
    closed  -      owner  y y y h
    closed  member -      y y y h
    closed  member member y y y h
    closed  member owner  y y y h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      h h h h
    private member member y y y h
    private member owner  y y y h
    mine                  f f f f
    owners                h h h h
    theirs                h h h h
";

/// A page at the top of a teamspace both are in; none private.
const CREATE_PAGE: &str = "
    default -      -      f f f h
    default -      member f f f h
    default -      owner  f f f h
    default member -      f f f h
    default member member y y y h
    default member owner  y y y h
    open    -      -      f f f h
    open    -      member f f f h
    open    -      owner  f f f h
    open    member -      f f f h
    open    member member y y y h
    open    member owner  y y y h
    closed  -      -      f f f h
    closed  -      member f f f h
    closed  -      owner  f f f h
    closed  member -      f f f h
    closed  member member y y y h
    closed  member owner  y y y h
    private -      -      h h h h
    private -      member h h h h
    private -      owner  h h h h
    private member -      h h h h
    private member member y y y h
    private member owner  y y y h
    mine                  f f f f
    owners                h h h h
    theirs                h h h h
";

#[test]
fn delegates_list_and_add_pages_where_both_may() {
    pages_table(Action::ListPages, LIST_PAGES);
    pages_table(Action::CreatePage, CREATE_PAGE);
}

// One page: a row per access the delegate and its owner each end up with, and whether the
// page is in the trash, checked on every space and share that gives them those.

fn access(cell: &str) -> Option<PageAccess> {
    match cell {
        "none" => None,
        "view" => Some(PageAccess::View),
        "edit" => Some(PageAccess::Edit),
        other => panic!("an access is none, view or edit: {other:?}"),
    }
}

/// What a space gives a role before shares, as the person tables have it.
fn space_access(role: WorkspaceRole, space: PageSpace) -> Option<PageAccess> {
    if !role.joins_teamspaces() {
        return None;
    }
    match space {
        PageSpace::Teamspace {
            membership: Some(_),
            ..
        }
        | PageSpace::Private { own: true } => Some(PageAccess::Edit),
        PageSpace::Teamspace {
            access: TeamspaceAccess::Private,
            membership: None,
        }
        | PageSpace::Private { own: false } => None,
        PageSpace::Teamspace {
            membership: None, ..
        } => Some(PageAccess::View),
    }
}

/// Calls `each` with every page the delegate and its owner may stand to: the spaces, the
/// shares, the owner's role and the trash, with the delegate's own and shared access and
/// its owner's.
fn every_page(
    mut each: impl FnMut(Resource, Resource, WorkspaceRole, [Option<PageAccess>; 4], String),
) {
    for (mine, theirs, label) in space_pairs() {
        for my_share in SHARES {
            for their_share in SHARES {
                for role in ROLES {
                    for trashed in [false, true] {
                        let page = |space, shared| Resource::Page {
                            space,
                            shared,
                            trashed,
                        };
                        let accesses = [
                            space_access(WorkspaceRole::Member, mine),
                            my_share,
                            space_access(role, theirs),
                            their_share,
                        ];
                        each(
                            page(mine, my_share),
                            page(theirs, their_share),
                            role,
                            accesses,
                            format!(
                                "{label}, shared {my_share:?} and {their_share:?}, owner \
                                 {role}, trashed {trashed}"
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// Rows by the delegate's access, its owner's and `live` or `trashed`, then one cell for an
/// owner who is in teamspaces and one for an owner who is a guest.
fn page_table(action: Action, table: &str) {
    let mut outcomes = std::collections::HashMap::new();
    for row in rows(table) {
        let [mine, theirs, state, people, guest] = row[..] else {
            panic!("a row is two accesses, a state and two cells: {row:?}");
        };
        let trashed = state == "trashed";
        let key = (access(mine), access(theirs), trashed);
        let earlier = outcomes.insert(key, cells::<2>(&[people, guest]));
        assert!(earlier.is_none(), "one row each: {row:?}");
    }
    assert_eq!(outcomes.len(), 18, "{action:?}: a row for every case");
    every_page(|mine, theirs, role, accesses, label| {
        let Resource::Page { trashed, .. } = mine else {
            unreachable!("a page")
        };
        let key = (
            accesses[0].max(accesses[1]),
            accesses[2].max(accesses[3]),
            trashed,
        );
        let want = outcomes[&key][usize::from(!role.joins_teamspaces())];
        for via_session in [false, true] {
            assert_eq!(
                outcome(authorize(
                    delegate(via_session, Some(role), theirs),
                    action,
                    mine
                )),
                want,
                "{action:?} on {label}"
            );
        }
    });
}

/// Any access of both shows a live page; edit access of both a trashed one.
const VIEW_PAGE: &str = "
    none none live    h h
    none none trashed h h
    none view live    h h
    none view trashed h h
    none edit live    h h
    none edit trashed h h
    view none live    h h
    view none trashed h h
    view view live    y y
    view view trashed h h
    view edit live    y y
    view edit trashed h h
    edit none live    h h
    edit none trashed h h
    edit view live    y y
    edit view trashed h h
    edit edit live    y y
    edit edit trashed y y
";

/// Editing and trashing: edit access of both, out of the trash.
const EDIT_PAGE: &str = "
    none none live    h h
    none none trashed h h
    none view live    h h
    none view trashed h h
    none edit live    h h
    none edit trashed h h
    view none live    h h
    view none trashed h h
    view view live    f f
    view view trashed h h
    view edit live    f f
    view edit trashed h h
    edit none live    h h
    edit none trashed h h
    edit view live    f f
    edit view trashed h h
    edit edit live    y y
    edit edit trashed f f
";

/// Restoring and deleting for good: a page in the trash both could edit.
const FROM_THE_TRASH: &str = "
    none none live    h h
    none none trashed h h
    none view live    h h
    none view trashed h h
    none edit live    h h
    none edit trashed h h
    view none live    h h
    view none trashed h h
    view view live    f f
    view view trashed h h
    view edit live    f f
    view edit trashed h h
    edit none live    h h
    edit none trashed h h
    edit view live    f f
    edit view trashed h h
    edit edit live    f f
    edit edit trashed y y
";

/// Who it is shared with: for a page both see, unless the owner is a guest.
const LIST_SHARES: &str = "
    none none live    h h
    none none trashed h h
    none view live    h h
    none view trashed h h
    none edit live    h h
    none edit trashed h h
    view none live    h h
    view none trashed h h
    view view live    y f
    view view trashed h h
    view edit live    y f
    view edit trashed h h
    edit none live    h h
    edit none trashed h h
    edit view live    y f
    edit view trashed h h
    edit edit live    y f
    edit edit trashed y f
";

/// Sharing it: for a live page both edit, unless the owner is a guest.
const SHARE_PAGE: &str = "
    none none live    h h
    none none trashed h h
    none view live    h h
    none view trashed h h
    none edit live    h h
    none edit trashed h h
    view none live    h h
    view none trashed h h
    view view live    f f
    view view trashed h h
    view edit live    f f
    view edit trashed h h
    edit none live    h h
    edit none trashed h h
    edit view live    f f
    edit view trashed h h
    edit edit live    y f
    edit edit trashed f f
";

#[test]
fn delegates_reach_a_page_only_as_far_as_their_owner_does() {
    for (action, table) in [
        (Action::ViewPage, VIEW_PAGE),
        (Action::EditPage, EDIT_PAGE),
        (Action::TrashPage, EDIT_PAGE),
        (Action::RestorePage, FROM_THE_TRASH),
        (Action::DeletePage, FROM_THE_TRASH),
        (Action::ListShares, LIST_SHARES),
        (Action::SharePage, SHARE_PAGE),
    ] {
        page_table(action, table);
    }
}

/// How one may move a page as its access comes: `none`, `view`, edit through a share only
/// (`share`), or edit through its space (`space`), which alone moves a page out of it.
fn move_class(base: Option<PageAccess>, shared: Option<PageAccess>) -> &'static str {
    match (base, base.max(shared)) {
        (Some(PageAccess::Edit), _) => "space",
        (_, Some(PageAccess::Edit)) => "share",
        (_, Some(PageAccess::View)) => "view",
        (_, None) => "none",
    }
}

/// Moving a page out of its space: for those whose space lets both edit it, out of the
/// trash.
const CHANGE_SPACE: &str = "
    none  none  live    h
    none  none  trashed h
    none  view  live    h
    none  view  trashed h
    none  share live    h
    none  share trashed h
    none  space live    h
    none  space trashed h
    view  none  live    h
    view  none  trashed h
    view  view  live    f
    view  view  trashed h
    view  share live    f
    view  share trashed h
    view  space live    f
    view  space trashed h
    share none  live    h
    share none  trashed h
    share view  live    f
    share view  trashed h
    share share live    f
    share share trashed f
    share space live    f
    share space trashed f
    space none  live    h
    space none  trashed h
    space view  live    f
    space view  trashed h
    space share live    f
    space share trashed f
    space space live    y
    space space trashed f
";

#[test]
fn delegates_move_a_page_out_of_its_space_only_if_both_may() {
    let mut outcomes = std::collections::HashMap::new();
    for row in rows(CHANGE_SPACE) {
        let [mine, theirs, state, cell] = row[..] else {
            panic!("a row is two classes, a state and a cell: {row:?}");
        };
        let earlier = outcomes.insert((mine, theirs, state == "trashed"), cells::<1>(&[cell])[0]);
        assert!(earlier.is_none(), "one row each: {row:?}");
    }
    assert_eq!(outcomes.len(), 32);
    every_page(|mine, theirs, role, accesses, label| {
        let Resource::Page { trashed, .. } = mine else {
            unreachable!("a page")
        };
        let key = (
            move_class(accesses[0], accesses[1]),
            move_class(accesses[2], accesses[3]),
            trashed,
        );
        assert_eq!(
            outcome(authorize(
                delegate(false, Some(role), theirs),
                Action::ChangePageSpace,
                mine
            )),
            outcomes[&key],
            "moving out of its space: {label}"
        );
    });
}

// Agents: a delegate makes and manages none, and sees one as its owner does.

#[test]
fn delegates_make_and_manage_no_agent() {
    for agents in [true, false] {
        for creation in AgentCreation::ALL {
            let resource = Resource::Agents {
                instance: agents,
                workspace: true,
                creation: *creation,
            };
            check(
                Action::CreateAgent,
                resource,
                |_| resource,
                ['f'; 4],
                "the workspace's agents",
            );
        }
    }
    for kind in [UserKind::Agent, UserKind::Integration] {
        for owners in [false, true] {
            let mine = Resource::Agent { kind, own: false };
            let theirs = |_| Resource::Agent { kind, own: owners };
            let label = format!("an {kind}, its owner's: {owners}");
            check(
                Action::ViewAgent,
                mine,
                theirs,
                ['y', 'y', 'y', 'h'],
                &label,
            );
            for action in [
                Action::UpdateAgent,
                Action::TransferAgent,
                Action::DeleteAgent,
                Action::ListAgentTokens,
                Action::CreateAgentToken,
                Action::RevokeAgentToken,
            ] {
                check(action, mine, theirs, ['f', 'f', 'f', 'h'], &label);
            }
        }
    }
}

#[test]
fn delegates_never_act_on_the_instance() {
    for creation in WorkspaceCreation::ALL {
        let instance = Resource::Instance {
            creation: *creation,
        };
        for action in [
            Action::CreateWorkspace,
            Action::ListAllWorkspaces,
            Action::SetLimits,
            Action::DeleteAnyWorkspace,
        ] {
            for role in [None].into_iter().chain(ROLES.map(Some)) {
                let mut actor = delegate(true, role, instance);
                actor.instance_admin = true;
                let refused = authorize(actor, action, instance).expect_err("refused");
                assert_eq!(
                    refused.to_string(),
                    "an agent or integration works inside its own workspace only",
                    "{action:?}"
                );
            }
        }
    }
}

#[test]
fn an_owner_standing_to_another_kind_of_resource_is_refused() {
    let page = Resource::Pages {
        space: PageSpace::Private { own: false },
    };
    let workspace = Resource::Workspace {
        members_can_create_teamspaces: true,
    };
    let actor = delegate(false, Some(WorkspaceRole::Owner), page);

    assert_eq!(
        authorize(actor, Action::ViewWorkspace, workspace),
        Err(NOT_APPLICABLE)
    );
}

#[test]
fn refusals_tell_whose_part_is_missing() {
    let open = |membership| Resource::Teamspace {
        access: TeamspaceAccess::Open,
        membership,
    };
    let pages = |membership| Resource::Pages {
        space: PageSpace::Teamspace {
            access: TeamspaceAccess::Open,
            membership,
        },
    };
    let member = Some(TeamspaceRole::Member);

    assert_eq!(
        authorize(
            delegate(false, Some(WorkspaceRole::Member), pages(None)),
            Action::CreatePage,
            pages(member)
        ),
        Err(OWNER_MAY_NOT),
    );
    assert_eq!(
        authorize(
            delegate(false, Some(WorkspaceRole::Owner), open(member)),
            Action::UpdateTeamspace,
            open(member)
        ),
        Err(DELEGATES_MANAGE_NOTHING),
    );
    assert!(OWNER_MAY_NOT.to_string().contains("its owner may not"));
}

/// Every resource, with every way an owner may stand to it: the same, but for the parts
/// that are about who stands to it.
fn resource_pairs() -> Vec<(Resource, Resource)> {
    let mut pairs = Vec::new();
    for members_can_create_teamspaces in [true, false] {
        let workspace = Resource::Workspace {
            members_can_create_teamspaces,
        };
        pairs.push((workspace, workspace));
    }
    for role in ROLES {
        for kind in UserKind::ALL {
            for mine in [false, true] {
                for theirs in [false, true] {
                    pairs.push((
                        Resource::Member {
                            role,
                            kind: *kind,
                            is_actor: mine,
                        },
                        Resource::Member {
                            role,
                            kind: *kind,
                            is_actor: theirs,
                        },
                    ));
                }
            }
        }
    }
    for (access, mine, theirs) in teamspace_rows() {
        pairs.push((
            Resource::Teamspace {
                access,
                membership: mine,
            },
            Resource::Teamspace {
                access,
                membership: theirs,
            },
        ));
    }
    for (mine, theirs, _) in space_pairs() {
        pairs.push((
            Resource::Pages { space: mine },
            Resource::Pages { space: theirs },
        ));
        for my_share in SHARES {
            for their_share in SHARES {
                for trashed in [false, true] {
                    pairs.push((
                        Resource::Page {
                            space: mine,
                            shared: my_share,
                            trashed,
                        },
                        Resource::Page {
                            space: theirs,
                            shared: their_share,
                            trashed,
                        },
                    ));
                }
            }
        }
    }
    for instance in [true, false] {
        for workspace in [true, false] {
            for creation in AgentCreation::ALL {
                let agents = Resource::Agents {
                    instance,
                    workspace,
                    creation: *creation,
                };
                pairs.push((agents, agents));
            }
        }
    }
    for kind in [UserKind::Agent, UserKind::Integration] {
        for theirs in [false, true] {
            pairs.push((
                Resource::Agent { kind, own: false },
                Resource::Agent { kind, own: theirs },
            ));
        }
    }
    pairs
}

/// Every action that applies inside a workspace, with every value of its parameter.
fn all_actions() -> Vec<Action> {
    let mut actions = vec![
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
    ];
    for role in ROLES {
        actions.push(Action::Invite { role });
        actions.push(Action::CreateInviteLink { role });
        actions.push(Action::ChangeRole { to: role });
        actions.push(Action::AddTeamspaceMember {
            newcomer: Newcomer::Person(role),
        });
    }
    for own in [false, true] {
        actions.push(Action::AddTeamspaceMember {
            newcomer: Newcomer::Delegate { own },
        });
    }
    for to in TeamspaceAccess::ALL {
        actions.push(Action::ChangeTeamspaceAccess { to: *to });
    }
    actions
}

#[test]
fn every_action_has_its_table() {
    let tabled: Vec<Action> = workspace_tables()
        .into_iter()
        .chain(member_tables())
        .chain(teamspace_tables())
        .map(|(action, _)| action)
        .chain([
            Action::ListPages,
            Action::CreatePage,
            Action::ViewPage,
            Action::EditPage,
            Action::TrashPage,
            Action::RestorePage,
            Action::DeletePage,
            Action::ListShares,
            Action::SharePage,
            Action::ChangePageSpace,
            Action::CreateAgent,
            Action::ViewAgent,
            Action::UpdateAgent,
            Action::TransferAgent,
            Action::DeleteAgent,
            Action::ListAgentTokens,
            Action::CreateAgentToken,
            Action::RevokeAgentToken,
        ])
        .collect();
    let all = all_actions();

    assert_eq!(all.len(), 60);
    for action in &all {
        assert_eq!(
            tabled.iter().filter(|tabled| *tabled == action).count(),
            1,
            "{action:?} needs one table"
        );
    }
    assert_eq!(tabled.len(), all.len());
}

/// What the owner is asked for an action of their delegate, as the tables above have it.
fn owners_part(action: Action) -> Option<Action> {
    match action {
        Action::JoinTeamspace => Some(Action::ViewTeamspace),
        Action::LeaveTeamspace => None,
        other => Some(other),
    }
}

#[test]
fn a_delegate_does_nothing_that_it_or_its_owner_could_not_alone() {
    for action in all_actions() {
        for (mine, theirs) in resource_pairs() {
            for role in ROLES {
                let allowed = authorize(delegate(false, Some(role), theirs), action, mine).is_ok();
                if !allowed {
                    continue;
                }
                let itself = Actor::person(false, false, Some(WorkspaceRole::Member));
                assert!(
                    authorize(itself, action, mine).is_ok(),
                    "{action:?} on {mine:?}: allowed beyond the delegate's own part"
                );
                if let Some(asked) = owners_part(action) {
                    let owner = Actor::person(false, false, Some(role));
                    assert!(
                        authorize(owner, asked, theirs).is_ok(),
                        "{action:?} on {theirs:?}: allowed beyond what its owner, {role}, may"
                    );
                }
            }
        }
    }
}

#[test]
fn owning_a_teamspace_gives_a_delegate_nothing_more() {
    let as_owner = |resource: Resource| match resource {
        Resource::Teamspace {
            access,
            membership: Some(_),
        } => Resource::Teamspace {
            access,
            membership: Some(TeamspaceRole::Owner),
        },
        Resource::Pages {
            space:
                PageSpace::Teamspace {
                    access,
                    membership: Some(_),
                },
        } => Resource::Pages {
            space: PageSpace::Teamspace {
                access,
                membership: Some(TeamspaceRole::Owner),
            },
        },
        Resource::Page {
            space:
                PageSpace::Teamspace {
                    access,
                    membership: Some(_),
                },
            shared,
            trashed,
        } => Resource::Page {
            space: PageSpace::Teamspace {
                access,
                membership: Some(TeamspaceRole::Owner),
            },
            shared,
            trashed,
        },
        other => other,
    };
    for action in all_actions() {
        for (mine, theirs) in resource_pairs() {
            for role in ROLES {
                let actor = delegate(false, Some(role), theirs);
                assert_eq!(
                    outcome(authorize(actor, action, as_owner(mine))),
                    outcome(authorize(actor, action, mine)),
                    "{action:?} on {mine:?} for an owner who is {role}"
                );
            }
        }
    }
}

#[test]
fn guests_and_agents_of_guests_see_no_other_member() {
    let owner_guest = delegate(
        false,
        Some(WorkspaceRole::Guest),
        Resource::Workspace {
            members_can_create_teamspaces: true,
        },
    );
    let owner_member = delegate(
        false,
        Some(WorkspaceRole::Member),
        Resource::Workspace {
            members_can_create_teamspaces: true,
        },
    );

    assert!(owner_guest.members_hidden());
    assert!(!owner_member.members_hidden());
    assert!(Actor::person(false, true, Some(WorkspaceRole::Guest)).members_hidden());
    assert!(!Actor::person(false, true, Some(WorkspaceRole::Admin)).members_hidden());
    assert!(!Actor::person(false, true, None).members_hidden());
}
