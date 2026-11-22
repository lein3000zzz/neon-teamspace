//! Invitations into workspaces, invite links, and the member limit they meet.

mod support;

use std::time::Duration;

use neon_app::ratelimit::Bucket;
use neon_app::workspaces::{LimitsUpdate, LinkSpec, WorkspaceError};
use neon_domain::auth::{Principal, Role};
use neon_domain::settings::Secret;
use neon_domain::workspaces::{DEFAULT_LINK_USES, INVITATION_TTL, WorkspaceRole};
use serde_json::json;
use sqlx::PgPool;
use support::{Harness, Options};
use uuid::Uuid;

fn link(role: WorkspaceRole) -> LinkSpec {
    LinkSpec {
        role,
        max_uses: None,
        expires_at: None,
    }
}

async fn in_default_teamspace(h: &Harness, who: &Principal, workspace_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM teamspace_members m JOIN teamspaces t ON t.id = m.teamspace_id
                        WHERE t.workspace_id = $1 AND t.access = 'default' AND m.user_id = $2)",
    )
    .bind(workspace_id)
    .bind(who.user.id)
    .fetch_one(&h.pool)
    .await
    .expect("checked")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_invitation_waits_for_its_answer(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;

    let sent = h
        .workspaces
        .invite(&ada, id, "  Bob@Example.com ", WorkspaceRole::Member)
        .await
        .expect("invited");
    assert_eq!(sent.invitee.id, bob.user.id);
    assert_eq!(sent.invitation.role, WorkspaceRole::Member);
    assert_eq!(
        sent.invitation.expires_at - sent.invitation.created_at,
        INVITATION_TTL
    );
    let not_yet = h.workspaces.workspace(&bob, id).await;
    assert!(
        matches!(not_yet, Err(WorkspaceError::NotFound(_))),
        "nothing is joined before accepting"
    );

    let pending = h
        .workspaces
        .sent_invitations(&ada, id)
        .await
        .expect("listed");
    assert_eq!(pending.len(), 1);
    let mine = h.workspaces.my_invitations(&bob).await.expect("listed");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].workspace.id, id);
    assert_eq!(
        mine[0].invited_by.as_ref().map(|person| person.id),
        Some(ada.user.id)
    );
    let not_theirs = h
        .workspaces
        .accept_invitation(&ada, sent.invitation.id)
        .await;
    assert!(matches!(not_theirs, Err(WorkspaceError::NotFound(_))));

    let joined = h
        .workspaces
        .accept_invitation(&bob, sent.invitation.id)
        .await
        .expect("accepted");
    assert_eq!(joined.workspace.id, id);
    assert_eq!(joined.role, WorkspaceRole::Member);
    assert!(in_default_teamspace(&h, &bob, id).await);
    assert!(
        h.workspaces
            .my_invitations(&bob)
            .await
            .expect("listed")
            .is_empty()
    );
    assert!(
        h.workspaces
            .sent_invitations(&ada, id)
            .await
            .expect("listed")
            .is_empty()
    );
    let twice = h
        .workspaces
        .accept_invitation(&bob, sent.invitation.id)
        .await;
    assert!(matches!(twice, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_guest_joins_no_teamspace(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;

    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;

    assert!(!in_default_teamspace(&h, &gus, id).await);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn declined_and_revoked_invitations_cannot_be_accepted(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;

    let first = h
        .workspaces
        .invite(&ada, id, "bob@example.com", WorkspaceRole::Admin)
        .await
        .expect("invited");
    h.workspaces
        .decline_invitation(&bob, first.invitation.id)
        .await
        .expect("declined");
    let declined = h
        .workspaces
        .accept_invitation(&bob, first.invitation.id)
        .await;
    assert!(matches!(declined, Err(WorkspaceError::NotFound(_))));

    let second = h
        .workspaces
        .invite(&ada, id, "bob@example.com", WorkspaceRole::Admin)
        .await
        .expect("invited again once declined");
    h.workspaces
        .revoke_invitation(&ada, id, second.invitation.id)
        .await
        .expect("revoked");
    let revoked = h
        .workspaces
        .accept_invitation(&bob, second.invitation.id)
        .await;
    assert!(matches!(revoked, Err(WorkspaceError::NotFound(_))));
    let again = h
        .workspaces
        .revoke_invitation(&ada, id, second.invitation.id)
        .await;
    assert!(matches!(again, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn invitations_expire_and_can_be_sent_again(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let sent = h
        .workspaces
        .invite(&ada, id, "bob@example.com", WorkspaceRole::Member)
        .await
        .expect("invited");

    let twice = h
        .workspaces
        .invite(&ada, id, "bob@example.com", WorkspaceRole::Guest)
        .await;
    assert!(
        matches!(twice, Err(WorkspaceError::AlreadyInvited)),
        "{twice:?}"
    );

    h.time.advance(INVITATION_TTL);
    let expired = h
        .workspaces
        .accept_invitation(&bob, sent.invitation.id)
        .await;
    assert!(matches!(expired, Err(WorkspaceError::NotFound(_))));
    assert!(
        h.workspaces
            .my_invitations(&bob)
            .await
            .expect("listed")
            .is_empty()
    );
    assert!(
        h.workspaces
            .sent_invitations(&ada, id)
            .await
            .expect("listed")
            .is_empty()
    );

    let fresh = h
        .workspaces
        .invite(&ada, id, "bob@example.com", WorkspaceRole::Guest)
        .await
        .expect("an expired invitation makes way for a new one");
    h.workspaces
        .accept_invitation(&bob, fresh.invitation.id)
        .await
        .expect("accepted");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn invitations_only_go_to_accounts_that_can_sign_in(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let disabled = h.account("off@example.com", Role::User).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.id)
        .execute(&h.pool)
        .await
        .expect("disabled");
    let unverified = h.account("new@example.com", Role::User).await;
    sqlx::query("UPDATE users SET email_verified_at = NULL WHERE id = $1")
        .bind(unverified.id)
        .execute(&h.pool)
        .await
        .expect("unverified");

    for email in ["nobody@example.com", "off@example.com", "new@example.com"] {
        let refused = h
            .workspaces
            .invite(&ada, id, email, WorkspaceRole::Member)
            .await;
        assert!(
            matches!(refused, Err(WorkspaceError::NoAccount)),
            "{email}: {refused:?}"
        );
    }
    let bad = h
        .workspaces
        .invite(&ada, id, "not an address", WorkspaceRole::Member)
        .await;
    assert!(matches!(bad, Err(WorkspaceError::InvalidInput(_))));
    let member = h
        .workspaces
        .invite(&ada, id, "ada@example.com", WorkspaceRole::Member)
        .await;
    assert!(matches!(member, Err(WorkspaceError::AlreadyMember)));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn address_lookups_are_rate_limited_per_inviter(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;

    let forbidden = h
        .workspaces
        .invite(&cy, id, "nobody@example.com", WorkspaceRole::Member)
        .await;
    assert!(
        matches!(forbidden, Err(WorkspaceError::Forbidden(_))),
        "a member learns nothing of addresses: {forbidden:?}"
    );
    let lookups: Vec<String> = h
        .limiter
        .seen()
        .into_iter()
        .filter(|(bucket, _)| *bucket == Bucket::WorkspaceInvite)
        .map(|(_, key)| key)
        .collect();
    assert_eq!(
        lookups,
        [ada.user.id.to_string()],
        "only allowed invites count"
    );

    h.limiter.refuse(Bucket::WorkspaceInvite);
    let limited = h
        .workspaces
        .invite(&ada, id, "nobody@example.com", WorkspaceRole::Member)
        .await;
    assert!(
        matches!(limited, Err(WorkspaceError::RateLimited { retry_after }) if retry_after == Duration::from_secs(30)),
        "{limited:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn who_may_invite_and_as_what(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    h.person("dee@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;

    let owner = h
        .workspaces
        .invite(&ada, id, "dee@example.com", WorkspaceRole::Owner)
        .await;
    assert!(matches!(owner, Err(WorkspaceError::Forbidden(_))));
    let by_member = h
        .workspaces
        .invite(&cy, id, "dee@example.com", WorkspaceRole::Member)
        .await;
    assert!(matches!(by_member, Err(WorkspaceError::Forbidden(_))));
    let listed = h.workspaces.sent_invitations(&cy, id).await;
    assert!(matches!(listed, Err(WorkspaceError::Forbidden(_))));
    h.workspaces
        .invite(&bob, id, "dee@example.com", WorkspaceRole::Admin)
        .await
        .expect("an admin invites an admin");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_member_limit_refuses_invitations_accepts_and_links(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let pending = h
        .workspaces
        .invite(&ada, id, "cy@example.com", WorkspaceRole::Member)
        .await
        .expect("invited before the limit");
    let created = h
        .workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Member))
        .await
        .expect("a link before the limit");
    h.bring_in(&ada, id, &bob, WorkspaceRole::Member).await;
    h.set("workspaces.limits.members", json!(2)).await;

    let invite = h
        .workspaces
        .invite(&ada, id, "admin@example.com", WorkspaceRole::Member)
        .await;
    assert!(
        matches!(invite, Err(WorkspaceError::MemberLimit { limit: 2 })),
        "{invite:?}"
    );
    assert_eq!(
        invite.expect_err("refused").to_string(),
        "this workspace has reached its limit of 2 members"
    );
    let accept = h
        .workspaces
        .accept_invitation(&cy, pending.invitation.id)
        .await;
    assert!(matches!(accept, Err(WorkspaceError::MemberLimit { .. })));
    let join = h.workspaces.join_with_link(&cy, &created.token).await;
    assert!(matches!(join, Err(WorkspaceError::MemberLimit { .. })));
    let new_link = h
        .workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Guest))
        .await;
    assert!(matches!(new_link, Err(WorkspaceError::MemberLimit { .. })));
    assert_eq!(
        h.workspaces
            .my_invitations(&cy)
            .await
            .expect("listed")
            .len(),
        1,
        "the invitation waits for room"
    );

    h.set("workspaces.limits.members", json!(1)).await;
    assert_eq!(
        h.workspaces
            .list_members(&ada, id)
            .await
            .expect("listed")
            .len(),
        2,
        "nobody is removed by a lower limit"
    );
    let details = h.workspaces.workspace(&ada, id).await.expect("seen");
    assert_eq!(details.limits.members, Some(1));

    h.workspaces
        .set_workspace_limits(
            &admin,
            id,
            LimitsUpdate {
                members: Some(Some(0)),
                ..LimitsUpdate::default()
            },
        )
        .await
        .expect("lifted for this workspace");
    h.workspaces
        .accept_invitation(&cy, pending.invitation.id)
        .await
        .expect("room now");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn joining_by_link_takes_a_use_and_its_role(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let member_link = h
        .workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Member))
        .await
        .expect("created");
    assert!(member_link.token.expose().starts_with("wsl_"));
    assert_eq!(member_link.link.max_uses, DEFAULT_LINK_USES);
    assert!(member_link.link.token_kept);
    let guest_link = h
        .workspaces
        .create_invite_link(
            &ada,
            id,
            LinkSpec {
                role: WorkspaceRole::Guest,
                max_uses: Some(1),
                expires_at: None,
            },
        )
        .await
        .expect("created");

    let joined = h
        .workspaces
        .join_with_link(
            &bob,
            &Secret::new(format!(" {} ", member_link.token.expose())),
        )
        .await
        .expect("joined");
    assert_eq!(joined.role, WorkspaceRole::Member);
    assert!(in_default_teamspace(&h, &bob, id).await);
    let twice = h.workspaces.join_with_link(&bob, &member_link.token).await;
    assert!(matches!(twice, Err(WorkspaceError::AlreadyMember)));

    let guest = h
        .workspaces
        .join_with_link(&gus, &guest_link.token)
        .await
        .expect("joined");
    assert_eq!(guest.role, WorkspaceRole::Guest);
    assert!(!in_default_teamspace(&h, &gus, id).await);

    let links = h.workspaces.invite_links(&ada, id).await.expect("listed");
    let uses: Vec<(WorkspaceRole, i32)> = links.iter().map(|link| (link.role, link.uses)).collect();
    assert_eq!(
        uses,
        [(WorkspaceRole::Guest, 1), (WorkspaceRole::Member, 1)],
        "newest first; joining twice took no second use"
    );
    for bad in ["", "wsl_nothing", &"x".repeat(200)] {
        let refused = h.workspaces.join_with_link(&gus, &Secret::new(bad)).await;
        assert!(
            matches!(refused, Err(WorkspaceError::InvalidInviteLink)),
            "{bad}: {refused:?}"
        );
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_link_used_up_expired_or_revoked_admits_nobody(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let once = h
        .workspaces
        .create_invite_link(
            &ada,
            id,
            LinkSpec {
                role: WorkspaceRole::Member,
                max_uses: Some(1),
                expires_at: None,
            },
        )
        .await
        .expect("created");
    let revoked = h
        .workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Member))
        .await
        .expect("created");
    let short = h
        .workspaces
        .create_invite_link(
            &ada,
            id,
            LinkSpec {
                role: WorkspaceRole::Member,
                max_uses: None,
                expires_at: Some(h.time.clock().now() + Duration::from_secs(3600)),
            },
        )
        .await
        .expect("created");
    h.workspaces
        .revoke_invite_link(&ada, id, revoked.link.id)
        .await
        .expect("revoked");

    h.workspaces
        .join_with_link(&bob, &once.token)
        .await
        .expect("the one use");
    let used_up = h.workspaces.join_with_link(&cy, &once.token).await;
    assert!(matches!(used_up, Err(WorkspaceError::InvalidInviteLink)));
    let gone = h.workspaces.join_with_link(&cy, &revoked.token).await;
    assert!(matches!(gone, Err(WorkspaceError::InvalidInviteLink)));
    h.time.advance(Duration::from_secs(3600));
    let expired = h.workspaces.join_with_link(&cy, &short.token).await;
    assert!(matches!(expired, Err(WorkspaceError::InvalidInviteLink)));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_links_last_use_goes_to_one_of_two_joining_at_once(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let once = h
        .workspaces
        .create_invite_link(
            &ada,
            id,
            LinkSpec {
                role: WorkspaceRole::Member,
                max_uses: Some(1),
                expires_at: None,
            },
        )
        .await
        .expect("created");

    let (first, second) = tokio::join!(
        h.workspaces.join_with_link(&bob, &once.token),
        h.workspaces.join_with_link(&cy, &once.token),
    );

    assert_eq!(
        [&first, &second]
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        1,
        "{first:?} {second:?}"
    );
    assert_eq!(
        h.workspaces
            .list_members(&ada, id)
            .await
            .expect("listed")
            .len(),
        2
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn link_rules_follow_the_invite_rules(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    let now = h.time.clock().now();

    for role in [WorkspaceRole::Owner, WorkspaceRole::Admin] {
        let refused = h.workspaces.create_invite_link(&ada, id, link(role)).await;
        assert!(
            matches!(refused, Err(WorkspaceError::Forbidden(_))),
            "{role}"
        );
    }
    let by_member = h
        .workspaces
        .create_invite_link(&cy, id, link(WorkspaceRole::Member))
        .await;
    assert!(matches!(by_member, Err(WorkspaceError::Forbidden(_))));
    for spec in [
        LinkSpec {
            max_uses: Some(0),
            ..link(WorkspaceRole::Member)
        },
        LinkSpec {
            max_uses: Some(1001),
            ..link(WorkspaceRole::Member)
        },
        LinkSpec {
            expires_at: Some(now),
            ..link(WorkspaceRole::Member)
        },
        LinkSpec {
            expires_at: Some(now + Duration::from_secs(366 * 24 * 3600)),
            ..link(WorkspaceRole::Member)
        },
    ] {
        let refused = h
            .workspaces
            .create_invite_link(&ada, id, spec.clone())
            .await;
        assert!(
            matches!(refused, Err(WorkspaceError::InvalidInput(_))),
            "{spec:?}: {refused:?}"
        );
    }
    let listed = h.workspaces.invite_links(&cy, id).await;
    assert!(matches!(listed, Err(WorkspaceError::Forbidden(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_links_token_is_copied_again_from_its_sealed_copy(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let created = h
        .workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Member))
        .await
        .expect("created");

    let copied = h
        .workspaces
        .invite_link_token(&ada, id, created.link.id)
        .await
        .expect("copied");
    assert_eq!(copied.expose(), created.token.expose());
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM workspace_invite_links")
        .fetch_one(&h.pool)
        .await
        .expect("read");
    assert_ne!(
        stored,
        created.token.expose().as_bytes(),
        "only its hash is kept"
    );
    let other = h
        .workspaces
        .invite_link_token(&ada, Uuid::now_v7(), created.link.id)
        .await;
    assert!(matches!(other, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_a_key_a_links_token_shows_once(pool: PgPool) {
    let h = Harness::with(
        pool,
        Options {
            envelope: neon_app::crypto::Envelope::default(),
            ..Options::default()
        },
    )
    .await;
    let ada = h.person("ada@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let created = h
        .workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Member))
        .await
        .expect("created");

    assert!(!created.link.token_kept);
    let copied = h
        .workspaces
        .invite_link_token(&ada, id, created.link.id)
        .await;
    assert!(
        matches!(copied, Err(WorkspaceError::TokenUnavailable(_))),
        "{copied:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_cleanup_removes_what_ended_a_month_ago(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let accepted = h
        .workspaces
        .invite(&ada, id, "bob@example.com", WorkspaceRole::Member)
        .await
        .expect("invited");
    h.workspaces
        .accept_invitation(&bob, accepted.invitation.id)
        .await
        .expect("accepted");
    h.workspaces
        .invite(&ada, id, "cy@example.com", WorkspaceRole::Member)
        .await
        .expect("left pending");
    h.workspaces
        .create_invite_link(&ada, id, link(WorkspaceRole::Member))
        .await
        .expect("a link");

    let report = h.workspaces.cleanup().await.expect("cleaned");
    assert_eq!(report, neon_app::workspaces::CleanupReport::default());

    h.time
        .advance(INVITATION_TTL + Duration::from_secs(31 * 24 * 3600));
    let report = h.workspaces.cleanup().await.expect("cleaned");
    assert_eq!(report.invitations, 2);
    assert_eq!(report.invite_links, 1);
    assert!(
        h.workspaces
            .my_invitations(&cy)
            .await
            .expect("listed")
            .is_empty()
    );
    assert_eq!(
        h.workspaces
            .list_members(&ada, id)
            .await
            .expect("listed")
            .len(),
        2,
        "members stay"
    );
}
