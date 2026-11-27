//! `/api/v1/admin/workspaces`: the instance admins' view of workspaces. They see each
//! workspace's name, owners, member count, limits and usage, set its limits and delete it,
//! and never what is inside: the workspace routes answer them as anyone outside.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::workspaces::{AdminWorkspace as AppAdminWorkspace, LimitsUpdate};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::dto::{Person, nullable};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::principal::Admin;

/// One limit of a workspace.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Limit {
    /// Set for this workspace alone: 0 is no limit. Absent, it follows the instance's
    /// default, the setting `workspaces.limits.*`.
    pub own: Option<i64>,
    /// What holds: absent is no limit.
    pub effective: Option<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceLimits {
    /// People, guests included.
    pub members: Limit,
    pub storage_bytes: Limit,
    pub agents: Limit,
}

/// What a workspace holds.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceUsage {
    pub members: i64,
    /// The bytes its attachments take together, as its storage limit counts them.
    pub storage_bytes: i64,
    /// Its agents and integrations together, deleted ones left out, as its agent limit
    /// counts them.
    pub agents: i64,
}

/// A workspace as instance admins see it: nothing of what is inside.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminWorkspace {
    pub id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    #[schema(example = "design")]
    pub slug: String,
    /// Oldest first.
    pub owners: Vec<Person>,
    pub member_count: i64,
    pub limits: WorkspaceLimits,
    pub usage: WorkspaceUsage,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<AppAdminWorkspace> for AdminWorkspace {
    fn from(entry: AppAdminWorkspace) -> Self {
        let (own, effective) = (entry.overrides, entry.limits);
        Self {
            id: entry.workspace.id,
            name: entry.workspace.name,
            slug: entry.workspace.slug,
            owners: entry.owners.into_iter().map(Person::from).collect(),
            member_count: entry.usage.members,
            limits: WorkspaceLimits {
                members: Limit {
                    own: own.members,
                    effective: effective.members,
                },
                storage_bytes: Limit {
                    own: own.storage_bytes,
                    effective: effective.storage_bytes,
                },
                agents: Limit {
                    own: own.agents,
                    effective: effective.agents,
                },
            },
            usage: WorkspaceUsage {
                members: entry.usage.members,
                storage_bytes: entry.usage.storage_bytes,
                agents: entry.usage.agents,
            },
            created_at: entry.workspace.created_at,
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListWorkspaces {
    /// Workspaces on the page, 1 to 200; 50 when absent.
    pub limit: Option<i64>,
    /// The `next` of the page before.
    pub after: Option<Uuid>,
}

/// A page of workspaces in the order they were made.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminWorkspacePage {
    pub workspaces: Vec<AdminWorkspace>,
    /// Pass it as `after` for the next page; absent on the last one.
    pub next: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/admin/workspaces",
    tag = "admin",
    summary = "List workspaces",
    description = "Every workspace on the instance with its owners, member count, limits and \
                   usage. Nothing of what is in them.",
    params(ListWorkspaces),
    responses(
        (status = OK, description = "A page of workspaces.", body = AdminWorkspacePage),
        (status = BAD_REQUEST, description = "invalid_input or invalid_query.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn admin_list_workspaces(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Query(query): Query<ListWorkspaces>,
) -> Result<Json<AdminWorkspacePage>, ApiError> {
    let page = state
        .workspaces
        .admin_workspaces(&admin, query.after, query.limit)
        .await?;
    Ok(Json(AdminWorkspacePage {
        workspaces: page
            .workspaces
            .into_iter()
            .map(AdminWorkspace::from)
            .collect(),
        next: page.next,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetLimits {
    /// The most people, guests included: 0 for no limit, null to follow the instance's
    /// default. Absent stays as it is.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0)]
    pub members: Option<Option<i64>>,
    /// The most bytes of attachments, likewise.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0)]
    pub storage_bytes: Option<Option<i64>>,
    /// The most agents, likewise.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0)]
    pub agents: Option<Option<i64>>,
}

#[utoipa::path(
    patch,
    path = "/admin/workspaces/{id}/limits",
    tag = "admin",
    summary = "Set a workspace's limits",
    description = "Changes the limits the body names. A workspace past a new limit keeps \
                   everyone in it, and takes no one more.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = SetLimits,
    responses(
        (status = OK, description = "The workspace.", body = AdminWorkspace),
        (status = BAD_REQUEST, description = "invalid_input: a limit below 0.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn set_workspace_limits(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
    Json(body): Json<SetLimits>,
) -> Result<Json<AdminWorkspace>, ApiError> {
    let update = LimitsUpdate {
        members: body.members,
        storage_bytes: body.storage_bytes,
        agents: body.agents,
    };
    let entry = state
        .workspaces
        .set_workspace_limits(&admin, id, update)
        .await?;
    Ok(Json(entry.into()))
}

#[utoipa::path(
    delete,
    path = "/admin/workspaces/{id}",
    tag = "admin",
    summary = "Delete a workspace",
    description = "Deletes it with everything in it. Only from a browser session.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, as for an API token, or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn admin_delete_workspace(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.workspaces.admin_delete_workspace(&admin, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
