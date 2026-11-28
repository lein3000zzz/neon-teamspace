//! Workspaces: creating them with their default teamspace, slugs, who may create them,
//! changing and deleting them, and their members' roles, with the last owner kept.

use crate::support;

use neon_app::auth::{AuthError, ProfileUpdate};
use neon_app::workspaces::{DEFAULT_TEAMSPACE_NAME, NewWorkspace, WorkspaceError, WorkspaceUpdate};
use neon_domain::auth::{Credential, Principal, Role};
use neon_domain::hue::hue_for_uuid;
use neon_domain::workspaces::{TeamspaceAccess, TeamspaceRole, WorkspaceRole};
use serde_json::json;
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;

fn named(name: &str) -> NewWorkspace {
    NewWorkspace {
        name: name.to_owned(),
        ..NewWorkspace::default()
    }
}

/// The same user, through an API token.
fn as_token(principal: &Principal) -> Principal {
    Principal {
        user: principal.user.clone(),
        credential: Credential::Token {
            id: Uuid::now_v7(),
            name: "ci".into(),
            scopes: vec![neon_domain::auth::Scope::Write],
            workspace_id: None,
        },
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn creating_a_workspace_makes_its_default_teamspace_and_owner_together(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;

    let created = h
        .workspaces
        .create_workspace(
            &ada,
            NewWorkspace {
                name: "  Design Team ".into(),
                icon: Some("🚀".into()),
                hue: Some(200),
                ..NewWorkspace::default()
            },
        )
        .await
        .expect("created");

    let workspace = &created.workspace;
    assert_eq!(workspace.name, "Design Team");
    assert_eq!(workspace.slug, "design-team");
    assert_eq!(workspace.icon.as_deref(), Some("🚀"));
    assert_eq!(workspace.hue(), 200);
    assert!(workspace.members_can_create_teamspaces);
    assert_eq!(created.role, WorkspaceRole::Owner);
    assert_eq!(created.members, Some(1));
    assert_eq!(created.limits.members, None);

    let list = h
        .workspaces
        .list_teamspaces(&ada, workspace.id)
        .await
        .expect("listed");
    assert_eq!(list.teamspaces.len(), 1);
    let general = &list.teamspaces[0];
    assert_eq!(general.teamspace.name, DEFAULT_TEAMSPACE_NAME);
    assert_eq!(general.teamspace.access, TeamspaceAccess::Default);
    assert_eq!(general.role, Some(TeamspaceRole::Owner));
    assert_eq!(general.members, 1);
    assert_eq!(
        h.workspaces
            .list_workspaces(&ada)
            .await
            .expect("listed")
            .into_iter()
            .map(|(workspace, role)| (workspace.slug, role))
            .collect::<Vec<_>>(),
        [("design-team".to_owned(), WorkspaceRole::Owner)]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_failed_creation_leaves_nothing_behind(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    h.workspace(&ada, "Design").await;

    let taken = h
        .workspaces
        .create_workspace(
            &ada,
            NewWorkspace {
                name: "Other".into(),
                slug: Some("design".into()),
                ..NewWorkspace::default()
            },
        )
        .await;

    assert!(matches!(taken, Err(WorkspaceError::SlugTaken)), "{taken:?}");
    assert_eq!(h.scalar::<i64>("SELECT count(*) FROM workspaces").await, 1);
    assert_eq!(h.scalar::<i64>("SELECT count(*) FROM teamspaces").await, 1);
    assert_eq!(
        h.scalar::<i64>("SELECT count(*) FROM workspace_members")
            .await,
        1
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn slugs_come_from_the_name_with_a_suffix_when_taken(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;

    let first = h.workspace(&ada, "Design").await.workspace;
    let second = h.workspace(&ada, "design!").await.workspace;
    let reserved = h.workspace(&ada, "Settings").await.workspace;
    let foreign = h.workspace(&ada, "日本語").await.workspace;
    let typed = h
        .workspaces
        .create_workspace(
            &ada,
            NewWorkspace {
                name: "Anything".into(),
                slug: Some("Team-42".into()),
                ..NewWorkspace::default()
            },
        )
        .await
        .expect("created")
        .workspace;

    assert_eq!(first.slug, "design");
    let suffix = second.slug.strip_prefix("design-").expect("a suffix");
    assert_eq!(suffix.len(), 6, "{}", second.slug);
    assert!(reserved.slug.starts_with("settings-"), "{}", reserved.slug);
    assert_eq!(foreign.slug, "workspace");
    assert_eq!(typed.slug, "team-42");
    for bad in ["admin", "no spaces", "-dash"] {
        let refused = h
            .workspaces
            .create_workspace(
                &ada,
                NewWorkspace {
                    name: "Anything".into(),
                    slug: Some(bad.into()),
                    ..NewWorkspace::default()
                },
            )
            .await;
        assert!(
            matches!(refused, Err(WorkspaceError::InvalidInput(_))),
            "{bad}: {refused:?}"
        );
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn names_icons_and_hues_are_checked(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;

    for new in [
        named("   "),
        named(&"x".repeat(101)),
        NewWorkspace {
            icon: Some("x".repeat(17)),
            ..named("Design")
        },
        NewWorkspace {
            hue: Some(360),
            ..named("Design")
        },
    ] {
        let refused = h.workspaces.create_workspace(&ada, new.clone()).await;
        assert!(
            matches!(refused, Err(WorkspaceError::InvalidInput(_))),
            "{new:?}: {refused:?}"
        );
    }
    let plain = h.workspace(&ada, "Plain").await.workspace;
    assert_eq!(plain.hue(), hue_for_uuid(plain.id));
    assert_eq!(plain.icon, None);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn creation_can_be_left_to_instance_admins(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let ada = h.person("ada@example.com").await;
    assert!(h.workspaces.may_create_workspace(&ada));
    h.set("workspaces.creation", json!("admins")).await;

    assert!(!h.workspaces.may_create_workspace(&ada));
    assert!(h.workspaces.may_create_workspace(&admin));
    let refused = h.workspaces.create_workspace(&ada, named("Design")).await;
    assert!(
        matches!(refused, Err(WorkspaceError::Forbidden(_))),
        "{refused:?}"
    );
    h.workspace(&admin, "Design").await;

    h.set("workspaces.creation", json!("anyone")).await;
    assert!(h.workspaces.may_create_workspace(&ada));
    h.workspace(&ada, "Mine").await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn outsiders_learn_nothing_of_a_workspace_instance_admins_included(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let admin = h.admin().await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;

    for outsider in [&admin, &bob] {
        let got = h.workspaces.workspace(outsider, id).await;
        assert!(matches!(got, Err(WorkspaceError::NotFound(_))), "{got:?}");
        let members = h.workspaces.list_members(outsider, id).await;
        assert!(matches!(members, Err(WorkspaceError::NotFound(_))));
        let teamspaces = h.workspaces.list_teamspaces(outsider, id).await;
        assert!(matches!(teamspaces, Err(WorkspaceError::NotFound(_))));
        let update = h
            .workspaces
            .update_workspace(outsider, id, WorkspaceUpdate::default())
            .await;
        assert!(matches!(update, Err(WorkspaceError::NotFound(_))));
    }
    let missing = h.workspaces.workspace(&ada, Uuid::now_v7()).await;
    assert!(matches!(missing, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn owners_and_admins_change_a_workspace_and_members_do_not(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.workspace(&ada, "Taken").await;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;

    let updated = h
        .workspaces
        .update_workspace(
            &bob,
            id,
            WorkspaceUpdate {
                name: Some("Design Studio".into()),
                icon: Some(Some("DS".into())),
                hue: Some(Some(30)),
                members_can_create_teamspaces: Some(false),
                ..WorkspaceUpdate::default()
            },
        )
        .await
        .expect("an admin updates");
    assert_eq!(updated.workspace.name, "Design Studio");
    assert_eq!(
        updated.workspace.slug, "design",
        "a new name keeps the slug"
    );
    assert_eq!(updated.workspace.icon.as_deref(), Some("DS"));
    assert_eq!(updated.workspace.hue(), 30);
    assert!(!updated.workspace.members_can_create_teamspaces);
    assert_eq!(updated.role, WorkspaceRole::Admin);
    assert_eq!(updated.members, Some(3));

    let cleared = h
        .workspaces
        .update_workspace(
            &ada,
            id,
            WorkspaceUpdate {
                icon: Some(None),
                hue: Some(None),
                slug: Some("studio".into()),
                ..WorkspaceUpdate::default()
            },
        )
        .await
        .expect("the owner updates");
    assert_eq!(cleared.workspace.icon, None);
    assert_eq!(cleared.workspace.hue_override, None);
    assert_eq!(cleared.workspace.slug, "studio");

    let taken = h
        .workspaces
        .update_workspace(
            &ada,
            id,
            WorkspaceUpdate {
                slug: Some("taken".into()),
                ..WorkspaceUpdate::default()
            },
        )
        .await;
    assert!(matches!(taken, Err(WorkspaceError::SlugTaken)), "{taken:?}");
    let refused = h
        .workspaces
        .update_workspace(&cy, id, WorkspaceUpdate::default())
        .await;
    assert!(
        matches!(refused, Err(WorkspaceError::Forbidden(_))),
        "{refused:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn only_an_owner_deletes_a_workspace_typing_its_name_at_a_browser(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.workspaces
        .invite(&ada, id, cy.user.address(), WorkspaceRole::Member)
        .await
        .expect("a pending invitation");
    h.workspaces
        .create_invite_link(
            &ada,
            id,
            neon_app::workspaces::LinkSpec {
                role: WorkspaceRole::Member,
                max_uses: None,
                expires_at: None,
            },
        )
        .await
        .expect("a link");
    h.workspaces
        .create_teamspace(
            &bob,
            id,
            neon_app::workspaces::NewTeamspace {
                name: "Private".into(),
                icon: None,
                hue: None,
                access: TeamspaceAccess::Private,
            },
        )
        .await
        .expect("a teamspace");

    let by_admin = h.workspaces.delete_workspace(&bob, id, "Design").await;
    assert!(matches!(by_admin, Err(WorkspaceError::Forbidden(_))));
    let mistyped = h.workspaces.delete_workspace(&ada, id, "design").await;
    assert!(matches!(
        mistyped,
        Err(WorkspaceError::ConfirmationMismatch)
    ));
    let by_token = h
        .workspaces
        .delete_workspace(&as_token(&ada), id, "Design")
        .await;
    assert!(matches!(by_token, Err(WorkspaceError::Forbidden(_))));

    h.workspaces
        .delete_workspace(&ada, id, " Design ")
        .await
        .expect("deleted");
    for query in [
        "SELECT count(*) FROM workspaces",
        "SELECT count(*) FROM workspace_members",
        "SELECT count(*) FROM teamspaces",
        "SELECT count(*) FROM teamspace_members",
        "SELECT count(*) FROM workspace_invitations",
        "SELECT count(*) FROM workspace_invite_links",
    ] {
        assert_eq!(h.scalar::<i64>(query).await, 0, "{query}");
    }
}

async fn change(
    h: &Harness,
    by: &Principal,
    workspace_id: Uuid,
    who: &Principal,
    role: WorkspaceRole,
) -> Result<neon_domain::workspaces::Member, WorkspaceError> {
    h.workspaces
        .change_member_role(by, workspace_id, who.user.id, role)
        .await
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn roles_change_as_the_rules_allow(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    let admin_on_owner = change(&h, &bob, id, &ada, WorkspaceRole::Member).await;
    assert!(matches!(admin_on_owner, Err(WorkspaceError::Forbidden(_))));
    let admin_makes_owner = change(&h, &bob, id, &cy, WorkspaceRole::Owner).await;
    assert!(matches!(
        admin_makes_owner,
        Err(WorkspaceError::Forbidden(_))
    ));
    let member_changes = change(&h, &cy, id, &cy, WorkspaceRole::Admin).await;
    assert!(matches!(member_changes, Err(WorkspaceError::Forbidden(_))));

    let made = change(&h, &bob, id, &cy, WorkspaceRole::Admin)
        .await
        .expect("an admin makes an admin");
    assert_eq!(made.role, WorkspaceRole::Admin);
    assert_eq!(made.person.id, cy.user.id);
    change(&h, &ada, id, &cy, WorkspaceRole::Owner)
        .await
        .expect("an owner makes an owner");
    change(&h, &cy, id, &ada, WorkspaceRole::Member)
        .await
        .expect("an owner demotes another owner");
    let missing = h
        .workspaces
        .change_member_role(&ada, id, Uuid::now_v7(), WorkspaceRole::Member)
        .await;
    assert!(matches!(missing, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_last_owner_is_kept(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;

    let demoted = h
        .workspaces
        .change_member_role(&ada, id, ada.user.id, WorkspaceRole::Admin)
        .await;
    assert!(
        matches!(demoted, Err(WorkspaceError::LastOwner(_))),
        "{demoted:?}"
    );
    let left = h.workspaces.leave_workspace(&ada, id).await;
    assert!(matches!(left, Err(WorkspaceError::LastOwner(_))));
    let removed_self = h.workspaces.remove_member(&ada, id, ada.user.id).await;
    assert!(matches!(removed_self, Err(WorkspaceError::Forbidden(_))));

    h.workspaces
        .change_member_role(&ada, id, bob.user.id, WorkspaceRole::Owner)
        .await
        .expect("a second owner");
    h.workspaces
        .leave_workspace(&ada, id)
        .await
        .expect("an owner leaves while another stays");
    let members = h.workspaces.list_members(&bob, id).await.expect("listed");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].role, WorkspaceRole::Owner);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn two_owners_demoting_themselves_at_once_leave_one(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.workspaces
        .change_member_role(&ada, id, bob.user.id, WorkspaceRole::Owner)
        .await
        .expect("a second owner");

    let (first, second) = tokio::join!(
        h.workspaces
            .change_member_role(&ada, id, ada.user.id, WorkspaceRole::Member),
        h.workspaces
            .change_member_role(&bob, id, bob.user.id, WorkspaceRole::Member),
    );

    let refused = [&first, &second]
        .into_iter()
        .filter(|result| matches!(result, Err(WorkspaceError::LastOwner(_))))
        .count();
    assert_eq!(refused, 1, "{first:?} {second:?}");
    let owners: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workspace_members WHERE workspace_id = $1 AND role = 'owner'",
    )
    .bind(id)
    .fetch_one(&h.pool)
    .await
    .expect("counted");
    assert_eq!(owners, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_remove_anyone_but_owners(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;

    let owner = h.workspaces.remove_member(&bob, id, ada.user.id).await;
    assert!(matches!(owner, Err(WorkspaceError::Forbidden(_))));
    let by_member = h.workspaces.remove_member(&cy, id, bob.user.id).await;
    assert!(matches!(by_member, Err(WorkspaceError::Forbidden(_))));
    h.workspaces
        .remove_member(&bob, id, cy.user.id)
        .await
        .expect("an admin removes a member");
    let gone = h.workspaces.workspace(&cy, id).await;
    assert!(matches!(gone, Err(WorkspaceError::NotFound(_))));
    assert_eq!(
        h.scalar::<i64>("SELECT count(*) FROM teamspace_members")
            .await,
        2,
        "the removed member left the default teamspace too"
    );
    h.workspaces
        .remove_member(&ada, id, bob.user.id)
        .await
        .expect("an owner removes an admin");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn guests_see_only_themselves_and_no_teamspace(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;

    let seen = h.workspaces.list_members(&gus, id).await.expect("listed");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].person.id, gus.user.id);
    assert_eq!(seen[0].role, WorkspaceRole::Guest);
    let all = h.workspaces.list_members(&ada, id).await.expect("listed");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].role, WorkspaceRole::Owner, "owners first");
    let teamspaces = h
        .workspaces
        .list_teamspaces(&gus, id)
        .await
        .expect("listed");
    assert!(teamspaces.teamspaces.is_empty() && teamspaces.hidden.is_empty());
    let guessed = h
        .workspaces
        .change_member_role(&gus, id, ada.user.id, WorkspaceRole::Guest)
        .await;
    assert!(matches!(guessed, Err(WorkspaceError::NotFound(_))));
    let removed = h.workspaces.remove_member(&gus, id, ada.user.id).await;
    assert!(matches!(removed, Err(WorkspaceError::NotFound(_))));
    let nobody = h.workspaces.remove_member(&gus, id, Uuid::now_v7()).await;
    assert_eq!(
        nobody.expect_err("refused").to_string(),
        removed.expect_err("refused").to_string(),
        "someone there and nobody look the same"
    );
    let details = h.workspaces.workspace(&gus, id).await.expect("seen");
    assert_eq!(details.role, WorkspaceRole::Guest);
    assert_eq!(details.members, None, "a guest learns no head count");
    assert_eq!(
        h.workspaces
            .workspace(&ada, id)
            .await
            .expect("seen")
            .members,
        Some(2)
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn becoming_a_guest_leaves_every_teamspace_and_back_joins_the_default(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    let open = h
        .workspaces
        .create_teamspace(
            &cy,
            id,
            neon_app::workspaces::NewTeamspace {
                name: "Open".into(),
                icon: None,
                hue: None,
                access: TeamspaceAccess::Open,
            },
        )
        .await
        .expect("created")
        .teamspace;
    let in_teamspaces = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM teamspace_members WHERE user_id = $1")
            .bind(cy.user.id)
            .fetch_one(&h.pool)
            .await
            .expect("counted")
    };
    assert_eq!(in_teamspaces().await, 2);

    h.workspaces
        .change_member_role(&ada, id, cy.user.id, WorkspaceRole::Guest)
        .await
        .expect("made a guest");
    assert_eq!(in_teamspaces().await, 0);
    let refused = h.workspaces.teamspace(&cy, id, open.id).await;
    assert!(matches!(refused, Err(WorkspaceError::NotFound(_))));

    h.workspaces
        .change_member_role(&ada, id, cy.user.id, WorkspaceRole::Member)
        .await
        .expect("a member again");
    assert_eq!(
        in_teamspaces().await,
        1,
        "back in the default teamspace only"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_owner_transfers_ownership_and_becomes_an_admin(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;

    let by_admin = h.workspaces.transfer_ownership(&bob, id, gus.user.id).await;
    assert!(matches!(by_admin, Err(WorkspaceError::Forbidden(_))));
    let to_self = h.workspaces.transfer_ownership(&ada, id, ada.user.id).await;
    assert!(matches!(to_self, Err(WorkspaceError::Forbidden(_))));
    let by_token = h
        .workspaces
        .transfer_ownership(&as_token(&ada), id, bob.user.id)
        .await;
    assert!(matches!(by_token, Err(WorkspaceError::Forbidden(_))));

    h.workspaces
        .transfer_ownership(&ada, id, gus.user.id)
        .await
        .expect("transferred to the guest");
    let roles: Vec<(Uuid, WorkspaceRole)> = h
        .workspaces
        .list_members(&gus, id)
        .await
        .expect("listed")
        .into_iter()
        .map(|member| (member.person.id, member.role))
        .collect();
    assert!(roles.contains(&(gus.user.id, WorkspaceRole::Owner)));
    assert!(roles.contains(&(ada.user.id, WorkspaceRole::Admin)));
    let general = h
        .workspaces
        .list_teamspaces(&gus, id)
        .await
        .expect("listed");
    assert_eq!(
        general.teamspaces[0].role,
        Some(TeamspaceRole::Member),
        "the guest joined the default teamspace as they stopped being one"
    );
}

fn pinning(hue: Option<i64>) -> ProfileUpdate {
    ProfileUpdate {
        hue_override: Some(hue),
        ..ProfileUpdate::default()
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_pinned_user_hue_shows_in_member_lists(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;

    let renamed = h
        .auth
        .update_profile(&ada, pinning(Some(123)))
        .await
        .expect("pinned");
    assert_eq!(renamed.hue(), 123);
    let members = h.workspaces.list_members(&ada, id).await.expect("listed");
    assert_eq!(members[0].person.hue(), 123);
    let cleared = h
        .auth
        .update_profile(&ada, pinning(None))
        .await
        .expect("cleared");
    assert_eq!(cleared.hue(), hue_for_uuid(ada.user.id));
    assert_eq!(cleared.role, Role::User);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_profile_change_with_a_bad_part_changes_nothing(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;

    for bad in [
        ProfileUpdate {
            display_name: Some("Ada Lovelace".into()),
            hue_override: Some(Some(999)),
        },
        ProfileUpdate {
            display_name: Some("x".repeat(101)),
            hue_override: Some(Some(40)),
        },
    ] {
        let refused = h.auth.update_profile(&ada, bad).await;
        assert!(
            matches!(refused, Err(AuthError::InvalidInput(_))),
            "{refused:?}"
        );
    }
    let (name, hue): (String, Option<i16>) =
        sqlx::query_as("SELECT display_name, hue FROM users WHERE id = $1")
            .bind(ada.user.id)
            .fetch_one(&h.pool)
            .await
            .expect("read");
    assert_eq!((name.as_str(), hue), ("ada", None), "nothing was saved");

    let both = h
        .auth
        .update_profile(
            &ada,
            ProfileUpdate {
                display_name: Some("Ada Lovelace".into()),
                hue_override: Some(Some(40)),
            },
        )
        .await
        .expect("changed");
    assert_eq!(
        (both.display_name.as_str(), both.hue()),
        ("Ada Lovelace", 40)
    );
    let nothing = h
        .auth
        .update_profile(&ada, ProfileUpdate::default())
        .await
        .expect("nothing to change");
    assert_eq!(nothing.id, ada.user.id);
}
