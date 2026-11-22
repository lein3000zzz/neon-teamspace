//! Teamspaces: who sees each kind, who joins and leaves, who manages their members, and
//! the default teamspace that stays as it is.

mod support;

use neon_app::workspaces::{NewTeamspace, TeamspaceUpdate, WorkspaceError, WorkspaceUpdate};
use neon_domain::auth::Principal;
use neon_domain::workspaces::{TeamspaceAccess, TeamspaceRole, WorkspaceRole};
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;

/// A workspace of Ada, its owner, with Bob as an admin, Cy and Dee as members and Gus as a
/// guest.
struct Team {
    h: Harness,
    id: Uuid,
    ada: Principal,
    bob: Principal,
    cy: Principal,
    dee: Principal,
    gus: Principal,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let dee = h.person("dee@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &dee, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;
    Team {
        h,
        id,
        ada,
        bob,
        cy,
        dee,
        gus,
    }
}

impl Team {
    async fn create(&self, by: &Principal, name: &str, access: TeamspaceAccess) -> Uuid {
        self.h
            .workspaces
            .create_teamspace(
                by,
                self.id,
                NewTeamspace {
                    name: name.to_owned(),
                    icon: None,
                    hue: None,
                    access,
                },
            )
            .await
            .expect("created")
            .teamspace
            .id
    }

    /// The names of the teamspaces `who` sees in full, and of those by name only.
    async fn seen_by(&self, who: &Principal) -> (Vec<String>, Vec<String>) {
        let list = self
            .h
            .workspaces
            .list_teamspaces(who, self.id)
            .await
            .expect("listed");
        (
            list.teamspaces
                .into_iter()
                .map(|entry| entry.teamspace.name)
                .collect(),
            list.hidden
                .into_iter()
                .map(|teamspace| teamspace.name)
                .collect(),
        )
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn each_kind_is_seen_by_whom_it_should_be(pool: PgPool) {
    let t = team(pool).await;
    t.create(&t.cy, "Open", TeamspaceAccess::Open).await;
    t.create(&t.cy, "Closed", TeamspaceAccess::Closed).await;
    t.create(&t.cy, "Secret", TeamspaceAccess::Private).await;

    let all: Vec<String> = ["General", "Closed", "Open", "Secret"]
        .map(String::from)
        .to_vec();
    assert_eq!(t.seen_by(&t.cy).await, (all, vec![]));
    let unjoined: Vec<String> = ["General", "Closed", "Open"].map(String::from).to_vec();
    assert_eq!(t.seen_by(&t.dee).await, (unjoined.clone(), vec![]));
    for manager in [&t.ada, &t.bob] {
        assert_eq!(
            t.seen_by(manager).await,
            (unjoined.clone(), vec!["Secret".to_owned()]),
            "owners and admins see a private teamspace by name only"
        );
    }
    assert_eq!(t.seen_by(&t.gus).await, (vec![], vec![]));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_private_teamspace_is_hidden_from_members_outside_it(pool: PgPool) {
    let t = team(pool).await;
    let secret = t.create(&t.cy, "Secret", TeamspaceAccess::Private).await;
    let w = &t.h.workspaces;

    for outsider in [&t.dee, &t.gus] {
        let got = w.teamspace(outsider, t.id, secret).await;
        assert!(matches!(got, Err(WorkspaceError::NotFound(_))), "{got:?}");
        let members = w.teamspace_members(outsider, t.id, secret).await;
        assert!(matches!(members, Err(WorkspaceError::NotFound(_))));
        let join = w.join_teamspace(outsider, t.id, secret).await;
        assert!(matches!(join, Err(WorkspaceError::NotFound(_))));
    }
    let full = w.teamspace(&t.bob, t.id, secret).await;
    assert!(
        matches!(full, Err(WorkspaceError::NotFound(_))),
        "an admin outside it sees its name only: {full:?}"
    );
    assert_eq!(
        w.teamspace_members(&t.bob, t.id, secret)
            .await
            .expect("an admin manages it")
            .len(),
        1
    );
    w.update_teamspace(
        &t.bob,
        t.id,
        secret,
        TeamspaceUpdate {
            name: Some("Renamed".into()),
            ..TeamspaceUpdate::default()
        },
    )
    .await
    .expect("an admin renames it");
    w.delete_teamspace(&t.ada, t.id, secret)
        .await
        .expect("an owner deletes it");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn open_teamspaces_are_joined_and_left_at_will(pool: PgPool) {
    let t = team(pool).await;
    let open = t.create(&t.cy, "Open", TeamspaceAccess::Open).await;
    let w = &t.h.workspaces;

    let joined = w.join_teamspace(&t.dee, t.id, open).await.expect("joined");
    assert_eq!(joined.role, Some(TeamspaceRole::Member));
    assert_eq!(joined.members, 2);
    let twice = w.join_teamspace(&t.dee, t.id, open).await;
    assert!(matches!(twice, Err(WorkspaceError::Forbidden(_))));
    w.leave_teamspace(&t.dee, t.id, open).await.expect("left");
    let not_in = w.leave_teamspace(&t.dee, t.id, open).await;
    assert!(matches!(not_in, Err(WorkspaceError::Forbidden(_))));
    let guest = w.join_teamspace(&t.gus, t.id, open).await;
    assert!(matches!(guest, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn closed_teamspaces_take_whom_their_owners_add(pool: PgPool) {
    let t = team(pool).await;
    let closed = t.create(&t.cy, "Closed", TeamspaceAccess::Closed).await;
    let w = &t.h.workspaces;

    let join = w.join_teamspace(&t.dee, t.id, closed).await;
    assert!(
        matches!(join, Err(WorkspaceError::Forbidden(_))),
        "{join:?}"
    );
    let by_outsider = w
        .add_teamspace_member(&t.dee, t.id, closed, t.dee.user.id, TeamspaceRole::Member)
        .await;
    assert!(matches!(by_outsider, Err(WorkspaceError::Forbidden(_))));

    let added = w
        .add_teamspace_member(&t.cy, t.id, closed, t.dee.user.id, TeamspaceRole::Member)
        .await
        .expect("its owner adds a member");
    assert_eq!(added.person.id, t.dee.user.id);
    let twice = w
        .add_teamspace_member(&t.cy, t.id, closed, t.dee.user.id, TeamspaceRole::Member)
        .await;
    assert!(matches!(twice, Err(WorkspaceError::AlreadyMember)));
    let guest = w
        .add_teamspace_member(&t.cy, t.id, closed, t.gus.user.id, TeamspaceRole::Member)
        .await;
    assert!(
        matches!(guest, Err(WorkspaceError::InvalidInput(_))),
        "{guest:?}"
    );
    let stranger = w
        .add_teamspace_member(&t.cy, t.id, closed, Uuid::now_v7(), TeamspaceRole::Member)
        .await;
    assert!(matches!(stranger, Err(WorkspaceError::NotFound(_))));
    let by_member = w
        .remove_teamspace_member(&t.dee, t.id, closed, t.cy.user.id)
        .await;
    assert!(matches!(by_member, Err(WorkspaceError::Forbidden(_))));

    w.join_teamspace(&t.bob, t.id, closed)
        .await
        .expect("an admin of the workspace joins it themselves");
    w.remove_teamspace_member(&t.bob, t.id, closed, t.dee.user.id)
        .await
        .expect("an admin removes a member");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_teamspace_keeps_an_owner_unless_the_workspace_managers_take_over(pool: PgPool) {
    let t = team(pool).await;
    let open = t.create(&t.cy, "Open", TeamspaceAccess::Open).await;
    let w = &t.h.workspaces;
    w.join_teamspace(&t.dee, t.id, open).await.expect("joined");

    let left = w.leave_teamspace(&t.cy, t.id, open).await;
    assert!(
        matches!(left, Err(WorkspaceError::LastTeamspaceOwner(_))),
        "{left:?}"
    );
    let demoted = w
        .change_teamspace_role(&t.cy, t.id, open, t.cy.user.id, TeamspaceRole::Member)
        .await;
    assert!(matches!(
        demoted,
        Err(WorkspaceError::LastTeamspaceOwner(_))
    ));

    let promoted = w
        .change_teamspace_role(&t.cy, t.id, open, t.dee.user.id, TeamspaceRole::Owner)
        .await
        .expect("a second owner");
    assert_eq!(promoted.role, TeamspaceRole::Owner);
    w.leave_teamspace(&t.cy, t.id, open)
        .await
        .expect("an owner leaves while another stays");

    w.remove_teamspace_member(&t.bob, t.id, open, t.dee.user.id)
        .await
        .expect("an admin of the workspace may leave it without owners");
    let members = w
        .teamspace_members(&t.ada, t.id, open)
        .await
        .expect("listed");
    assert!(members.is_empty());
    let entry = w
        .update_teamspace(
            &t.ada,
            t.id,
            open,
            TeamspaceUpdate {
                access: Some(TeamspaceAccess::Closed),
                ..TeamspaceUpdate::default()
            },
        )
        .await
        .expect("the owner of the workspace still manages it");
    assert_eq!(entry.teamspace.access, TeamspaceAccess::Closed);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_default_teamspace_stays_as_it_is(pool: PgPool) {
    let t = team(pool).await;
    let w = &t.h.workspaces;
    let list = w.list_teamspaces(&t.cy, t.id).await.expect("listed");
    let general = list.teamspaces[0].teamspace.clone();
    assert!(general.is_default());
    assert_eq!(list.teamspaces[0].members, 4, "everyone but the guest");

    let deleted = w.delete_teamspace(&t.ada, t.id, general.id).await;
    assert!(matches!(deleted, Err(WorkspaceError::Forbidden(_))));
    let left = w.leave_teamspace(&t.cy, t.id, general.id).await;
    assert!(matches!(left, Err(WorkspaceError::Forbidden(_))));
    let removed = w
        .remove_teamspace_member(&t.ada, t.id, general.id, t.cy.user.id)
        .await;
    assert!(matches!(removed, Err(WorkspaceError::Forbidden(_))));
    for access in [
        TeamspaceAccess::Private,
        TeamspaceAccess::Open,
        TeamspaceAccess::Closed,
    ] {
        let changed = w
            .update_teamspace(
                &t.ada,
                t.id,
                general.id,
                TeamspaceUpdate {
                    access: Some(access),
                    ..TeamspaceUpdate::default()
                },
            )
            .await;
        assert!(
            matches!(changed, Err(WorkspaceError::Forbidden(_))),
            "{access}"
        );
    }
    let renamed = w
        .update_teamspace(
            &t.bob,
            t.id,
            general.id,
            TeamspaceUpdate {
                name: Some("Everyone".into()),
                icon: Some(Some("🏠".into())),
                hue: Some(Some(90)),
                ..TeamspaceUpdate::default()
            },
        )
        .await
        .expect("renamed");
    assert_eq!(renamed.teamspace.name, "Everyone");
    assert_eq!(renamed.teamspace.hue(), 90);
    w.change_teamspace_role(&t.ada, t.id, general.id, t.cy.user.id, TeamspaceRole::Owner)
        .await
        .expect("roles change in the default teamspace");

    let other = t.create(&t.cy, "Open", TeamspaceAccess::Open).await;
    let made_default = w
        .update_teamspace(
            &t.cy,
            t.id,
            other,
            TeamspaceUpdate {
                access: Some(TeamspaceAccess::Default),
                ..TeamspaceUpdate::default()
            },
        )
        .await;
    assert!(matches!(made_default, Err(WorkspaceError::Forbidden(_))));
    let created_default = w
        .create_teamspace(
            &t.ada,
            t.id,
            NewTeamspace {
                name: "Second".into(),
                icon: None,
                hue: None,
                access: TeamspaceAccess::Default,
            },
        )
        .await;
    assert!(matches!(
        created_default,
        Err(WorkspaceError::InvalidInput(_))
    ));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn who_creates_teamspaces_follows_the_workspace_setting(pool: PgPool) {
    let t = team(pool).await;
    let w = &t.h.workspaces;
    t.create(&t.cy, "By a member", TeamspaceAccess::Open).await;

    w.update_workspace(
        &t.bob,
        t.id,
        WorkspaceUpdate {
            members_can_create_teamspaces: Some(false),
            ..WorkspaceUpdate::default()
        },
    )
    .await
    .expect("updated");
    let new = || NewTeamspace {
        name: "Another".into(),
        icon: None,
        hue: None,
        access: TeamspaceAccess::Closed,
    };
    let member = w.create_teamspace(&t.cy, t.id, new()).await;
    assert!(matches!(member, Err(WorkspaceError::Forbidden(_))));
    let guest = w.create_teamspace(&t.gus, t.id, new()).await;
    assert!(matches!(guest, Err(WorkspaceError::Forbidden(_))));
    let created = w
        .create_teamspace(&t.bob, t.id, new())
        .await
        .expect("an admin still creates them");
    assert_eq!(created.role, Some(TeamspaceRole::Owner));
    for bad in [
        NewTeamspace {
            name: " ".into(),
            ..new()
        },
        NewTeamspace {
            hue: Some(400),
            ..new()
        },
    ] {
        let refused = w.create_teamspace(&t.bob, t.id, bad).await;
        assert!(matches!(refused, Err(WorkspaceError::InvalidInput(_))));
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn teamspaces_belong_to_their_workspace(pool: PgPool) {
    let t = team(pool).await;
    let w = &t.h.workspaces;
    let open = t.create(&t.cy, "Open", TeamspaceAccess::Open).await;
    let elsewhere = t.h.workspace(&t.dee, "Elsewhere").await.workspace.id;

    let crossed = w.teamspace(&t.dee, elsewhere, open).await;
    assert!(
        matches!(crossed, Err(WorkspaceError::NotFound(_))),
        "{crossed:?}"
    );
    let deleted = w.delete_teamspace(&t.dee, elsewhere, open).await;
    assert!(matches!(deleted, Err(WorkspaceError::NotFound(_))));
    let leaving = w.remove_member(&t.ada, t.id, t.cy.user.id).await;
    assert!(leaving.is_ok());
    let members = w
        .teamspace_members(&t.ada, t.id, open)
        .await
        .expect("listed");
    assert!(
        members.is_empty(),
        "leaving the workspace left its teamspaces"
    );
}
