//! What the schema itself holds to: unique slugs, one default teamspace per workspace,
//! one membership and one pending invitation per account, teamspace members who are in
//! the workspace, and everything gone with its workspace.

use neon_domain::auth::{Email, Role, User, UserKind};
use neon_domain::workspaces::{
    Invitation, InvitationState, InviteLink, Teamspace, TeamspaceAccess, TeamspaceRole, Workspace,
    WorkspaceRole,
};
use neon_store::{is_unique_violation, teamspaces, users, workspace_invites, workspaces};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a valid time")
}

async fn user(pool: &PgPool, email: &str) -> Uuid {
    let address = Email::parse(email).expect("valid");
    let user = User {
        id: Uuid::now_v7(),
        kind: UserKind::Human,
        email: Some(address.clone()),
        display_name: "Someone".into(),
        role: Role::User,
        hue_override: Some(10),
        email_verified_at: Some(now()),
        disabled_at: None,
        created_at: now(),
        updated_at: now(),
        workspace_id: None,
        owner_id: None,
    };
    users::insert(pool, &user, &address, "hash")
        .await
        .expect("saved");
    assert_eq!(
        users::by_id(pool, user.id)
            .await
            .expect("read")
            .map(|user| user.hue_override),
        Some(Some(10))
    );
    user.id
}

fn workspace(slug: &str) -> Workspace {
    Workspace {
        id: Uuid::now_v7(),
        name: "Design".into(),
        slug: slug.into(),
        icon: None,
        hue_override: Some(359),
        members_can_create_teamspaces: true,
        agents_enabled: true,
        agent_creation: neon_domain::agents::AgentCreation::Members,
        created_by: None,
        created_at: now(),
        updated_at: now(),
    }
}

fn teamspace(workspace_id: Uuid, access: TeamspaceAccess) -> Teamspace {
    Teamspace {
        id: Uuid::now_v7(),
        workspace_id,
        name: "Team".into(),
        icon: Some("🚀".into()),
        hue_override: None,
        access,
        created_by: None,
        created_at: now(),
        updated_at: now(),
    }
}

fn invitation(workspace_id: Uuid, user_id: Uuid) -> Invitation {
    Invitation {
        id: Uuid::now_v7(),
        workspace_id,
        user_id,
        role: WorkspaceRole::Member,
        invited_by: None,
        created_at: now(),
        expires_at: now() + time::Duration::days(14),
        state: InvitationState::Pending,
        decided_at: None,
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn slugs_are_unique_and_well_formed(pool: PgPool) {
    let first = workspace("design");
    assert!(workspaces::insert(&pool, &first).await.expect("saved"));
    assert_eq!(
        workspaces::get(&pool, first.id).await.expect("read"),
        Some(first.clone())
    );

    assert!(
        !workspaces::insert(&pool, &workspace("design"))
            .await
            .expect("skipped"),
        "a taken slug saves nothing"
    );
    let mut renamed = workspace("other");
    workspaces::insert(&pool, &renamed).await.expect("saved");
    renamed.slug = "design".into();
    let taken = workspaces::update(&pool, &renamed)
        .await
        .expect_err("refused");
    assert!(
        is_unique_violation(&taken, workspaces::SLUG_TAKEN),
        "{taken}"
    );

    for bad in ["Design", "a--b", "-a", "a b", ""] {
        let refused = workspaces::insert(&pool, &workspace(bad)).await;
        assert!(refused.is_err(), "{bad:?}");
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_workspace_has_one_default_teamspace(pool: PgPool) {
    let first = workspace("design");
    let second = workspace("ops");
    workspaces::insert(&pool, &first).await.expect("saved");
    workspaces::insert(&pool, &second).await.expect("saved");

    teamspaces::insert(&pool, &teamspace(first.id, TeamspaceAccess::Default))
        .await
        .expect("saved");
    let again = teamspaces::insert(&pool, &teamspace(first.id, TeamspaceAccess::Default))
        .await
        .expect_err("refused");
    assert!(
        is_unique_violation(&again, "teamspaces_one_default"),
        "{again}"
    );
    teamspaces::insert(&pool, &teamspace(second.id, TeamspaceAccess::Default))
        .await
        .expect("another workspace has its own");
    teamspaces::insert(&pool, &teamspace(first.id, TeamspaceAccess::Open))
        .await
        .expect("as many others as wanted");
    let default = teamspaces::list_with_membership(&pool, first.id, Uuid::now_v7())
        .await
        .expect("listed");
    assert_eq!(default.len(), 2);
    assert_eq!(
        default[0].0.access,
        TeamspaceAccess::Default,
        "the default one first"
    );
    assert!(
        !teamspaces::delete(&pool, first.id, default[0].0.id)
            .await
            .expect("skipped"),
        "the default teamspace is not deleted on its own"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_account_is_in_a_workspace_and_a_teamspace_once(pool: PgPool) {
    let ada = user(&pool, "ada@example.com").await;
    let design = workspace("design");
    workspaces::insert(&pool, &design).await.expect("saved");
    let team = teamspace(design.id, TeamspaceAccess::Open);
    teamspaces::insert(&pool, &team).await.expect("saved");

    let outside = teamspaces::add_member(&pool, &team, ada, TeamspaceRole::Member, now()).await;
    assert!(
        outside.is_err(),
        "a teamspace's members are in its workspace"
    );

    workspaces::add_member(&pool, design.id, ada, WorkspaceRole::Owner, now())
        .await
        .expect("added");
    let twice = workspaces::add_member(&pool, design.id, ada, WorkspaceRole::Member, now())
        .await
        .expect_err("refused");
    assert!(
        is_unique_violation(&twice, workspaces::ALREADY_MEMBER),
        "{twice}"
    );
    teamspaces::add_member(&pool, &team, ada, TeamspaceRole::Owner, now())
        .await
        .expect("added");
    let again = teamspaces::add_member(&pool, &team, ada, TeamspaceRole::Member, now())
        .await
        .expect_err("refused");
    assert!(
        is_unique_violation(&again, teamspaces::ALREADY_MEMBER),
        "{again}"
    );

    workspaces::remove_member(&pool, design.id, ada)
        .await
        .expect("removed");
    assert_eq!(
        teamspaces::count_members(&pool, team.id)
            .await
            .expect("counted"),
        0
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_account_has_one_pending_invitation_per_workspace(pool: PgPool) {
    let ada = user(&pool, "ada@example.com").await;
    let design = workspace("design");
    let ops = workspace("ops");
    workspaces::insert(&pool, &design).await.expect("saved");
    workspaces::insert(&pool, &ops).await.expect("saved");

    let first = invitation(design.id, ada);
    workspace_invites::insert(&pool, &first)
        .await
        .expect("saved");
    let twice = workspace_invites::insert(&pool, &invitation(design.id, ada))
        .await
        .expect_err("refused");
    assert!(
        is_unique_violation(&twice, workspace_invites::ALREADY_INVITED),
        "{twice}"
    );
    workspace_invites::insert(&pool, &invitation(ops.id, ada))
        .await
        .expect("another workspace may invite too");

    assert_eq!(
        workspace_invites::decide(&pool, design.id, first.id, InvitationState::Declined, now())
            .await
            .expect("declined"),
        Some(first.user_id)
    );
    workspace_invites::insert(&pool, &invitation(design.id, ada))
        .await
        .expect("a declined invitation leaves room for a new one");
    let owner = Invitation {
        role: WorkspaceRole::Owner,
        ..invitation(ops.id, user(&pool, "bob@example.com").await)
    };
    assert!(
        workspace_invites::insert(&pool, &owner).await.is_err(),
        "nobody is invited as an owner"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_link_takes_no_more_uses_than_it_has(pool: PgPool) {
    let design = workspace("design");
    workspaces::insert(&pool, &design).await.expect("saved");
    let link = InviteLink {
        id: Uuid::now_v7(),
        workspace_id: design.id,
        role: WorkspaceRole::Guest,
        max_uses: 1,
        uses: 0,
        created_by: None,
        created_at: now(),
        expires_at: now() + time::Duration::days(1),
        revoked_at: None,
        revoked_by: None,
        token_kept: false,
    };
    workspace_invites::insert_link(&pool, &link, b"hash", None)
        .await
        .expect("saved");

    assert!(
        workspace_invites::consume_link(&pool, link.id, now())
            .await
            .expect("taken")
    );
    assert!(
        !workspace_invites::consume_link(&pool, link.id, now())
            .await
            .expect("none left")
    );
    let admin = InviteLink {
        id: Uuid::now_v7(),
        role: WorkspaceRole::Admin,
        ..link.clone()
    };
    assert!(
        workspace_invites::insert_link(&pool, &admin, b"other", None)
            .await
            .is_err(),
        "a link admits members or guests"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn deleting_a_workspace_deletes_all_that_is_in_it(pool: PgPool) {
    let ada = user(&pool, "ada@example.com").await;
    let bob = user(&pool, "bob@example.com").await;
    let design = workspace("design");
    let ops = workspace("ops");
    workspaces::insert(&pool, &design).await.expect("saved");
    workspaces::insert(&pool, &ops).await.expect("saved");
    for space in [&design, &ops] {
        let team = teamspace(space.id, TeamspaceAccess::Default);
        teamspaces::insert(&pool, &team).await.expect("saved");
        workspaces::add_member(&pool, space.id, ada, WorkspaceRole::Owner, now())
            .await
            .expect("added");
        teamspaces::add_member(&pool, &team, ada, TeamspaceRole::Owner, now())
            .await
            .expect("added");
        workspace_invites::insert(&pool, &invitation(space.id, bob))
            .await
            .expect("saved");
    }

    assert!(workspaces::delete(&pool, design.id).await.expect("deleted"));

    for (query, left) in [
        ("SELECT count(*) FROM workspaces", 1),
        ("SELECT count(*) FROM workspace_members", 1),
        ("SELECT count(*) FROM teamspaces", 1),
        ("SELECT count(*) FROM teamspace_members", 1),
        ("SELECT count(*) FROM workspace_invitations", 1),
        ("SELECT count(*) FROM users", 2),
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .fetch_one(&pool)
            .await
            .expect("counted");
        assert_eq!(count, left, "{query}");
    }
}
