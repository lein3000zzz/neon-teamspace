//! What [`person_sees_page`] promises the queries that filter pages in the database, proved
//! over every role, space and share: a page shows through its space or through any share,
//! and an agent or integration sees what it and its owner both see, each as a person.

use super::*;
use crate::pages::PageAccess;

const ROLES: [Option<WorkspaceRole>; 5] = [
    None,
    Some(WorkspaceRole::Owner),
    Some(WorkspaceRole::Admin),
    Some(WorkspaceRole::Member),
    Some(WorkspaceRole::Guest),
];

const SHARES: [Option<PageAccess>; 3] = [None, Some(PageAccess::View), Some(PageAccess::Edit)];

/// Where nobody stands but through a share: someone else's private pages.
const NOWHERE: PageSpace = PageSpace::Private { own: false };

/// Every space as someone may stand to it.
fn spaces() -> Vec<PageSpace> {
    let mut spaces = vec![
        PageSpace::Private { own: true },
        PageSpace::Private { own: false },
    ];
    for access in TeamspaceAccess::ALL {
        for membership in [
            None,
            Some(TeamspaceRole::Owner),
            Some(TeamspaceRole::Member),
        ] {
            spaces.push(PageSpace::Teamspace {
                access: *access,
                membership,
            });
        }
    }
    spaces
}

fn page(space: PageSpace, shared: Option<PageAccess>) -> Resource {
    Resource::Page {
        space,
        shared,
        trashed: false,
    }
}

#[test]
fn it_is_what_the_rules_answer_a_person() {
    for role in ROLES {
        for space in spaces() {
            for shared in SHARES {
                for via_session in [false, true] {
                    let actor = Actor::person(false, via_session, role);
                    assert_eq!(
                        authorize(actor, Action::ViewPage, page(space, shared)).is_ok(),
                        person_sees_page(role, space, shared),
                        "{role:?} {space:?} {shared:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_page_shows_through_its_space_or_through_any_share() {
    for role in ROLES {
        for space in spaces() {
            for shared in SHARES {
                let through_space = person_sees_page(role, space, None);
                let through_a_share =
                    shared.is_some() && person_sees_page(role, NOWHERE, Some(PageAccess::View));
                assert_eq!(
                    person_sees_page(role, space, shared),
                    through_space || through_a_share,
                    "{role:?} {space:?} {shared:?}"
                );
            }
        }
    }
}

#[test]
fn an_agent_sees_what_it_and_its_owner_both_see() {
    for role in ROLES {
        for owner_role in ROLES {
            for mine in spaces() {
                for theirs in spaces() {
                    for shared in SHARES {
                        for owner_shared in SHARES {
                            let actor = Actor {
                                instance_admin: false,
                                via_session: false,
                                role,
                                on_behalf_of: Some(OwnerStanding {
                                    role: owner_role,
                                    resource: page(theirs, owner_shared),
                                }),
                            };
                            assert_eq!(
                                authorize(actor, Action::ViewPage, page(mine, shared)).is_ok(),
                                person_sees_page(role, mine, shared)
                                    && person_sees_page(owner_role, theirs, owner_shared),
                                "{role:?} {mine:?} {shared:?} for {owner_role:?} {theirs:?} \
                                 {owner_shared:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
