//! Agents and integrations: who makes them and within which limits, how their tokens act
//! for their owners, suspension, handing them over and deleting them, how they are named,
//! and tokens bound to one workspace.

mod support;

use std::fmt::Debug;

use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::auth::{AuthError, UserUpdate};
use neon_app::databases::{NewDatabase, NewProperty, NewRow, PropertySpec};
use neon_app::pages::{AccessChange, Location, NewPage, PageError, PageView};
use neon_app::workspaces::{
    AgentUpdate, LimitsUpdate, NewAgent, NewTeamspace, WorkspaceError, WorkspaceUpdate,
};
use neon_domain::agents::{Agent, AgentCreation, AgentStatus};
use neon_domain::auth::{Credential, Principal, Scope, UserKind};
use neon_domain::databases::PropertyKind;
use neon_domain::pages::{PageAccess, Placement};
use neon_domain::workspaces::{TeamspaceAccess, TeamspaceRole, WorkspaceRole};
use serde_json::{Map, json};
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;

/// Ada owns Design, with Bob as an admin, Cy as a member and Gus as a guest. Besides
/// General, Cy owns Open, Closed and Secret, which is private.
struct Team {
    h: Harness,
    id: Uuid,
    general: Uuid,
    open: Uuid,
    closed: Uuid,
    secret: Uuid,
    ada: Principal,
    bob: Principal,
    cy: Principal,
    gus: Principal,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let gus = h.person("gus@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &gus, WorkspaceRole::Guest).await;
    let general = h
        .workspaces
        .list_teamspaces(&ada, id)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let mut made = Vec::new();
    for (name, access) in [
        ("Open", TeamspaceAccess::Open),
        ("Closed", TeamspaceAccess::Closed),
        ("Secret", TeamspaceAccess::Private),
    ] {
        let entry = h
            .workspaces
            .create_teamspace(
                &cy,
                id,
                NewTeamspace {
                    name: name.to_owned(),
                    icon: None,
                    hue: None,
                    access,
                },
            )
            .await
            .expect("created");
        made.push(entry.teamspace.id);
    }
    h.changes.take();
    Team {
        h,
        id,
        general,
        open: made[0],
        closed: made[1],
        secret: made[2],
        ada,
        bob,
        cy,
        gus,
    }
}

impl Team {
    async fn page(
        &self,
        by: &Principal,
        teamspace: Uuid,
        title: &str,
    ) -> Result<PageView, PageError> {
        self.h
            .pages
            .create_page(
                by,
                self.id,
                Location::Teamspace(teamspace),
                Placement::Last,
                NewPage {
                    title: title.to_owned(),
                    ..NewPage::default()
                },
            )
            .await
    }

    async fn update(&self, by: &Principal, update: WorkspaceUpdate) {
        self.h
            .workspaces
            .update_workspace(by, self.id, update)
            .await
            .expect("updated");
    }

    async fn status(&self, agent: &Agent) -> AgentStatus {
        self.h
            .agents
            .agent(&self.ada, self.id, agent.id)
            .await
            .expect("seen")
            .status
    }

    async fn add(&self, by: &Principal, teamspace: Uuid, who: Uuid) -> Result<(), WorkspaceError> {
        self.h
            .workspaces
            .add_teamspace_member(by, self.id, teamspace, who, TeamspaceRole::Member)
            .await
            .map(|_| ())
    }

    /// Ada gives `who` the workspace role `role`.
    async fn role(&self, who: &Principal, role: WorkspaceRole) {
        self.h
            .workspaces
            .change_member_role(&self.ada, self.id, who.user.id, role)
            .await
            .expect("changed");
    }
}

fn new(name: &str) -> NewAgent {
    NewAgent {
        name: name.to_owned(),
        hue: None,
    }
}

fn assert_forbidden<T: Debug>(result: Result<T, WorkspaceError>) -> &'static str {
    match result {
        Err(WorkspaceError::Forbidden(reason)) => reason,
        other => panic!("expected forbidden, got {other:?}"),
    }
}

fn assert_hidden<T: Debug, E: Debug>(result: Result<T, E>) {
    let text = format!("{result:?}");
    assert!(text.contains("NotFound"), "expected not found, got {text}");
}

fn assert_page_forbidden<T: Debug>(result: Result<T, PageError>) -> &'static str {
    match result {
        Err(PageError::Forbidden(reason)) => reason,
        other => panic!("expected forbidden, got {other:?}"),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn agents_are_made_by_whom_the_workspace_lets_and_within_its_limit(pool: PgPool) {
    let t = team(pool).await;
    let agents = &t.h.agents;

    let scout = t.h.agent(&t.cy, t.id, "  Scout ").await;
    assert_eq!(scout.name, "Scout");
    assert_eq!(scout.kind, UserKind::Agent);
    assert_eq!(
        scout.owner.as_ref().map(|owner| owner.id),
        Some(t.cy.user.id)
    );
    assert_eq!(scout.status, AgentStatus::Active);
    assert!(
        agents.create_agent(&t.cy, t.id, new("")).await.is_err(),
        "a name is needed"
    );
    assert_eq!(
        assert_forbidden(agents.create_agent(&t.gus, t.id, new("Peek")).await),
        "guests make no agents"
    );

    // Only owners and admins, once the workspace says so.
    t.update(
        &t.ada,
        WorkspaceUpdate {
            agent_creation: Some(AgentCreation::Admins),
            ..WorkspaceUpdate::default()
        },
    )
    .await;
    assert_eq!(
        assert_forbidden(agents.create_agent(&t.cy, t.id, new("Two")).await),
        "in this workspace only owners and admins make agents"
    );
    t.h.agent(&t.bob, t.id, "Bobby").await;

    // Turned off in the workspace, then on the instance.
    t.update(
        &t.ada,
        WorkspaceUpdate {
            agents_enabled: Some(false),
            ..WorkspaceUpdate::default()
        },
    )
    .await;
    assert_eq!(
        assert_forbidden(agents.create_agent(&t.ada, t.id, new("Off")).await),
        "agents are turned off in this workspace"
    );
    t.update(
        &t.ada,
        WorkspaceUpdate {
            agents_enabled: Some(true),
            ..WorkspaceUpdate::default()
        },
    )
    .await;
    t.h.set("mcp.identities", json!(["user"])).await;
    assert_eq!(
        assert_forbidden(agents.create_agent(&t.ada, t.id, new("Off")).await),
        "agents are turned off on this instance"
    );
    t.h.set("mcp.identities", json!(["user", "agent"])).await;

    // Agents count against their own limit, people against theirs.
    let admin = t.h.admin().await;
    let limits = |members, agents| LimitsUpdate {
        members: Some(Some(members)),
        agents: Some(Some(agents)),
        storage_bytes: None,
    };
    let set =
        t.h.workspaces
            .set_workspace_limits(&admin, t.id, limits(4, 3))
            .await
            .expect("set");
    assert_eq!(set.usage.members, 4, "people alone");
    assert_eq!(set.usage.agents, 2);
    t.h.agent(&t.ada, t.id, "Third").await;
    assert!(matches!(
        agents.create_agent(&t.ada, t.id, new("Fourth")).await,
        Err(WorkspaceError::AgentLimit { limit: 3 })
    ));
    let details = t.h.workspaces.workspace(&t.ada, t.id).await.expect("read");
    assert_eq!((details.members, details.agents), (Some(4), Some(3)));

    // A deleted one frees its place; 0 is no limit.
    agents
        .delete_agent(&t.ada, t.id, scout.id)
        .await
        .expect("deleted");
    t.h.agent(&t.ada, t.id, "Fourth").await;
    t.h.workspaces
        .set_workspace_limits(&admin, t.id, limits(4, 0))
        .await
        .expect("set");
    t.h.agent(&t.ada, t.id, "Fifth").await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_is_a_member_of_its_workspace_alone(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;

    let members =
        t.h.workspaces
            .list_members(&t.ada, t.id)
            .await
            .expect("listed");
    let listed = members
        .iter()
        .find(|member| member.person.id == scout.id)
        .expect("listed");
    assert_eq!(listed.role, WorkspaceRole::Member);
    assert_eq!(listed.person.kind, UserKind::Agent);
    assert_eq!(listed.person.email, None);
    assert_eq!(
        listed
            .person
            .owner
            .as_ref()
            .map(|owner| owner.display_name.as_str()),
        Some("cy")
    );
    assert!(!listed.suspended);
    let general =
        t.h.workspaces
            .teamspace_members(&t.ada, t.id, t.general)
            .await
            .expect("listed");
    assert!(general.iter().any(|member| member.person.id == scout.id));

    // Its role stays member, and it is deleted rather than removed.
    for role in [
        WorkspaceRole::Admin,
        WorkspaceRole::Owner,
        WorkspaceRole::Guest,
    ] {
        assert_eq!(
            assert_forbidden(
                t.h.workspaces
                    .change_member_role(&t.ada, t.id, scout.id, role)
                    .await
            ),
            "an agent's or integration's role in its workspace is always member"
        );
    }
    assert!(
        assert_forbidden(t.h.workspaces.remove_member(&t.ada, t.id, scout.id).await)
            .contains("is deleted, not removed")
    );
    assert_eq!(
        assert_forbidden(
            t.h.workspaces
                .transfer_ownership(&t.ada, t.id, scout.id)
                .await
        ),
        "ownership goes to a person"
    );
    // Nor is it a teamspace's owner.
    t.add(&t.cy, t.open, t.cy.user.id).await.ok();
    assert!(matches!(
        t.h.workspaces
            .add_teamspace_member(&t.cy, t.id, t.open, scout.id, TeamspaceRole::Owner)
            .await,
        Err(WorkspaceError::InvalidInput(_))
    ));

    // Its token sees its workspace alone, and joins no other.
    let (_, worker) = t.h.agent_token(&t.cy, &scout, &["read", "write"]).await;
    let other = t.h.workspace(&t.ada, "Elsewhere").await.workspace.id;
    let listed =
        t.h.workspaces
            .list_workspaces(&worker)
            .await
            .expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0.id, t.id);
    assert_eq!(listed[0].1, WorkspaceRole::Member);
    assert_hidden(t.h.workspaces.workspace(&worker, other).await);
    let sent =
        t.h.workspaces
            .invite(&t.ada, other, "cy@example.com", WorkspaceRole::Member)
            .await
            .expect("invited");
    assert_eq!(
        assert_forbidden(
            t.h.workspaces
                .accept_invitation(&worker, sent.invitation.id)
                .await
        ),
        "an agent or integration belongs to its own workspace alone"
    );
    assert_eq!(
        assert_forbidden(
            t.h.workspaces
                .create_workspace(&worker, Default::default())
                .await
        ),
        "an agent or integration works inside its own workspace only"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_does_only_what_both_it_and_its_owner_may(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.ada, t.id, "Scout").await;
    let (_, worker) = t.h.agent_token(&t.ada, &scout, &["write"]).await;
    let Credential::Token { workspace_id, .. } = &worker.credential else {
        panic!("a token");
    };
    assert_eq!(*workspace_id, Some(t.id));
    assert_eq!(worker.user.id, scout.id);

    // General: both are in it. What it makes names it.
    let notes = t.page(&worker, t.general, "Notes").await.expect("made");
    assert_eq!(notes.page.created_by, Some(scout.id));
    assert_eq!(notes.access, PageAccess::Edit);

    // Open: it joins on its own, but Ada is not in it, so it only views.
    let open_page = t.page(&t.cy, t.open, "Plans").await.expect("made");
    t.h.workspaces
        .join_teamspace(&worker, t.id, t.open)
        .await
        .expect("joined");
    assert!(
        assert_page_forbidden(t.page(&worker, t.open, "Mine").await).contains("its owner may not")
    );
    let seen =
        t.h.pages
            .page(&worker, open_page.page.id)
            .await
            .expect("seen");
    assert_eq!(seen.access, PageAccess::View);
    t.h.workspaces
        .join_teamspace(&t.ada, t.id, t.open)
        .await
        .expect("joined");
    assert_eq!(
        t.h.pages
            .page(&worker, open_page.page.id)
            .await
            .expect("seen")
            .access,
        PageAccess::Edit
    );

    // Closed: Ada adds her agent only where she is herself; an admin adds anyone's.
    assert_eq!(
        assert_forbidden(t.add(&t.ada, t.closed, scout.id).await),
        "you add your agent only to teamspaces you are in yourself"
    );
    t.add(&t.bob, t.closed, scout.id)
        .await
        .expect("added by an admin");
    let closed_page = t.page(&t.cy, t.closed, "Budget").await.expect("made");
    assert_eq!(
        t.h.pages
            .page(&worker, closed_page.page.id)
            .await
            .expect("seen")
            .access,
        PageAccess::View,
        "Ada views Closed from outside it"
    );

    // Secret: hidden from both, until both are in it.
    let secret_page = t.page(&t.cy, t.secret, "Payroll").await.expect("made");
    assert_hidden(t.h.pages.page(&worker, secret_page.page.id).await);
    t.add(&t.cy, t.secret, t.ada.user.id).await.expect("added");
    assert_hidden(t.h.pages.page(&worker, secret_page.page.id).await);
    t.add(&t.ada, t.secret, scout.id)
        .await
        .expect("added by its owner");
    assert_eq!(
        t.h.pages
            .page(&worker, secret_page.page.id)
            .await
            .expect("seen")
            .access,
        PageAccess::Edit
    );
    t.h.changes.take();
    t.h.workspaces
        .leave_teamspace(&t.ada, t.id, t.secret)
        .await
        .expect("left");
    assert_hidden(t.h.pages.page(&worker, secret_page.page.id).await);
    assert!(t.h.changes.take().contains(&AccessChange::Teamspace {
        workspace_id: t.id,
        teamspace_id: t.secret,
        user_id: Some(t.ada.user.id),
    }));

    // No private pages: its owner could not see them.
    assert_eq!(
        assert_page_forbidden(
            t.h.pages
                .create_page(
                    &worker,
                    t.id,
                    Location::Private,
                    Placement::Last,
                    NewPage::default(),
                )
                .await,
        ),
        "an agent or integration has no private pages: its owner could not see them"
    );
    assert!(
        t.h.pages
            .private_tree(&worker, t.id, None, None)
            .await
            .is_err()
    );
    // Still in Secret, it reaches a page there as far as a share to Ada does.
    let shared = t.page(&t.cy, t.secret, "Shared").await.expect("made");
    assert_hidden(t.h.pages.page(&worker, shared.page.id).await);
    t.h.pages
        .share_page(&t.cy, shared.page.id, t.ada.user.id, PageAccess::View)
        .await
        .expect("shared with Ada");
    assert_eq!(
        t.h.pages
            .page(&worker, shared.page.id)
            .await
            .expect("seen")
            .access,
        PageAccess::View
    );
    t.h.pages
        .change_share(&t.cy, shared.page.id, t.ada.user.id, PageAccess::Edit)
        .await
        .expect("changed");
    assert_eq!(
        t.h.pages
            .page(&worker, shared.page.id)
            .await
            .expect("seen")
            .access,
        PageAccess::Edit
    );
    // A share to the agent itself gives it nothing its owner lacks.
    let alone = t.page(&t.cy, t.closed, "Alone").await.expect("made");
    t.h.pages
        .share_page(&t.cy, alone.page.id, scout.id, PageAccess::Edit)
        .await
        .expect("shared with the agent");
    assert_eq!(
        t.h.pages
            .page(&worker, alone.page.id)
            .await
            .expect("seen")
            .access,
        PageAccess::View,
        "Ada views Closed from outside"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn agents_work_in_the_workspace_and_manage_nothing(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.ada, t.id, "Scout").await;
    let (_, worker) = t.h.agent_token(&t.ada, &scout, &["read", "write"]).await;
    let ws = &t.h.workspaces;
    let agents = &t.h.agents;

    assert_forbidden(
        ws.update_workspace(&worker, t.id, WorkspaceUpdate::default())
            .await,
    );
    assert_forbidden(
        ws.invite(&worker, t.id, "eve@example.com", WorkspaceRole::Member)
            .await,
    );
    assert_forbidden(
        ws.change_member_role(&worker, t.id, t.cy.user.id, WorkspaceRole::Guest)
            .await,
    );
    assert_forbidden(ws.remove_member(&worker, t.id, t.cy.user.id).await);
    assert_forbidden(ws.leave_workspace(&worker, t.id).await);
    assert_forbidden(
        ws.create_teamspace(
            &worker,
            t.id,
            NewTeamspace {
                name: "Bots".to_owned(),
                icon: None,
                hue: None,
                access: TeamspaceAccess::Open,
            },
        )
        .await,
    );
    assert_forbidden(t.add(&worker, t.general, t.cy.user.id).await);
    assert_forbidden(agents.create_agent(&worker, t.id, new("Child")).await);
    assert_forbidden(agents.delete_agent(&worker, t.id, scout.id).await);
    assert_forbidden(agents.agent_tokens(&worker, t.id, scout.id).await);
    // What it may see it sees, as its owner does.
    let members = ws.list_members(&worker, t.id).await.expect("listed");
    assert_eq!(members.len(), 5);
    let listed = agents.list_agents(&worker, t.id).await.expect("listed");
    assert_eq!(listed.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn agent_tokens_are_made_by_the_owner_read_and_write_only(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;
    let agents = &t.h.agents;
    let make = |by: &Principal, scopes: &'static [&'static str]| {
        let by = by.clone();
        async move {
            agents
                .create_agent_token(&by, t.id, scout.id, "worker", None, scopes)
                .await
        }
    };

    assert_eq!(
        assert_forbidden(make(&t.cy, &["read", "admin"]).await),
        "an agent's token reads and writes, and never has the admin scope"
    );
    assert_eq!(
        assert_forbidden(make(&t.bob, &["read"]).await),
        "only its owner makes an agent's tokens"
    );
    assert_hidden(make(&t.gus, &["read"]).await);
    let session_token =
        t.h.auth
            .create_api_token(&t.cy, "ci", None, &["write"], None)
            .await;
    let (secret, _) = session_token.expect("made");
    let by_token =
        t.h.auth
            .authenticate_token(secret.expose())
            .await
            .expect("works");
    assert!(assert_forbidden(make(&by_token, &["read"]).await).contains("browser session"));

    let (secret, token) = make(&t.cy, &["write", "read"]).await.expect("made");
    assert!(secret.expose().starts_with("nw_"));
    assert_eq!(token.scopes, [Scope::Read, Scope::Write]);
    assert_eq!(token.workspace_id, Some(t.id));
    assert_eq!(token.user_id, scout.id);

    // Listed without the secret, by its owner and the workspace's admins, not others.
    let listed = agents
        .agent_tokens(&t.bob, t.id, scout.id)
        .await
        .expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].prefix, &secret.expose()[..10]);
    agents
        .agent_tokens(&t.ada, t.id, scout.id)
        .await
        .expect("an owner of the workspace lists them");
    let other = t.h.person("dee@example.com").await;
    t.h.bring_in(&t.ada, t.id, &other, WorkspaceRole::Member)
        .await;
    assert_forbidden(agents.agent_tokens(&other, t.id, scout.id).await);
    assert!(
        t.h.auth
            .list_api_tokens(&t.cy)
            .await
            .expect("listed")
            .iter()
            .all(|own| own.user_id == t.cy.user.id)
    );

    // Revoked by an admin, it stops at once.
    t.h.changes.take();
    agents
        .revoke_agent_token(&t.bob, t.id, scout.id, token.id)
        .await
        .expect("revoked");
    assert!(matches!(
        t.h.auth.authenticate_token(secret.expose()).await,
        Err(AuthError::Unauthenticated)
    ));
    assert_eq!(
        t.h.changes.take(),
        [AccessChange::Account { user_id: scout.id }]
    );
    assert_hidden(
        agents
            .revoke_agent_token(&t.bob, t.id, scout.id, token.id)
            .await,
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_is_suspended_while_its_owner_is_out_or_disabled(pool: PgPool) {
    let t = team(pool).await;
    let dee = t.h.person("dee@example.com").await;
    t.h.bring_in(&t.ada, t.id, &dee, WorkspaceRole::Member)
        .await;
    let scout = t.h.agent(&dee, t.id, "Scout").await;
    let (secret, worker) = t.h.agent_token(&dee, &scout, &["write"]).await;
    let works = || async { t.h.auth.authenticate_token(secret.expose()).await.is_ok() };
    let refresh = || async { t.h.auth.refresh(&worker).await.is_ok() };

    // Dee leaves: suspended, the token refused, the agent still a member.
    t.h.workspaces
        .leave_workspace(&dee, t.id)
        .await
        .expect("left");
    assert!(!works().await && !refresh().await);
    assert_eq!(t.status(&scout).await, AgentStatus::Suspended);
    let members =
        t.h.workspaces
            .list_members(&t.ada, t.id)
            .await
            .expect("listed");
    assert!(
        members
            .iter()
            .any(|member| member.person.id == scout.id && member.suspended)
    );
    assert!(
        members
            .iter()
            .filter(|member| member.person.kind.is_human())
            .all(|member| !member.suspended)
    );
    assert!(matches!(
        t.h.agents
            .create_agent_token(&dee, t.id, scout.id, "again", None, &["read"])
            .await,
        Err(WorkspaceError::NotFound(_))
    ));

    // Back in the workspace, it works again: suspension is worked out, never stored.
    t.h.bring_in(&t.ada, t.id, &dee, WorkspaceRole::Member)
        .await;
    assert!(works().await && refresh().await);
    assert_eq!(t.status(&scout).await, AgentStatus::Active);

    // Disabled, then enabled.
    let admin = t.h.admin().await;
    let disable = |disabled| UserUpdate {
        disabled: Some(disabled),
        ..UserUpdate::default()
    };
    t.h.auth
        .update_user(&admin, dee.user.id, disable(true))
        .await
        .expect("disabled");
    assert!(!works().await);
    assert_eq!(t.status(&scout).await, AgentStatus::Suspended);
    t.h.auth
        .update_user(&admin, dee.user.id, disable(false))
        .await
        .expect("enabled");
    assert!(works().await);

    // Removed, then handed to Bob: it resumes, and Dee's tokens are revoked.
    t.h.workspaces
        .remove_member(&t.ada, t.id, dee.user.id)
        .await
        .expect("removed");
    assert_eq!(t.status(&scout).await, AgentStatus::Suspended);
    t.h.changes.take();
    let handed =
        t.h.agents
            .transfer_agent(&t.ada, t.id, scout.id, t.bob.user.id)
            .await
            .expect("handed over");
    assert_eq!(handed.status, AgentStatus::Active);
    assert_eq!(
        handed.owner.as_ref().map(|owner| owner.id),
        Some(t.bob.user.id)
    );
    assert_eq!(
        t.h.changes.take(),
        [AccessChange::Account { user_id: scout.id }]
    );
    t.h.bring_in(&t.ada, t.id, &dee, WorkspaceRole::Member)
        .await;
    assert!(!works().await, "Dee's token does not come back with her");
    let (_, bobs) = t.h.agent_token(&t.bob, &handed, &["read"]).await;
    assert_eq!(bobs.user.owner_id, Some(t.bob.user.id));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_is_suspended_while_its_owner_is_a_guest(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;
    let (secret, worker) = t.h.agent_token(&t.cy, &scout, &["write"]).await;
    let works = || async { t.h.auth.authenticate_token(secret.expose()).await.is_ok() };

    // A guest owns no agent: Cy's is suspended, its token refused, its connections looked
    // at again.
    t.h.changes.take();
    t.role(&t.cy, WorkspaceRole::Guest).await;
    assert_eq!(
        t.h.changes.take(),
        [AccessChange::Workspace {
            workspace_id: t.id,
            user_id: Some(t.cy.user.id)
        }]
    );
    assert!(!works().await && t.h.auth.refresh(&worker).await.is_err());
    assert_eq!(t.status(&scout).await, AgentStatus::Suspended);
    assert!(
        t.h.workspaces
            .list_members(&t.bob, t.id)
            .await
            .expect("listed")
            .iter()
            .any(|member| member.person.id == scout.id && member.suspended)
    );
    assert_hidden(
        t.h.agents
            .create_agent_token(&t.cy, t.id, scout.id, "more", None, &["read"])
            .await,
    );

    // A member again, Cy has it back.
    t.role(&t.cy, WorkspaceRole::Member).await;
    assert!(works().await);
    assert_eq!(t.status(&scout).await, AgentStatus::Active);

    // Or the workspace's owners and admins hand it to someone else, which resumes it.
    t.role(&t.cy, WorkspaceRole::Guest).await;
    let handed =
        t.h.agents
            .transfer_agent(&t.bob, t.id, scout.id, t.bob.user.id)
            .await
            .expect("handed over");
    assert_eq!(handed.status, AgentStatus::Active);
}

/// A request that authenticated as an agent just before its owner became a guest still
/// sees no one: what it reads is what a guest reads.
#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_of_a_guest_reads_no_people(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;
    let (_, worker) = t.h.agent_token(&t.cy, &scout, &["read"]).await;
    let database =
        t.h.databases
            .create_database(
                &t.ada,
                t.id,
                Location::Teamspace(t.general),
                Placement::Last,
                NewDatabase {
                    title: "Tasks".to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("made");
    let db = database.page.page.id;
    let assignee =
        t.h.databases
            .add_property(
                &t.ada,
                db,
                NewProperty {
                    name: "Assignee".to_owned(),
                    spec: PropertySpec {
                        kind: PropertyKind::Person,
                        number_format: None,
                        currency: None,
                        prefix: None,
                    },
                    options: None,
                    placement: Placement::Last,
                },
            )
            .await
            .expect("added");
    let mut values = Map::new();
    values.insert("Name".to_owned(), json!("Triage"));
    values.insert(assignee.id.to_string(), json!([t.ada.user.id]));
    let row =
        t.h.databases
            .create_row(
                &t.ada,
                db,
                NewRow {
                    values,
                    placement: Placement::Last,
                    group: None,
                },
            )
            .await
            .expect("a row");
    let read = t.h.databases.row(&worker, row.id).await.expect("read");
    assert_eq!(
        read.created_by,
        Some(t.ada.user.id),
        "a member's agent sees people"
    );

    t.role(&t.cy, WorkspaceRole::Guest).await;
    t.h.pages
        .share_page(&t.ada, db, t.cy.user.id, PageAccess::View)
        .await
        .expect("shared with Cy");
    let read = t.h.databases.row(&worker, row.id).await.expect("read");
    assert_eq!(read.created_by, None, "{read:?}");
    assert!(
        !read.values.contains_key(&assignee.id.to_string()),
        "{:?}",
        read.values
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn turning_agents_off_refuses_their_tokens(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;
    let (secret, _) = t.h.agent_token(&t.cy, &scout, &["read"]).await;
    let works = || async { t.h.auth.authenticate_token(secret.expose()).await.is_ok() };

    t.h.changes.take();
    t.update(
        &t.bob,
        WorkspaceUpdate {
            agents_enabled: Some(false),
            ..WorkspaceUpdate::default()
        },
    )
    .await;
    assert_eq!(
        t.h.changes.take(),
        [AccessChange::Workspace {
            workspace_id: t.id,
            user_id: None
        }]
    );
    assert!(!works().await);
    assert_eq!(t.status(&scout).await, AgentStatus::Disabled);
    assert!(
        assert_forbidden(
            t.h.agents
                .create_agent_token(&t.cy, t.id, scout.id, "more", None, &["read"])
                .await
        )
        .contains("turned off")
    );
    t.update(
        &t.bob,
        WorkspaceUpdate {
            agents_enabled: Some(true),
            ..WorkspaceUpdate::default()
        },
    )
    .await;
    assert!(works().await);

    t.h.set("mcp.identities", json!(["user"])).await;
    assert!(!works().await);
    assert_eq!(t.status(&scout).await, AgentStatus::Disabled);
    t.h.set("mcp.identities", json!(["agent", "user"])).await;
    assert!(works().await);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn only_the_owner_renames_and_the_managers_hand_over(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;
    let agents = &t.h.agents;
    let rename = |name: &str| AgentUpdate {
        name: Some(name.to_owned()),
        hue: Some(Some(42)),
    };

    let renamed = agents
        .update_agent(&t.cy, t.id, scout.id, rename("Ranger"))
        .await
        .expect("renamed");
    assert_eq!((renamed.name.as_str(), renamed.hue()), ("Ranger", 42));
    let unpinned = agents
        .update_agent(
            &t.cy,
            t.id,
            scout.id,
            AgentUpdate {
                name: None,
                hue: Some(None),
            },
        )
        .await
        .expect("unpinned");
    assert_eq!(unpinned.hue_override, None);
    assert_eq!(
        assert_forbidden(
            agents
                .update_agent(&t.ada, t.id, scout.id, rename("Mine"))
                .await
        ),
        "only its owner renames or recolours it"
    );
    assert_hidden(
        agents
            .update_agent(&t.gus, t.id, scout.id, rename("Mine"))
            .await,
    );

    let dee = t.h.person("dee@example.com").await;
    t.h.bring_in(&t.ada, t.id, &dee, WorkspaceRole::Member)
        .await;
    assert!(
        assert_forbidden(
            agents
                .transfer_agent(&t.cy, t.id, scout.id, dee.user.id)
                .await
        )
        .contains("owners and admins")
    );
    assert!(matches!(
        agents
            .transfer_agent(&t.bob, t.id, scout.id, t.gus.user.id)
            .await,
        Err(WorkspaceError::InvalidInput(_))
    ));
    let other = t.h.agent(&t.ada, t.id, "Other").await;
    assert!(matches!(
        agents
            .transfer_agent(&t.bob, t.id, scout.id, other.id)
            .await,
        Err(WorkspaceError::InvalidInput(_))
    ));
    assert_hidden(
        agents
            .transfer_agent(&t.bob, t.id, scout.id, Uuid::now_v7())
            .await,
    );
    // Not to someone disabled, with whom it would stay suspended.
    let admin = t.h.admin().await;
    let disable = |disabled| UserUpdate {
        disabled: Some(disabled),
        ..UserUpdate::default()
    };
    t.h.auth
        .update_user(&admin, dee.user.id, disable(true))
        .await
        .expect("disabled");
    assert!(matches!(
        agents
            .transfer_agent(&t.bob, t.id, scout.id, dee.user.id)
            .await,
        Err(WorkspaceError::InvalidInput(reason)) if reason.contains("disabled")
    ));
    assert_eq!(t.status(&scout).await, AgentStatus::Active, "still Cy's");
    t.h.auth
        .update_user(&admin, dee.user.id, disable(false))
        .await
        .expect("enabled");
    agents
        .transfer_agent(&t.bob, t.id, scout.id, dee.user.id)
        .await
        .expect("handed over");
    assert!(
        assert_forbidden(agents.delete_agent(&t.cy, t.id, scout.id).await)
            .contains("only its owner")
    );
    agents
        .delete_agent(&dee, t.id, scout.id)
        .await
        .expect("deleted");
    agents
        .delete_agent(&dee, t.id, scout.id)
        .await
        .expect("deleted again, nothing done");
    assert_eq!(
        assert_forbidden(
            agents
                .update_agent(&dee, t.id, scout.id, rename("Back"))
                .await
        ),
        "this agent is deleted"
    );
    agents
        .delete_agent(&t.bob, t.id, other.id)
        .await
        .expect("an admin deletes it");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn deleting_an_agent_keeps_what_it_made_named(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.cy, t.id, "Scout").await;
    let (secret, worker) = t.h.agent_token(&t.cy, &scout, &["write"]).await;
    let database =
        t.h.databases
            .create_database(
                &t.cy,
                t.id,
                Location::Teamspace(t.general),
                Placement::Last,
                NewDatabase {
                    title: "Tasks".to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("made");
    let db = database.page.page.id;
    let assignee =
        t.h.databases
            .add_property(
                &t.cy,
                db,
                NewProperty {
                    name: "Assignee".to_owned(),
                    spec: PropertySpec {
                        kind: PropertyKind::Person,
                        number_format: None,
                        currency: None,
                        prefix: None,
                    },
                    options: None,
                    placement: Placement::Last,
                },
            )
            .await
            .expect("added");
    let values = |title: &str, people: Vec<Uuid>| {
        let mut values = Map::new();
        values.insert("Name".to_owned(), json!(title));
        values.insert(assignee.id.to_string(), json!(people));
        values
    };
    let row =
        t.h.databases
            .create_row(
                &worker,
                db,
                NewRow {
                    values: values("Triage", vec![scout.id]),
                    placement: Placement::Last,
                    group: None,
                },
            )
            .await
            .expect("an agent makes a row, assigned to itself");
    assert_eq!(row.created_by, Some(scout.id));

    // Integrations are not given work.
    let integration = Uuid::now_v7();
    neon_store::agents::insert(
        &mut t.h.pool.acquire().await.expect("a connection"),
        &neon_store::agents::New {
            id: integration,
            kind: UserKind::Integration,
            workspace_id: t.id,
            owner_id: t.ada.user.id,
            name: "GitHub",
            hue_override: None,
            at: t.h.time.clock().now(),
        },
    )
    .await
    .expect("an integration");
    assert!(matches!(
        t.h.databases
            .create_row(
                &t.cy,
                db,
                NewRow {
                    values: values("Hook", vec![integration]),
                    placement: Placement::Last,
                    group: None,
                },
            )
            .await,
        Err(neon_app::databases::DatabaseError::InvalidInput(_))
    ));
    assert_forbidden(
        t.h.agents
            .create_agent_token(&t.ada, t.id, integration, "hook", None, &["read"])
            .await,
    );

    let table =
        t.h.databases
            .export_table(&t.cy, db)
            .await
            .expect("exported");
    let column = table
        .columns
        .iter()
        .position(|c| c == "Assignee")
        .expect("a column");
    assert_eq!(table.rows[0][column], "Scout · via cy");

    t.h.agents
        .delete_agent(&t.cy, t.id, scout.id)
        .await
        .expect("deleted");
    assert!(t.h.auth.authenticate_token(secret.expose()).await.is_err());
    assert!(t.h.auth.refresh(&worker).await.is_err());
    let listed = t.h.agents.list_agents(&t.ada, t.id).await.expect("listed");
    assert_eq!(
        listed.last().map(|agent| (agent.id, agent.status)),
        Some((scout.id, AgentStatus::Deleted))
    );
    assert!(
        t.h.workspaces
            .list_members(&t.ada, t.id)
            .await
            .expect("listed")
            .iter()
            .all(|member| member.person.id != scout.id)
    );
    let kept = t.h.databases.row(&t.cy, row.id).await.expect("read");
    assert_eq!(kept.created_by, Some(scout.id));
    let table =
        t.h.databases
            .export_table(&t.cy, db)
            .await
            .expect("exported");
    assert_eq!(table.rows[0][column], "Scout · via cy (deleted)");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_bound_token_works_in_its_workspace_alone(pool: PgPool) {
    let t = team(pool).await;
    let other = t.h.workspace(&t.cy, "Elsewhere").await.workspace.id;
    let auth = &t.h.auth;
    let bound = |by: &Principal, scopes: &'static [&'static str], workspace| {
        let by = by.clone();
        async move {
            let (secret, token) = auth
                .create_api_token(&by, "bound", None, scopes, Some(workspace))
                .await?;
            assert_eq!(token.workspace_id, Some(workspace));
            auth.authenticate_token(secret.expose()).await
        }
    };

    let cy = bound(&t.cy, &["read", "write"], t.id).await.expect("works");
    assert_eq!(cy.workspace(), Some(t.id));
    let listed = t.h.workspaces.list_workspaces(&cy).await.expect("listed");
    assert_eq!(listed.iter().map(|(w, _)| w.id).collect::<Vec<_>>(), [t.id]);
    assert_hidden(t.h.workspaces.workspace(&cy, other).await);
    assert_hidden(t.h.workspaces.list_teamspaces(&cy, other).await);
    t.page(&cy, t.general, "Works here").await.expect("made");

    // The admin scope only for owners and admins, for this workspace's admin work.
    assert!(matches!(
        bound(&t.cy, &["admin"], t.id).await,
        Err(AuthError::Forbidden(_))
    ));
    assert!(matches!(
        bound(&t.cy, &["read"], Uuid::now_v7()).await,
        Err(AuthError::NotFound(_))
    ));
    let ada_writes = bound(&t.ada, &["write"], t.id).await.expect("works");
    assert_eq!(
        assert_forbidden(
            t.h.workspaces
                .update_workspace(&ada_writes, t.id, WorkspaceUpdate::default())
                .await
        ),
        "only the workspace's owners and admins may do this"
    );
    let ada_admin = bound(&t.ada, &["write", "admin"], t.id)
        .await
        .expect("works");
    t.update(&ada_admin, WorkspaceUpdate::default()).await;
    assert!(!ada_admin.is_admin());
}

/// `neon_agents_operations_total` of one operation and status.
fn count(snapshotter: &Snapshotter, operation: &str, status: &str) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let (MetricKind::Counter, DebugValue::Counter(count)) = (kind, value) else {
                return None;
            };
            let labels: Vec<(String, String)> = key
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect();
            let wanted = key.name() == "neon_agents_operations_total"
                && labels.contains(&("operation".to_owned(), operation.to_owned()))
                && labels.contains(&("status".to_owned(), status.to_owned()));
            wanted.then_some(count)
        })
        .sum()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn agent_use_cases_are_measured_with_refusals_as_ok(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recording = metrics::set_default_local_recorder(&recorder);
    let t = team(pool).await;
    let agents = neon_app::workspaces::AgentService::new(t.h.workspaces.clone());

    agents
        .create_agent(&t.cy, t.id, new("Scout"))
        .await
        .expect("made");
    assert_forbidden(agents.create_agent(&t.gus, t.id, new("Peek")).await);

    assert_eq!(count(&snapshotter, "create_agent", "ok"), 2);
    assert_eq!(count(&snapshotter, "create_agent", "error"), 0);
}
