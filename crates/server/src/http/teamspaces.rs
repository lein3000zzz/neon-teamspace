//! `/api/v1/workspaces/{id}/teamspaces`: a workspace's teamspaces and their members.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. A guest sees no teamspace, and a member outside a private teamspace does not see
//! it: both get `not_found` for it.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::workspaces::{
    NewTeamspace, TeamspaceEntry, TeamspaceList as AppTeamspaceList, TeamspaceUpdate,
};
use neon_domain::workspaces as domain;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::dto::{Person, nullable};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path};
use super::principal::Authenticated;

/// Who sees a teamspace and who may join it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamspaceAccess {
    /// The workspace's own, which everyone but guests is in.
    Default,
    /// Every member sees it and joins at will.
    Open,
    /// Every member sees it; its owners add who joins.
    Closed,
    /// Only its members see it.
    Private,
}

impl From<domain::TeamspaceAccess> for TeamspaceAccess {
    fn from(access: domain::TeamspaceAccess) -> Self {
        match access {
            domain::TeamspaceAccess::Default => Self::Default,
            domain::TeamspaceAccess::Open => Self::Open,
            domain::TeamspaceAccess::Closed => Self::Closed,
            domain::TeamspaceAccess::Private => Self::Private,
        }
    }
}

impl From<TeamspaceAccess> for domain::TeamspaceAccess {
    fn from(access: TeamspaceAccess) -> Self {
        match access {
            TeamspaceAccess::Default => Self::Default,
            TeamspaceAccess::Open => Self::Open,
            TeamspaceAccess::Closed => Self::Closed,
            TeamspaceAccess::Private => Self::Private,
        }
    }
}

/// What a person may do in a teamspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamspaceRole {
    Owner,
    Member,
}

impl From<domain::TeamspaceRole> for TeamspaceRole {
    fn from(role: domain::TeamspaceRole) -> Self {
        match role {
            domain::TeamspaceRole::Owner => Self::Owner,
            domain::TeamspaceRole::Member => Self::Member,
        }
    }
}

impl From<TeamspaceRole> for domain::TeamspaceRole {
    fn from(role: TeamspaceRole) -> Self {
        match role {
            TeamspaceRole::Owner => Self::Owner,
            TeamspaceRole::Member => Self::Member,
        }
    }
}

/// A teamspace the caller may see.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Teamspace {
    pub id: Uuid,
    pub workspace_id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    pub icon: Option<String>,
    /// The hue it is drawn in, 0 to 359: the one it pinned, or else its id's.
    #[schema(minimum = 0, maximum = 359, example = 98)]
    pub hue: u16,
    #[schema(minimum = 0, maximum = 359)]
    pub hue_override: Option<u16>,
    pub access: TeamspaceAccess,
    /// The caller's role in it; absent when they are not in it.
    pub role: Option<TeamspaceRole>,
    pub member_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<TeamspaceEntry> for Teamspace {
    fn from(entry: TeamspaceEntry) -> Self {
        let teamspace = entry.teamspace;
        Self {
            hue: teamspace.hue(),
            id: teamspace.id,
            workspace_id: teamspace.workspace_id,
            name: teamspace.name,
            icon: teamspace.icon,
            hue_override: teamspace.hue_override,
            access: teamspace.access.into(),
            role: entry.role.map(Into::into),
            member_count: entry.members,
            created_at: teamspace.created_at,
            updated_at: teamspace.updated_at,
        }
    }
}

/// A private teamspace the caller is not in, by name only.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TeamspaceName {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TeamspaceList {
    /// The teamspaces the caller sees, the default one first, then by name. None for a
    /// guest.
    pub teamspaces: Vec<Teamspace>,
    /// For owners and admins of the workspace: the private teamspaces they are not in, by
    /// name only, to manage or delete them.
    pub hidden: Vec<TeamspaceName>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/teamspaces",
    tag = "teamspaces",
    summary = "List the teamspaces the caller may see",
    description = "With the caller's role in each. Private teamspaces show only to their \
                   members; the workspace's owners and admins also get their names.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The teamspaces.", body = TeamspaceList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_teamspaces(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<TeamspaceList>, ApiError> {
    let list = state.workspaces.list_teamspaces(&principal, id).await?;
    Ok(Json(list.into()))
}

impl From<AppTeamspaceList> for TeamspaceList {
    fn from(list: AppTeamspaceList) -> Self {
        Self {
            teamspaces: list.teamspaces.into_iter().map(Teamspace::from).collect(),
            hidden: list
                .hidden
                .into_iter()
                .map(|teamspace| TeamspaceName {
                    id: teamspace.id,
                    name: teamspace.name,
                })
                .collect(),
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTeamspace {
    /// 1 to 100 characters.
    #[schema(example = "Design")]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// 0 to 359; the id's hue when absent.
    #[serde(default)]
    #[schema(minimum = 0, maximum = 359)]
    pub hue_override: Option<i64>,
    /// Open, closed or private.
    pub access: TeamspaceAccess,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/teamspaces",
    tag = "teamspaces",
    summary = "Create a teamspace",
    description = "Open, closed or private, with the caller as its owner. Owners and admins \
                   create teamspaces, and members too unless the workspace's \
                   `members_can_create_teamspaces` is off.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateTeamspace,
    responses(
        (status = CREATED, description = "The teamspace.", body = Teamspace),
        (status = BAD_REQUEST, description = "invalid_input, as for the default access.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateTeamspace>,
) -> Result<(StatusCode, Json<Teamspace>), ApiError> {
    let created = state
        .workspaces
        .create_teamspace(
            &principal,
            id,
            NewTeamspace {
                name: body.name,
                icon: body.icon,
                hue: body.hue_override,
                access: body.access.into(),
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(created.into())))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}",
    tag = "teamspaces",
    summary = "A teamspace",
    description = "A private teamspace answers `not_found` to everyone outside it, the \
                   workspace's owners and admins included.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    responses(
        (status = OK, description = "The teamspace.", body = Teamspace),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Teamspace>, ApiError> {
    let entry = state
        .workspaces
        .teamspace(&principal, id, teamspace_id)
        .await?;
    Ok(Json(entry.into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateTeamspace {
    #[serde(default)]
    pub name: Option<String>,
    /// Null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub icon: Option<Option<String>>,
    /// 0 to 359; null goes back to the id's hue.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0, maximum = 359)]
    pub hue_override: Option<Option<i64>>,
    /// Open, closed or private; the default teamspace keeps its access.
    #[serde(default)]
    pub access: Option<TeamspaceAccess>,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}",
    tag = "teamspaces",
    summary = "Change a teamspace",
    description = "Changes what the body names. For the teamspace's owners and the \
                   workspace's owners and admins, who join a private one first.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    request_body = UpdateTeamspace,
    responses(
        (status = OK, description = "The teamspace.", body = Teamspace),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, as for the default teamspace's access, or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateTeamspace>,
) -> Result<Json<Teamspace>, ApiError> {
    let update = TeamspaceUpdate {
        name: body.name,
        icon: body.icon,
        hue: body.hue_override,
        access: body.access.map(Into::into),
    };
    let entry = state
        .workspaces
        .update_teamspace(&principal, id, teamspace_id, update)
        .await?;
    Ok(Json(entry.into()))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}",
    tag = "teamspaces",
    summary = "Delete a teamspace",
    description = "Any but the default one. For the teamspace's owners and the workspace's \
                   owners and admins, who delete a private one without being in it.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .delete_teamspace(&principal, id, teamspace_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}/join",
    tag = "teamspaces",
    summary = "Join a teamspace",
    description = "Any member joins an open teamspace. Closed and private ones take whom their \
                   owners add; the workspace's owners and admins may join them themselves, which \
                   is how they come to manage a private one.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    responses(
        (status = OK, description = "The teamspace.", body = Teamspace),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn join_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Teamspace>, ApiError> {
    let entry = state
        .workspaces
        .join_teamspace(&principal, id, teamspace_id)
        .await?;
    Ok(Json(entry.into()))
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}/leave",
    tag = "teamspaces",
    summary = "Leave a teamspace",
    description = "Any but the default one. Its last owner may not, unless they own or \
                   administer the workspace.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Left."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "last_teamspace_owner.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn leave_teamspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .leave_teamspace(&principal, id, teamspace_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A member of a teamspace.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TeamspaceMember {
    pub user: Person,
    pub role: TeamspaceRole,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
}

impl From<domain::TeamspaceMember> for TeamspaceMember {
    fn from(member: domain::TeamspaceMember) -> Self {
        Self {
            user: member.person.into(),
            role: member.role.into(),
            joined_at: member.joined_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TeamspaceMemberList {
    /// Owners first, then by name.
    pub members: Vec<TeamspaceMember>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}/members",
    tag = "teamspaces",
    summary = "List a teamspace's members",
    description = "For everyone who may see the teamspace: a private one's members are listed \
                   to its members only.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    responses(
        (status = OK, description = "The members.", body = TeamspaceMemberList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_teamspace_members(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<TeamspaceMemberList>, ApiError> {
    let members = state
        .workspaces
        .teamspace_members(&principal, id, teamspace_id)
        .await?;
    Ok(Json(TeamspaceMemberList {
        members: members.into_iter().map(TeamspaceMember::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddTeamspaceMember {
    /// A member of the workspace who is not a guest.
    pub user_id: Uuid,
    /// `member` when absent.
    #[serde(default)]
    pub role: Option<TeamspaceRole>,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}/members",
    tag = "teamspaces",
    summary = "Add a member to a teamspace",
    description = "Any teamspace but the default one, which everyone is in. For the \
                   teamspace's owners and the workspace's owners and admins, who join a private \
                   one first. Guests are added to none.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
    ),
    request_body = AddTeamspaceMember,
    responses(
        (status = CREATED, description = "The member.", body = TeamspaceMember),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, as for a guest, who is in no teamspace; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such teamspace, or the account is not in the workspace.", body = ErrorBody),
        (status = CONFLICT, description = "already_member.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn add_teamspace_member(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<AddTeamspaceMember>,
) -> Result<(StatusCode, Json<TeamspaceMember>), ApiError> {
    let member = state
        .workspaces
        .add_teamspace_member(
            &principal,
            id,
            teamspace_id,
            body.user_id,
            body.role.unwrap_or(TeamspaceRole::Member).into(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(member.into())))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeTeamspaceRole {
    pub role: TeamspaceRole,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}/members/{user_id}",
    tag = "teamspaces",
    summary = "Change a teamspace member's role",
    description = "For the teamspace's owners and the workspace's owners and admins, who join a \
                   private one first. The last owner stays one unless an owner or admin of the \
                   workspace changes it.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
        ("user_id" = Uuid, Path, description = "The member's account id."),
    ),
    request_body = ChangeTeamspaceRole,
    responses(
        (status = OK, description = "The member.", body = TeamspaceMember),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "last_teamspace_owner.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_teamspace_role(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id, user_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(body): Json<ChangeTeamspaceRole>,
) -> Result<Json<TeamspaceMember>, ApiError> {
    let member = state
        .workspaces
        .change_teamspace_role(&principal, id, teamspace_id, user_id, body.role.into())
        .await?;
    Ok(Json(member.into()))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/teamspaces/{teamspace_id}/members/{user_id}",
    tag = "teamspaces",
    summary = "Remove a member from a teamspace",
    description = "Any teamspace but the default one. For the teamspace's owners and the \
                   workspace's owners and admins, who join a private one first.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("teamspace_id" = Uuid, Path, description = "The teamspace's id."),
        ("user_id" = Uuid, Path, description = "The member's account id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Removed."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "last_teamspace_owner.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn remove_teamspace_member(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, teamspace_id, user_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .remove_teamspace_member(&principal, id, teamspace_id, user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
