//! `/api/v1/workspaces`: the caller's workspaces, their members, the invitations they send
//! and their invite links; and `/api/v1/workspace-invite-links/join`.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. Who may do what in a workspace is the workspace rules' to say; someone outside a
//! workspace, an instance admin included, gets `not_found` for it. A token bound to a
//! workspace works on its own workspace's routes, and lists that workspace alone; making a
//! workspace and joining one by link need a person's own session or token.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::workspaces::{
    Joined, LinkPreview, LinkSpec, NewWorkspace, SentInvitation as AppSentInvitation,
    WorkspaceDetails, WorkspaceUpdate,
};
use neon_domain::agents::AgentCreation as DomainAgentCreation;
use neon_domain::settings::Secret;
use neon_domain::workspaces::{self as domain};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::account::InvitingWorkspace;
use super::dto::{InviteStatus, Person, TokenRequest, WorkspaceRole, nullable};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path};
use super::principal::{Account, Authenticated};

/// A workspace the caller is in, as their list shows it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceSummary {
    pub id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    /// For URLs: lowercase letters, digits and dashes, unique on the instance.
    #[schema(example = "design")]
    pub slug: String,
    /// An emoji or a short text.
    #[schema(example = "🚀")]
    pub icon: Option<String>,
    /// The hue it is drawn in, 0 to 359: the one it pinned, or else its id's.
    #[schema(minimum = 0, maximum = 359, example = 250)]
    pub hue: u16,
    /// The hue it pinned, if any.
    #[schema(minimum = 0, maximum = 359)]
    pub hue_override: Option<u16>,
    /// The caller's role in it.
    pub role: WorkspaceRole,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl WorkspaceSummary {
    fn new(workspace: &domain::Workspace, role: domain::WorkspaceRole) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name.clone(),
            slug: workspace.slug.clone(),
            icon: workspace.icon.clone(),
            hue: workspace.hue(),
            hue_override: workspace.hue_override,
            role: role.into(),
            created_at: workspace.created_at,
        }
    }
}

impl From<Joined> for WorkspaceSummary {
    fn from(joined: Joined) -> Self {
        Self::new(&joined.workspace, joined.role)
    }
}

/// The limits that hold for a workspace; absent is no limit.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EffectiveLimits {
    /// The most people in it, guests included.
    pub members: Option<u64>,
    pub storage_bytes: Option<u64>,
    /// The most agents and integrations, together.
    pub agents: Option<u64>,
}

impl From<domain::Limits> for EffectiveLimits {
    fn from(limits: domain::Limits) -> Self {
        Self {
            members: limits.members,
            storage_bytes: limits.storage_bytes,
            agents: limits.agents,
        }
    }
}

/// Who makes agents in a workspace: its owners and admins, or every member but guests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentCreation {
    Admins,
    Members,
}

impl From<DomainAgentCreation> for AgentCreation {
    fn from(creation: DomainAgentCreation) -> Self {
        match creation {
            DomainAgentCreation::Admins => Self::Admins,
            DomainAgentCreation::Members => Self::Members,
        }
    }
}

impl From<AgentCreation> for DomainAgentCreation {
    fn from(creation: AgentCreation) -> Self {
        match creation {
            AgentCreation::Admins => Self::Admins,
            AgentCreation::Members => Self::Members,
        }
    }
}

/// A workspace's agents as a whole: whether they work, and who makes them.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceAgents {
    /// Whether the instance has agents at all, as the setting `mcp.identities` says. Off,
    /// none is made and their tokens are refused, whatever the workspace says.
    pub available: bool,
    /// Whether the workspace has its agents on. Off, none is made and their tokens are
    /// refused.
    pub enabled: bool,
    pub creation: AgentCreation,
    /// How many agents and integrations it has, deleted ones left out. Absent for a
    /// guest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
}

/// A workspace the caller is in.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Workspace {
    #[serde(flatten)]
    pub summary: WorkspaceSummary,
    /// Members, besides owners and admins, may create teamspaces.
    pub members_can_create_teamspaces: bool,
    /// How many people are in it, guests included, agents and integrations left out.
    /// Absent for a guest, who knows of no member but themselves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<i64>,
    pub agents: WorkspaceAgents,
    pub limits: EffectiveLimits,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<WorkspaceDetails> for Workspace {
    fn from(details: WorkspaceDetails) -> Self {
        Self {
            summary: WorkspaceSummary::new(&details.workspace, details.role),
            members_can_create_teamspaces: details.workspace.members_can_create_teamspaces,
            member_count: details.members,
            agents: WorkspaceAgents {
                available: details.agents_on,
                enabled: details.workspace.agents_enabled,
                creation: details.workspace.agent_creation.into(),
                count: details.agents,
            },
            limits: details.limits.into(),
            updated_at: details.workspace.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceList {
    /// By name.
    pub workspaces: Vec<WorkspaceSummary>,
    /// Whether the caller may create a workspace, as the `workspaces.creation` setting says.
    pub can_create: bool,
}

#[utoipa::path(
    get,
    path = "/workspaces",
    tag = "workspaces",
    summary = "List the caller's workspaces",
    description = "Each workspace the caller is in, with their role, slug and hue, and whether \
                   they may create one.",
    responses(
        (status = OK, description = "The workspaces.", body = WorkspaceList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_workspaces(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<WorkspaceList>, ApiError> {
    let listed = state.workspaces.list_workspaces(&principal).await?;
    Ok(Json(WorkspaceList {
        workspaces: listed
            .iter()
            .map(|(workspace, role)| WorkspaceSummary::new(workspace, *role))
            .collect(),
        can_create: state.workspaces.may_create_workspace(&principal),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspace {
    /// 1 to 100 characters.
    #[schema(example = "Design")]
    pub name: String,
    /// Made from the name when absent, with a short suffix when another workspace has it.
    #[serde(default)]
    #[schema(example = "design")]
    pub slug: Option<String>,
    /// An emoji or up to 16 characters of text.
    #[serde(default)]
    pub icon: Option<String>,
    /// 0 to 359; the id's hue when absent.
    #[serde(default)]
    #[schema(minimum = 0, maximum = 359)]
    pub hue_override: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/workspaces",
    tag = "workspaces",
    summary = "Create a workspace",
    description = "Makes the workspace with its default teamspace, General, and the caller as \
                   the owner of both. `workspaces.creation` may leave this to instance admins.",
    request_body = CreateWorkspace,
    responses(
        (status = CREATED, description = "The workspace.", body = Workspace),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "token_bound: a token bound to a workspace; or forbidden: only instance admins create workspaces here; or insufficient_scope.", body = ErrorBody),
        (status = CONFLICT, description = "slug_taken: another workspace has the slug given.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_workspace(
    State(state): State<AppState>,
    Account(principal): Account,
    Json(body): Json<CreateWorkspace>,
) -> Result<(StatusCode, Json<Workspace>), ApiError> {
    let created = state
        .workspaces
        .create_workspace(
            &principal,
            NewWorkspace {
                name: body.name,
                slug: body.slug,
                icon: body.icon,
                hue: body.hue_override,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(created.into())))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}",
    tag = "workspaces",
    summary = "A workspace",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The workspace.", body = Workspace),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace, or the caller is not in it.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_workspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<Workspace>, ApiError> {
    Ok(Json(
        state.workspaces.workspace(&principal, id).await?.into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWorkspace {
    #[serde(default)]
    pub name: Option<String>,
    /// A new name leaves the slug as it is; this changes it.
    #[serde(default)]
    pub slug: Option<String>,
    /// Null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub icon: Option<Option<String>>,
    /// 0 to 359; null goes back to the id's hue.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0, maximum = 359)]
    pub hue_override: Option<Option<i64>>,
    #[serde(default)]
    pub members_can_create_teamspaces: Option<bool>,
    /// Turns the workspace's agents on or off. Off, none is made and every agent's tokens
    /// are refused, its open connections closed.
    #[serde(default)]
    pub agents_enabled: Option<bool>,
    /// Who makes agents in it.
    #[serde(default)]
    pub agent_creation: Option<AgentCreation>,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}",
    tag = "workspaces",
    summary = "Change a workspace",
    description = "Changes what the body names. For owners and admins.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = UpdateWorkspace,
    responses(
        (status = OK, description = "The workspace.", body = Workspace),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "slug_taken.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_workspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateWorkspace>,
) -> Result<Json<Workspace>, ApiError> {
    let update = WorkspaceUpdate {
        name: body.name,
        slug: body.slug,
        icon: body.icon,
        hue: body.hue_override,
        members_can_create_teamspaces: body.members_can_create_teamspaces,
        agents_enabled: body.agents_enabled,
        agent_creation: body.agent_creation.map(Into::into),
    };
    Ok(Json(
        state
            .workspaces
            .update_workspace(&principal, id, update)
            .await?
            .into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteWorkspace {
    /// The workspace's name, typed to confirm.
    #[schema(example = "Design")]
    pub confirm_name: String,
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}",
    tag = "workspaces",
    summary = "Delete a workspace",
    description = "Deletes it with everything in it, for an owner who types its name. Only from \
                   a browser session.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = DeleteWorkspace,
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = BAD_REQUEST, description = "confirmation_mismatch: the name typed is not the workspace's.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not an owner, or not a browser session; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn delete_workspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<DeleteWorkspace>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .delete_workspace(&principal, id, &body.confirm_name)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A member of a workspace.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Member {
    pub user: Person,
    /// An agent's or integration's is always member.
    pub role: WorkspaceRole,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
    /// An agent or integration whose owner is out of the workspace or disabled: its
    /// tokens are refused until that ends, or it is handed to someone else. Never a person.
    pub suspended: bool,
}

impl From<domain::Member> for Member {
    fn from(member: domain::Member) -> Self {
        Self {
            user: member.person.into(),
            role: member.role.into(),
            joined_at: member.joined_at,
            suspended: member.suspended,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberList {
    /// Owners first, then admins, members and guests, each by name.
    pub members: Vec<Member>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/members",
    tag = "workspaces",
    summary = "List a workspace's members",
    description = "Every member, or for a guest, only themselves.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The members.", body = MemberList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_members(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<MemberList>, ApiError> {
    let members = state.workspaces.list_members(&principal, id).await?;
    Ok(Json(MemberList {
        members: members.into_iter().map(Member::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeRole {
    pub role: WorkspaceRole,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}/members/{user_id}",
    tag = "workspaces",
    summary = "Change a member's role",
    description = "Owners change anyone's role, owner included; admins anyone's but an owner's, \
                   and make no owners. A member who becomes a guest leaves every teamspace; a \
                   guest who becomes more joins the default one.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("user_id" = Uuid, Path, description = "The member's account id."),
    ),
    request_body = ChangeRole,
    responses(
        (status = OK, description = "The member.", body = Member),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace or member.", body = ErrorBody),
        (status = CONFLICT, description = "last_owner: the workspace would have no owner.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_member_role(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ChangeRole>,
) -> Result<Json<Member>, ApiError> {
    let member = state
        .workspaces
        .change_member_role(&principal, id, user_id, body.role.into())
        .await?;
    Ok(Json(member.into()))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/members/{user_id}",
    tag = "workspaces",
    summary = "Remove a member",
    description = "Takes them out of the workspace and its teamspaces. Owners remove anyone, \
                   admins anyone but an owner. To remove yourself, leave.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("user_id" = Uuid, Path, description = "The member's account id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Removed."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "last_owner.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn remove_member(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .remove_member(&principal, id, user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/leave",
    tag = "workspaces",
    summary = "Leave a workspace",
    description = "Leaves it and its teamspaces. The last owner cannot: they make another \
                   owner first, or delete the workspace. Nor can the last owner of one of its \
                   teamspaces, unless they own or administer the workspace: they make another \
                   owner of it first, or delete it.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = NO_CONTENT, description = "Left."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "last_owner or last_teamspace_owner.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn leave_workspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.workspaces.leave_workspace(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferOwnership {
    /// The member who becomes an owner.
    pub user_id: Uuid,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/transfer",
    tag = "workspaces",
    summary = "Transfer ownership",
    description = "Makes another member an owner and the owner calling an admin, together. Only \
                   from a browser session.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = TransferOwnership,
    responses(
        (status = NO_CONTENT, description = "Transferred."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not an owner, the member is one already, or not a browser session; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace or member.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn transfer_ownership(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferOwnership>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .transfer_ownership(&principal, id, body.user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An invitation a workspace sent, waiting for its answer.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SentInvitation {
    pub id: Uuid,
    /// The account invited.
    pub invitee: Person,
    /// What they join as: admin, member or guest.
    pub role: WorkspaceRole,
    /// Who invited them; absent once their account is gone.
    pub invited_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

impl From<AppSentInvitation> for SentInvitation {
    fn from(sent: AppSentInvitation) -> Self {
        let invitation = sent.invitation;
        Self {
            id: invitation.id,
            invitee: sent.invitee.into(),
            role: invitation.role.into(),
            invited_by: invitation.invited_by,
            created_at: invitation.created_at,
            expires_at: invitation.expires_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SentInvitationList {
    /// Newest first.
    pub invitations: Vec<SentInvitation>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInvitation {
    /// The address of an account on this instance.
    #[schema(example = "bob@example.com")]
    pub email: String,
    /// Admin, member or guest. Owners are made by transferring ownership.
    pub role: WorkspaceRole,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/invitations",
    tag = "workspaces",
    summary = "Invite an account",
    description = "Invites the account of an address, which accepts or declines under \
                   `/account/invitations`; nothing is joined before. It expires after 14 days. \
                   Invitations only go to existing accounts: an address without one answers \
                   `no_account`, and as that tells whether an address has an account, each \
                   inviter's invitations are rate-limited. For owners and admins.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateInvitation,
    responses(
        (status = CREATED, description = "The invitation.", body = SentInvitation),
        (status = BAD_REQUEST, description = "invalid_input: the address is not one.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, as for an owner role, or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace; or no_account: no account here has the address.", body = ErrorBody),
        (status = CONFLICT, description = "already_member, already_invited, or member_limit: the workspace is full.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_invitation(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateInvitation>,
) -> Result<(StatusCode, Json<SentInvitation>), ApiError> {
    let sent = state
        .workspaces
        .invite(&principal, id, &body.email, body.role.into())
        .await?;
    Ok((StatusCode::CREATED, Json(sent.into())))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/invitations",
    tag = "workspaces",
    summary = "List the invitations waiting for an answer",
    description = "For owners and admins.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The pending invitations.", body = SentInvitationList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_invitations(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<SentInvitationList>, ApiError> {
    let sent = state.workspaces.sent_invitations(&principal, id).await?;
    Ok(Json(SentInvitationList {
        invitations: sent.into_iter().map(SentInvitation::from).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/invitations/{invitation_id}/revoke",
    tag = "workspaces",
    summary = "Revoke an invitation",
    description = "It can no longer be accepted. For owners and admins.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("invitation_id" = Uuid, Path, description = "The invitation's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Revoked."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace, or no such pending invitation.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn revoke_invitation(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, invitation_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .revoke_invitation(&principal, id, invitation_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An invite link, without its token.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteLink {
    pub id: Uuid,
    /// What people join as: member or guest.
    pub role: WorkspaceRole,
    /// How many people it admits, and how many joined with it.
    pub max_uses: i32,
    pub uses: i32,
    pub status: InviteStatus,
    pub created_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub revoked_at: Option<OffsetDateTime>,
    /// Whether its token can be copied again.
    pub token_kept: bool,
}

impl InviteLink {
    fn new(link: domain::InviteLink, now: OffsetDateTime) -> Self {
        Self {
            status: link.status(now).into(),
            id: link.id,
            role: link.role.into(),
            max_uses: link.max_uses,
            uses: link.uses,
            created_by: link.created_by,
            created_at: link.created_at,
            expires_at: link.expires_at,
            revoked_at: link.revoked_at,
            token_kept: link.token_kept,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteLinkList {
    /// Newest first.
    pub links: Vec<InviteLink>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInviteLink {
    /// Member or guest.
    pub role: WorkspaceRole,
    /// 1 to 1000; 10 when absent.
    #[serde(default)]
    pub max_uses: Option<i32>,
    /// After now and within a year; a week from now when absent.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

/// A new invite link with its token. The token is shown here, and again only from its
/// sealed copy when a secret key is configured.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreatedInviteLink {
    pub link: InviteLink,
    /// Joined with by `POST /api/v1/workspace-invite-links/join`.
    #[schema(example = "wsl_4yVd0q3oJ2...")]
    pub token: String,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/invite-links",
    tag = "workspaces",
    summary = "Create an invite link",
    description = "Whoever opens it signed in joins with its role, up to its number of uses, \
                   until it expires or is revoked. For owners and admins.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateInviteLink,
    responses(
        (status = CREATED, description = "The link and its token.", body = CreatedInviteLink),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, as for a role other than member or guest, or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "member_limit.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_invite_link(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateInviteLink>,
) -> Result<(StatusCode, Json<CreatedInviteLink>), ApiError> {
    let created = state
        .workspaces
        .create_invite_link(
            &principal,
            id,
            LinkSpec {
                role: body.role.into(),
                max_uses: body.max_uses,
                expires_at: body.expires_at,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedInviteLink {
            token: created.token.expose().to_owned(),
            link: InviteLink::new(created.link, OffsetDateTime::now_utc()),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/invite-links",
    tag = "workspaces",
    summary = "List invite links",
    description = "Without their tokens: copy one with `GET .../invite-links/{link_id}/token`. \
                   For owners and admins.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The links.", body = InviteLinkList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_invite_links(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<InviteLinkList>, ApiError> {
    let now = OffsetDateTime::now_utc();
    let links = state.workspaces.invite_links(&principal, id).await?;
    Ok(Json(InviteLinkList {
        links: links
            .into_iter()
            .map(|link| InviteLink::new(link, now))
            .collect(),
    }))
}

/// An invite link's token, to copy again.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteLinkToken {
    pub token: String,
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/invite-links/{link_id}/token",
    tag = "workspaces",
    summary = "Copy an invite link's token again",
    description = "Opens the token's sealed copy, which is kept while a secret key is \
                   configured. For owners and admins.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("link_id" = Uuid, Path, description = "The link's id."),
    ),
    responses(
        (status = OK, description = "The token.", body = InviteLinkToken),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "token_unavailable: it was not kept, or its key is gone.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn invite_link_token(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<InviteLinkToken>, ApiError> {
    let token = state
        .workspaces
        .invite_link_token(&principal, id, link_id)
        .await?;
    Ok(Json(InviteLinkToken {
        token: token.expose().to_owned(),
    }))
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/invite-links/{link_id}/revoke",
    tag = "workspaces",
    summary = "Revoke an invite link",
    description = "It admits nobody more, and stays listed. For owners and admins.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("link_id" = Uuid, Path, description = "The link's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Revoked."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn revoke_invite_link(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .workspaces
        .revoke_invite_link(&principal, id, link_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What an invite link admits to, for the signed-in account about to use it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteLinkPreview {
    pub workspace: InvitingWorkspace,
    /// What it admits as: member or guest.
    pub role: WorkspaceRole,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// The caller's role in the workspace when they are in it already, so joining would
    /// answer `already_member`.
    pub member_role: Option<WorkspaceRole>,
}

impl From<LinkPreview> for InviteLinkPreview {
    fn from(preview: LinkPreview) -> Self {
        Self {
            workspace: InvitingWorkspace::new(&preview.workspace),
            role: preview.role.into(),
            expires_at: preview.expires_at,
            member_role: preview.member.map(Into::into),
        }
    }
}

#[utoipa::path(
    post,
    path = "/workspace-invite-links/preview",
    tag = "workspaces",
    summary = "Look at an invite link before joining",
    description = "The workspace an invite link admits to and the role it admits with, without \
                   using it, for the page that asks before joining. Needs a signed-in account. \
                   It takes from the allowance that joining takes from.",
    request_body = TokenRequest,
    responses(
        (status = OK, description = "What the link admits to.", body = InviteLinkPreview),
        (status = BAD_REQUEST, description = "invalid_invite_link: unknown, used up, expired or revoked.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "token_bound: a token bound to a workspace; or insufficient_scope.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn preview_invite_link(
    State(state): State<AppState>,
    Account(principal): Account,
    Json(body): Json<TokenRequest>,
) -> Result<Json<InviteLinkPreview>, ApiError> {
    let preview = state
        .workspaces
        .preview_link(&principal, &Secret::new(body.token))
        .await?;
    Ok(Json(preview.into()))
}

#[utoipa::path(
    post,
    path = "/workspace-invite-links/join",
    tag = "workspaces",
    summary = "Join a workspace by invite link",
    description = "Using the link is accepting it: the caller joins with its role, and the \
                   default teamspace unless as a guest. Needs a signed-in account; links never \
                   make one.",
    request_body = TokenRequest,
    responses(
        (status = OK, description = "The workspace joined.", body = WorkspaceSummary),
        (status = BAD_REQUEST, description = "invalid_invite_link: unknown, used up, expired or revoked.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "token_bound: a token bound to a workspace; or insufficient_scope.", body = ErrorBody),
        (status = CONFLICT, description = "already_member, or member_limit.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn join_with_link(
    State(state): State<AppState>,
    Account(principal): Account,
    Json(body): Json<TokenRequest>,
) -> Result<Json<WorkspaceSummary>, ApiError> {
    let joined = state
        .workspaces
        .join_with_link(&principal, &Secret::new(body.token))
        .await?;
    Ok(Json(joined.into()))
}
