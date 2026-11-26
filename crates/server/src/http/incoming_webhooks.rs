//! `/api/v1/workspaces/{id}/incoming-webhooks`: incoming webhooks, each acting as an
//! integration of the workspace on one database or page with what is posted to its URL.
//!
//! The workspace's owners and admins make and manage them; agents and integrations do
//! not. Every route needs a browser session or an API token: `read` for `GET`, `write` for
//! the rest. What a webhook takes, at `POST /hooks/in/{token}`, is in the webhooks guide of
//! the docs, beside the sync's WebSockets outside this description.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::webhooks::{
    IncomingUpdate, IncomingWebhook as AppIncoming, LoggedRequest, NewIncomingWebhook,
    TargetKind as AppTargetKind,
};
use neon_domain::settings::Secret;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::agents::AgentStatus;
use super::dto::Owner;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path};
use super::principal::Authenticated;
use super::web::RequestOrigin;
use super::webhooks::WebhookCreator;

/// What an incoming webhook works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// It makes rows, and changes them by unique id.
    Database,
    /// It adds markdown at the end.
    Page,
}

impl From<AppTargetKind> for TargetKind {
    fn from(kind: AppTargetKind) -> Self {
        match kind {
            AppTargetKind::Database => Self::Database,
            AppTargetKind::Page => Self::Page,
        }
    }
}

/// An incoming webhook's target.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingTarget {
    pub id: Uuid,
    pub kind: TargetKind,
    /// Its title; absent for a caller who may not see it.
    pub title: Option<String>,
}

/// The integration an incoming webhook acts as.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingIntegration {
    pub id: Uuid,
    /// Suspended while its owner is out of the workspace, a guest there or disabled: its
    /// webhook refuses what is sent meanwhile.
    pub status: AgentStatus,
    /// The person it acts for; absent once their account is gone.
    pub owner: Option<Owner>,
}

/// An incoming webhook, without its URL's token or its secret.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingWebhook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Its integration's name, which shows as `name · via Owner` where it writes.
    #[schema(example = "CI")]
    pub name: String,
    pub integration: IncomingIntegration,
    pub target: IncomingTarget,
    /// Whether it takes only requests signed with its secret.
    pub signed: bool,
    /// Until when the secret before the last rotation still verifies.
    #[serde(with = "time::serde::rfc3339::option")]
    pub previous_secret_expires_at: Option<OffsetDateTime>,
    /// False while it is turned off and refuses what is sent.
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub disabled_at: Option<OffsetDateTime>,
    pub created_by: Option<WebhookCreator>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<AppIncoming> for IncomingWebhook {
    fn from(hook: AppIncoming) -> Self {
        Self {
            enabled: hook.disabled_at.is_none(),
            id: hook.id,
            workspace_id: hook.workspace_id,
            name: hook.name,
            integration: IncomingIntegration {
                id: hook.integration.id,
                status: hook.integration.status.into(),
                owner: hook.integration.owner.map(Owner::from),
            },
            target: IncomingTarget {
                id: hook.target.id,
                kind: hook.target.kind.into(),
                title: hook.target.title,
            },
            signed: hook.signed,
            previous_secret_expires_at: hook.previous_secret_expires_at,
            disabled_at: hook.disabled_at,
            created_by: hook.created_by.map(WebhookCreator::from),
            created_at: hook.created_at,
            updated_at: hook.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingWebhookList {
    /// Oldest first.
    pub hooks: Vec<IncomingWebhook>,
}

/// A new incoming webhook with its URL and, when it checks signatures, its secret: shown
/// this once.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreatedIncomingWebhook {
    pub hook: IncomingWebhook,
    /// Where senders post: `/hooks/in/` and its token, after the server's public URL, or
    /// without one, the origin the request was sent to. The token is in this answer only.
    #[schema(example = "https://neon.example.com/hooks/in/nwh_...")]
    pub url: String,
    /// `whsec_` and 43 characters; absent unless it checks signatures.
    pub signing_secret: Option<String>,
}

/// An incoming webhook with its new URL, shown this once.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingWebhookUrl {
    pub hook: IncomingWebhook,
    #[schema(example = "https://neon.example.com/hooks/in/nwh_...")]
    pub url: String,
}

/// An incoming webhook with its new signing secret, shown this once.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingWebhookSecret {
    pub hook: IncomingWebhook,
    #[schema(example = "whsec_...")]
    pub signing_secret: String,
}

/// A request an incoming webhook took, without what it sent.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingRequest {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    /// The status it was answered with.
    pub status_code: i32,
    /// `ok`, or the error's code, such as `invalid_signature`.
    #[schema(example = "ok")]
    pub outcome: String,
    /// `create_row`, `update_row` or `append`, once its body read.
    pub action: Option<String>,
    /// The sender's address, as the server tells clients apart.
    #[schema(example = "203.0.113.7")]
    pub client_ip: String,
    pub latency_ms: i32,
}

impl From<LoggedRequest> for IncomingRequest {
    fn from(request: LoggedRequest) -> Self {
        Self {
            id: request.id,
            at: request.at,
            status_code: request.status_code,
            outcome: request.outcome,
            action: request.action,
            client_ip: request.client_ip,
            latency_ms: request.latency_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IncomingRequestLog {
    /// The last 50 but those refused for their rate, newest first.
    pub requests: Vec<IncomingRequest>,
}

/// The URL senders post to, with `token`, below the server's origin as `origin` has it.
fn url(RequestOrigin(origin): &RequestOrigin, token: &Secret) -> String {
    format!("{origin}/hooks/in/{}", token.expose())
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/incoming-webhooks",
    tag = "webhooks",
    summary = "List incoming webhooks",
    description = "The workspace's incoming webhooks, oldest first, for its owners and admins.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The webhooks.", body = IncomingWebhookList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not an owner or admin; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_incoming_webhooks(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<IncomingWebhookList>, ApiError> {
    let hooks = state.webhooks.list_incoming(&principal, id).await?;
    Ok(Json(IncomingWebhookList {
        hooks: hooks.into_iter().map(IncomingWebhook::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateIncomingWebhook {
    /// Its integration's name, 1 to 100 characters.
    #[schema(example = "CI")]
    pub name: String,
    /// A database, whose rows it makes and changes, or a page, which it adds to: one the
    /// caller may share.
    pub target_id: Uuid,
    /// Take only requests signed with a secret made now. False unless set.
    #[serde(default)]
    pub signed: bool,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/incoming-webhooks",
    tag = "webhooks",
    summary = "Make an incoming webhook",
    description = "For the workspace's owners and admins, on a database or page they may share. \
                   It makes an integration the caller owns, within the workspace's agent \
                   limit, which joins no teamspace and gets the target shared with it to \
                   edit: what it may change is its target, and no more than its owner may. Its \
                   URL, and its secret when it checks signatures, are in this answer only.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateIncomingWebhook,
    responses(
        (status = CREATED, description = "The webhook, with its URL.", body = CreatedIncomingWebhook),
        (status = BAD_REQUEST, description = "invalid_input: a name out of bounds, or a row as the target.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not an owner or admin, or the target is not theirs to share; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace or page.", body = ErrorBody),
        (status = CONFLICT, description = "agent_limit; or no_secret_key for a signed one without secrets.keys.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_incoming_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    origin: RequestOrigin,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateIncomingWebhook>,
) -> Result<(StatusCode, Json<CreatedIncomingWebhook>), ApiError> {
    let created = state
        .webhooks
        .create_incoming(
            &principal,
            id,
            NewIncomingWebhook {
                name: body.name,
                target_id: body.target_id,
                signed: body.signed,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedIncomingWebhook {
            url: url(&origin, &created.token),
            signing_secret: created
                .signing_secret
                .map(|secret| secret.expose().to_owned()),
            hook: created.hook.into(),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}",
    tag = "webhooks",
    summary = "An incoming webhook",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The webhook.", body = IncomingWebhook),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_incoming_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<IncomingWebhook>, ApiError> {
    Ok(Json(
        state
            .webhooks
            .incoming(&principal, id, hook_id)
            .await?
            .into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateIncomingWebhook {
    /// Renames its integration too.
    #[serde(default)]
    #[schema(example = "CI")]
    pub name: Option<String>,
    /// False turns it off: it refuses what is sent until it is on again.
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}",
    tag = "webhooks",
    summary = "Rename an incoming webhook, or turn it on or off",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    request_body = UpdateIncomingWebhook,
    responses(
        (status = OK, description = "The webhook.", body = IncomingWebhook),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_incoming_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateIncomingWebhook>,
) -> Result<Json<IncomingWebhook>, ApiError> {
    let update = IncomingUpdate {
        name: body.name,
        enabled: body.enabled,
    };
    Ok(Json(
        state
            .webhooks
            .update_incoming(&principal, id, hook_id, update)
            .await?
            .into(),
    ))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}",
    tag = "webhooks",
    summary = "Delete an incoming webhook",
    description = "With its integration, which leaves the workspace and its target's share; \
                   what it made still names it. Its URL stops working.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
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
pub(super) async fn delete_incoming_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .webhooks
        .delete_incoming(&principal, id, hook_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}/token",
    tag = "webhooks",
    summary = "Give an incoming webhook a new URL",
    description = "The new URL is in this answer only; the old one stops working at once.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The webhook with its new URL.", body = IncomingWebhookUrl),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn rotate_incoming_token(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    origin: RequestOrigin,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<IncomingWebhookUrl>, ApiError> {
    let (token, hook) = state
        .webhooks
        .rotate_incoming_token(&principal, id, hook_id)
        .await?;
    Ok(Json(IncomingWebhookUrl {
        url: url(&origin, &token),
        hook: hook.into(),
    }))
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}/secret",
    tag = "webhooks",
    summary = "Make or rotate an incoming webhook's signing secret",
    description = "A new secret, in this answer only, which requests must be signed with from \
                   now. One rotated still verifies for 24 hours, so a sender can move over.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The webhook with its new secret.", body = IncomingWebhookSecret),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "no_secret_key: secrets.keys is not set.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn rotate_incoming_secret(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<IncomingWebhookSecret>, ApiError> {
    let (secret, hook) = state
        .webhooks
        .rotate_incoming_secret(&principal, id, hook_id)
        .await?;
    Ok(Json(IncomingWebhookSecret {
        signing_secret: secret.expose().to_owned(),
        hook: hook.into(),
    }))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}/secret",
    tag = "webhooks",
    summary = "Stop checking an incoming webhook's signatures",
    description = "It takes unsigned requests again; its secrets are forgotten.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The webhook.", body = IncomingWebhook),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn remove_incoming_secret(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<IncomingWebhook>, ApiError> {
    Ok(Json(
        state
            .webhooks
            .remove_incoming_secret(&principal, id, hook_id)
            .await?
            .into(),
    ))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/incoming-webhooks/{hook_id}/requests",
    tag = "webhooks",
    summary = "An incoming webhook's requests",
    description = "Its last 50 requests but those refused for their rate, newest first: when, \
                   from where, what they asked and how they were answered, never their body.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("hook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The log.", body = IncomingRequestLog),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn incoming_webhook_requests(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, hook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<IncomingRequestLog>, ApiError> {
    let requests = state
        .webhooks
        .incoming_requests(&principal, id, hook_id)
        .await?;
    Ok(Json(IncomingRequestLog {
        requests: requests.into_iter().map(IncomingRequest::from).collect(),
    }))
}
