//! What instance admins do with workspaces, and the RED metrics and spans of the workspace
//! use cases.

use crate::support;

use metrics_util::MetricKind;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use neon_app::workspaces::{LimitsUpdate, WorkspaceError};
use neon_domain::auth::{Credential, Principal, Role, Scope};
use neon_domain::workspaces::{LimitOverrides, WorkspaceRole};
use serde_json::json;
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;

/// The same admin, through a token with the admin scope.
fn admin_token(principal: &Principal) -> Principal {
    Principal {
        user: principal.user.clone(),
        credential: Credential::Token {
            id: Uuid::now_v7(),
            name: "ops".into(),
            scopes: vec![Scope::Admin],
            workspace_id: None,
        },
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn instance_admins_list_names_owners_counts_and_limits(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let design = h.workspace(&ada, "Design").await.workspace.id;
    let ops = h.workspace(&bob, "Ops").await.workspace.id;
    h.bring_in(&ada, design, &bob, WorkspaceRole::Admin).await;
    h.workspaces
        .change_member_role(&ada, design, bob.user.id, WorkspaceRole::Owner)
        .await
        .expect("a second owner");
    h.set("workspaces.limits.members", json!(25)).await;

    let page = h
        .workspaces
        .admin_workspaces(&admin, None, None)
        .await
        .expect("listed");

    assert_eq!(page.next, None);
    let ids: Vec<Uuid> = page
        .workspaces
        .iter()
        .map(|entry| entry.workspace.id)
        .collect();
    assert_eq!(ids, [design, ops], "in the order they were made");
    let first = &page.workspaces[0];
    let owners: Vec<&str> = first
        .owners
        .iter()
        .filter_map(|p| p.email.as_deref())
        .collect();
    assert_eq!(owners, ["ada@example.com", "bob@example.com"]);
    assert_eq!(first.usage.members, 2);
    assert_eq!(first.usage.storage_bytes, 0);
    assert_eq!(first.overrides, LimitOverrides::default());
    assert_eq!(first.limits.members, Some(25));
    assert_eq!(first.limits.agents, None);

    let paged = h
        .workspaces
        .admin_workspaces(&admin, None, Some(1))
        .await
        .expect("listed");
    assert_eq!(paged.workspaces.len(), 1);
    assert_eq!(paged.next, Some(design));
    let rest = h
        .workspaces
        .admin_workspaces(&admin, paged.next, Some(1))
        .await
        .expect("listed");
    assert_eq!(rest.workspaces[0].workspace.id, ops);
    assert_eq!(rest.next, None);
    let too_many = h.workspaces.admin_workspaces(&admin, None, Some(201)).await;
    assert!(matches!(too_many, Err(WorkspaceError::InvalidInput(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn instance_admins_set_limits_and_delete_without_entering(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    let ada = h.person("ada@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.set("workspaces.limits.agents", json!(3)).await;

    let set = h
        .workspaces
        .set_workspace_limits(
            &admin,
            id,
            LimitsUpdate {
                members: Some(Some(10)),
                storage_bytes: Some(Some(1 << 30)),
                agents: None,
            },
        )
        .await
        .expect("set");
    assert_eq!(
        set.overrides,
        LimitOverrides {
            members: Some(10),
            storage_bytes: Some(1 << 30),
            agents: None,
        }
    );
    assert_eq!(set.limits.members, Some(10));
    assert_eq!(set.limits.agents, Some(3), "the default where none is set");
    let back = h
        .workspaces
        .set_workspace_limits(
            &admin,
            id,
            LimitsUpdate {
                members: Some(None),
                ..LimitsUpdate::default()
            },
        )
        .await
        .expect("set");
    assert_eq!(back.overrides.members, None);
    assert_eq!(
        back.overrides.storage_bytes,
        Some(1 << 30),
        "the rest stays"
    );
    let negative = h
        .workspaces
        .set_workspace_limits(
            &admin,
            id,
            LimitsUpdate {
                agents: Some(Some(-1)),
                ..LimitsUpdate::default()
            },
        )
        .await;
    assert!(matches!(negative, Err(WorkspaceError::InvalidInput(_))));
    let missing = h
        .workspaces
        .set_workspace_limits(&admin, Uuid::now_v7(), LimitsUpdate::default())
        .await;
    assert!(matches!(missing, Err(WorkspaceError::NotFound(_))));

    let inside = h.workspaces.workspace(&admin, id).await;
    assert!(
        matches!(inside, Err(WorkspaceError::NotFound(_))),
        "no content access: {inside:?}"
    );
    let by_token = h
        .workspaces
        .admin_delete_workspace(&admin_token(&admin), id)
        .await;
    assert!(matches!(by_token, Err(WorkspaceError::Forbidden(_))));
    h.workspaces
        .admin_delete_workspace(&admin, id)
        .await
        .expect("deleted");
    assert!(
        h.workspaces
            .list_workspaces(&ada)
            .await
            .expect("listed")
            .is_empty()
    );
    let again = h.workspaces.admin_delete_workspace(&admin, id).await;
    assert!(matches!(again, Err(WorkspaceError::NotFound(_))));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn people_who_are_not_instance_admins_do_none_of_it(pool: PgPool) {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    let demoted = h.account("former@example.com", Role::Admin).await;
    let former = Principal {
        user: demoted,
        credential: Credential::Token {
            id: Uuid::now_v7(),
            name: "old".into(),
            scopes: vec![Scope::Read, Scope::Write],
            workspace_id: None,
        },
    };

    for who in [&ada, &former] {
        let list = h.workspaces.admin_workspaces(who, None, None).await;
        assert!(
            matches!(list, Err(WorkspaceError::Forbidden(_))),
            "{list:?}"
        );
        let limits = h
            .workspaces
            .set_workspace_limits(who, id, LimitsUpdate::default())
            .await;
        assert!(matches!(limits, Err(WorkspaceError::Forbidden(_))));
        let delete = h.workspaces.admin_delete_workspace(who, id).await;
        assert!(matches!(delete, Err(WorkspaceError::Forbidden(_))));
    }
}

fn count(snapshotter: &Snapshotter, operation: &str, status: &str) -> u64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter_map(|(key, _, _, value)| {
            let (kind, key) = key.into_parts();
            let label = |name: &str| {
                key.labels()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
            };
            match (kind, value) {
                (MetricKind::Counter, DebugValue::Counter(n))
                    if key.name() == "neon_workspaces_operations_total"
                        && label("operation").as_deref() == Some(operation)
                        && label("status").as_deref() == Some(status) =>
                {
                    Some(n)
                }
                _ => None,
            }
        })
        .sum()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn refusals_count_as_ok_and_failures_as_errors(pool: PgPool) {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics = metrics::set_default_local_recorder(&recorder);
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;

    let refused = h.workspaces.workspace(&bob, id).await;
    assert!(matches!(refused, Err(WorkspaceError::NotFound(_))));
    h.workspaces.workspace(&ada, id).await.expect("seen");
    assert_eq!(count(&snapshotter, "get_workspace", "ok"), 2);

    h.pool.close().await;
    let failed = h.workspaces.workspace(&ada, id).await;
    assert!(
        matches!(failed, Err(WorkspaceError::Database(_))),
        "{failed:?}"
    );
    assert_eq!(count(&snapshotter, "get_workspace", "error"), 1);
}
