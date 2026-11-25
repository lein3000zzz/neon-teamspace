//! `/api/v1/workspaces/{id}/agents`: a workspace's agents and integrations, and agents'
//! API tokens.
//!
//! An agent is a member of its workspace that acts for a person there, its owner: what it
//! may do is what both may. Its owner makes it, renames and recolours it, and makes its
//! tokens; the workspace's owners and admins hand it to someone else, and they or its owner
//! delete it. Every route needs a browser session or an API token: `read` for `GET`, `write`
//! for the rest; making and revoking tokens need a browser session.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::workspaces::{AgentUpdate, NewAgent};
use neon_domain::agents::{self as domain, AgentStatus as DomainStatus};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::account::{ApiToken, ApiTokenList, CreatedApiToken};
use super::dto::{Owner, Scope, UserKind, nullable};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path};
use super::principal::Authenticated;

/// Whether an agent or integration works now, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Its tokens work.
    Active,
    /// Its owner is out of the workspace or disabled, or their account is gone: its tokens
    /// are refused until that ends, or it is handed to someone else.
    Suspended,
    /// Agents are off, on the instance or in the workspace: its tokens are refused.
    Disabled,
    /// Deleted: what it made still names it.
    Deleted,
}

impl From<DomainStatus> for AgentStatus {
    fn from(status: DomainStatus) -> Self {
        match status {
            DomainStatus::Active => Self::Active,
            DomainStatus::Suspended => Self::Suspended,
            DomainStatus::Disabled => Self::Disabled,
            DomainStatus::Deleted => Self::Deleted,
        }
    }
}

/// An agent or integration of a workspace.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Agent {
    pub id: Uuid,
    /// `agent` or `integration`.
    pub kind: UserKind,
    pub workspace_id: Uuid,
    #[schema(example = "Scout")]
    pub name: String,
    /// The hue it is drawn in, 0 to 359: the one pinned, or else its id's.
    #[schema(minimum = 0, maximum = 359, example = 160)]
    pub hue: u16,
    #[schema(minimum = 0, maximum = 359)]
    pub hue_override: Option<u16>,
    /// The person it acts for; absent once their account is gone.
    pub owner: Option<Owner>,
    pub status: AgentStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub deleted_at: Option<OffsetDateTime>,
}

impl From<domain::Agent> for Agent {
    fn from(agent: domain::Agent) -> Self {
        Self {
            hue: agent.hue(),
            id: agent.id,
            kind: agent.kind.into(),
            workspace_id: agent.workspace_id,
            name: agent.name,
            hue_override: agent.hue_override,
            owner: agent.owner.map(Owner::from),
            status: agent.status.into(),
            created_at: agent.created_at,
            updated_at: agent.updated_at,
            deleted_at: agent.deleted_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentList {
    /// Those not deleted first, each group by name.
    pub agents: Vec<Agent>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/agents",
    tag = "agents",
    summary = "List a workspace's agents",
    description = "Its agents and integrations, deleted ones too, with their owners and \
                   whether they work. For everyone in the workspace but guests.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The agents.", body = AgentList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_agents(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentList>, ApiError> {
    let agents = state.agents.list_agents(&principal, id).await?;
    Ok(Json(AgentList {
        agents: agents.into_iter().map(Agent::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgent {
    /// 1 to 100 characters.
    #[schema(example = "Scout")]
    pub name: String,
    /// The hue to pin, 0 to 359; absent, its id's.
    #[serde(default)]
    #[schema(minimum = 0, maximum = 359)]
    pub hue_override: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/agents",
    tag = "agents",
    summary = "Make an agent",
    description = "Makes an agent that the caller owns, a member of the workspace and of its \
                   default teamspace. While agents are on, on the instance and in the \
                   workspace, for whom the workspace lets make them, never a guest or an \
                   agent, and within its agent limit.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateAgent,
    responses(
        (status = CREATED, description = "The agent.", body = Agent),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: agents are off, or the caller may not make them; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "agent_limit: the workspace has as many agents and integrations as its limit allows.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_agent(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateAgent>,
) -> Result<(StatusCode, Json<Agent>), ApiError> {
    let new = NewAgent {
        name: body.name,
        hue: body.hue_override,
    };
    let agent = state.agents.create_agent(&principal, id, new).await?;
    Ok((StatusCode::CREATED, Json(agent.into())))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/agents/{agent_id}",
    tag = "agents",
    summary = "An agent",
    description = "An agent or integration of the workspace, deleted or not. For everyone in \
                   the workspace but guests.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
    ),
    responses(
        (status = OK, description = "The agent.", body = Agent),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_agent(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Agent>, ApiError> {
    Ok(Json(
        state.agents.agent(&principal, id, agent_id).await?.into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgent {
    /// 1 to 100 characters.
    #[serde(default)]
    #[schema(example = "Scout")]
    pub name: Option<String>,
    /// 0 to 359; null goes back to the id's hue.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0, maximum = 359)]
    pub hue_override: Option<Option<i64>>,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}/agents/{agent_id}",
    tag = "agents",
    summary = "Rename or recolour an agent",
    description = "Changes what the body names, for the agent's owner.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
    ),
    request_body = UpdateAgent,
    responses(
        (status = OK, description = "The agent.", body = Agent),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not its owner, or it is deleted; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_agent(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateAgent>,
) -> Result<Json<Agent>, ApiError> {
    let update = AgentUpdate {
        name: body.name,
        hue: body.hue_override,
    };
    Ok(Json(
        state
            .agents
            .update_agent(&principal, id, agent_id, update)
            .await?
            .into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferAgent {
    /// The person of the workspace, not a guest, who owns it from now on.
    pub owner_id: Uuid,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/agents/{agent_id}/transfer",
    tag = "agents",
    summary = "Hand an agent to someone else",
    description = "Makes another person of the workspace, not a guest, its owner, for the \
                   workspace's owners and admins. It acts for them from then on, which \
                   resumes it if it was suspended; its tokens, which its former owner made, \
                   are revoked.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
    ),
    request_body = TransferAgent,
    responses(
        (status = OK, description = "The agent.", body = Agent),
        (status = BAD_REQUEST, description = "invalid_input: not a person, or a guest.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not an owner or admin, or it is deleted; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such agent or member.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn transfer_agent(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<TransferAgent>,
) -> Result<Json<Agent>, ApiError> {
    Ok(Json(
        state
            .agents
            .transfer_agent(&principal, id, agent_id, body.owner_id)
            .await?
            .into(),
    ))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/agents/{agent_id}",
    tag = "agents",
    summary = "Delete an agent",
    description = "For its owner and the workspace's owners and admins. It leaves the \
                   workspace, its tokens are revoked and its open connections close; what it \
                   made still names it. Deleting one deleted already does nothing.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
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
pub(super) async fn delete_agent(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.agents.delete_agent(&principal, id, agent_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/agents/{agent_id}/tokens",
    tag = "agents",
    summary = "List an agent's tokens",
    description = "Without their secrets. For its owner and the workspace's owners and admins.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
    ),
    responses(
        (status = OK, description = "The tokens.", body = ApiTokenList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_agent_tokens(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ApiTokenList>, ApiError> {
    let tokens = state.agents.agent_tokens(&principal, id, agent_id).await?;
    Ok(Json(ApiTokenList {
        tokens: tokens.into_iter().map(ApiToken::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentToken {
    /// 1 to 64 characters.
    #[schema(example = "Hermes on the build box")]
    pub name: String,
    /// `read`, `write` or both; never `admin`.
    pub scopes: Vec<Scope>,
    /// After now; absent for a token that never expires.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/agents/{agent_id}/tokens",
    tag = "agents",
    summary = "Make an agent's token",
    description = "A token that acts as the agent, for its owner, bound to the workspace: \
                   whatever it does, the agent may do and its owner may too. Its secret is \
                   in this answer only; the server keeps its hash. For the agent's owner, \
                   from a browser session, while the agent works.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
    ),
    request_body = CreateAgentToken,
    responses(
        (status = CREATED, description = "The token.", body = CreatedApiToken),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not its owner, not a browser session, the admin scope, or the agent does not work now; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn create_agent_token(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<CreateAgentToken>,
) -> Result<(StatusCode, Json<CreatedApiToken>), ApiError> {
    let scopes: Vec<&str> = body.scopes.iter().map(|scope| scope.as_str()).collect();
    let (secret, token) = state
        .agents
        .create_agent_token(
            &principal,
            id,
            agent_id,
            &body.name,
            body.expires_at,
            &scopes,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedApiToken {
            token: secret.expose().to_owned(),
            api_token: token.into(),
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/agents/{agent_id}/tokens/{token_id}",
    tag = "agents",
    summary = "Revoke an agent's token",
    description = "It stops working at once, and the connections it opened close. For the \
                   agent's owner and the workspace's owners and admins, from a browser \
                   session.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("agent_id" = Uuid, Path, description = "The agent's id."),
        ("token_id" = Uuid, Path, description = "The token's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Revoked."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not its owner, an owner or an admin, or not a browser session.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn revoke_agent_token(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, agent_id, token_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .agents
        .revoke_agent_token(&principal, id, agent_id, token_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
