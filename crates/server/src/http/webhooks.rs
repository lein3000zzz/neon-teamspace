//! `/api/v1/workspaces/{id}/webhooks`: outgoing webhooks, which tell a URL of what happens
//! to the rows an identity is assigned to, signed with a secret of their own.
//!
//! The workspace's owners and admins register and manage anyone's; everyone else but
//! guests their own and their agents' and integrations'. Agents and integrations manage
//! none. Every route needs a browser session or an API token: `read` for `GET`, `write` for
//! the rest. The webhooks guide in the docs describes what they send and how to check it.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::webhooks::{
    Creator, DeliveryAttempt as AppAttempt, NewWebhook, OutgoingWebhook, WebhookUpdate,
};
use neon_domain::webhooks::EventType;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::dto::Person;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::principal::Authenticated;

/// What an outgoing webhook may be told of. Receivers ignore types they do not know: more
/// come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum WebhookEvent {
    /// The identity was added to a person property of a row.
    #[serde(rename = "row.assigned")]
    RowAssigned,
    /// The status of a row the identity is in a person property of changed.
    #[serde(rename = "row.status_changed")]
    RowStatusChanged,
    /// A page's text, or a comment on it, mentions the identity.
    #[serde(rename = "page.mentioned")]
    PageMentioned,
    /// Someone replied in a thread of comments the identity wrote in.
    #[serde(rename = "comment.replied")]
    CommentReplied,
}

impl WebhookEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::RowAssigned => EventType::Assigned.as_str(),
            Self::RowStatusChanged => EventType::StatusChanged.as_str(),
            Self::PageMentioned => EventType::Mentioned.as_str(),
            Self::CommentReplied => EventType::Replied.as_str(),
        }
    }
}

impl From<EventType> for WebhookEvent {
    fn from(event: EventType) -> Self {
        match event {
            EventType::Assigned => Self::RowAssigned,
            EventType::StatusChanged => Self::RowStatusChanged,
            EventType::Mentioned => Self::PageMentioned,
            EventType::Replied => Self::CommentReplied,
        }
    }
}

/// Who made a webhook.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookCreator {
    pub id: Uuid,
    #[schema(example = "Ada Lovelace")]
    pub display_name: String,
}

impl From<Creator> for WebhookCreator {
    fn from(creator: Creator) -> Self {
        Self {
            id: creator.id,
            display_name: creator.display_name,
        }
    }
}

/// An outgoing webhook, without its secret.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Webhook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The identity whose events it delivers, without their address.
    pub identity: Person,
    #[schema(example = "https://hooks.example.com/neon")]
    pub url: String,
    pub events: Vec<WebhookEvent>,
    /// Whether it is told of the identity's own changes too.
    pub include_own: bool,
    /// False while it is off: turned off by hand, or after deliveries kept failing.
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub disabled_at: Option<OffsetDateTime>,
    /// Why it is off.
    #[schema(
        example = "turned off after 5 deliveries in a row failed; the last: the receiver answered 500"
    )]
    pub disabled_reason: Option<String>,
    /// Deliveries given up on in a row since the last one that went through.
    pub failures: i32,
    /// Until when the secret before the last rotation signs deliveries too.
    #[serde(with = "time::serde::rfc3339::option")]
    pub previous_secret_expires_at: Option<OffsetDateTime>,
    /// Absent once their account is gone.
    pub created_by: Option<WebhookCreator>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<OutgoingWebhook> for Webhook {
    fn from(webhook: OutgoingWebhook) -> Self {
        Self {
            enabled: webhook.disabled_at.is_none(),
            id: webhook.id,
            workspace_id: webhook.workspace_id,
            identity: webhook.identity.into(),
            url: webhook.url,
            events: webhook.events.into_iter().map(WebhookEvent::from).collect(),
            include_own: webhook.include_own,
            disabled_at: webhook.disabled_at,
            disabled_reason: webhook.disabled_reason,
            failures: webhook.failures,
            previous_secret_expires_at: webhook.previous_secret_expires_at,
            created_by: webhook.created_by.map(WebhookCreator::from),
            created_at: webhook.created_at,
            updated_at: webhook.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookList {
    /// Oldest first.
    pub webhooks: Vec<Webhook>,
}

/// A webhook with its signing secret, shown this once.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WebhookWithSecret {
    pub webhook: Webhook,
    /// `whsec_` and 43 characters. The server keeps it encrypted and never shows it again.
    #[schema(example = "whsec_...")]
    pub secret: String,
}

/// How one attempt at a delivery went.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeliveryAttempt {
    pub id: Uuid,
    /// The delivery, the same over its attempts; absent for a test event.
    pub delivery_id: Option<Uuid>,
    /// The event's id, as its payload says.
    pub event_id: Uuid,
    #[schema(example = "row.assigned")]
    pub event_type: String,
    /// 1 for the first try.
    pub attempt: i32,
    pub outcome: AttemptOutcome,
    /// The receiver's HTTP status, when it answered.
    pub status_code: Option<i32>,
    pub latency_ms: Option<i32>,
    /// Why it failed or was skipped: `unresolvable`, `address_refused`, `connect`, `tls`,
    /// `timeout`, `protocol`, `redirect`, `http_status`, or `internal`, the server failing to
    /// make it; for a skipped one `no_access`, the identity or whoever set the URL no longer
    /// seeing the row, `lapsed`, whoever set the URL no longer allowed to register it, or
    /// `disabled`.
    #[schema(example = "http_status")]
    pub error_kind: Option<String>,
    /// The start of the answer's body, at most 256 bytes.
    pub excerpt: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

/// How an attempt went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    /// The receiver answered 2xx.
    Delivered,
    /// It failed, and is tried again later.
    Retrying,
    /// It failed, and was given up on.
    Failed,
    /// It was not sent: the identity no longer sees the row, or the webhook is off.
    Skipped,
}

impl From<AppAttempt> for DeliveryAttempt {
    fn from(attempt: AppAttempt) -> Self {
        Self {
            outcome: match attempt.outcome.as_str() {
                "delivered" => AttemptOutcome::Delivered,
                "retrying" => AttemptOutcome::Retrying,
                "skipped" => AttemptOutcome::Skipped,
                _ => AttemptOutcome::Failed,
            },
            id: attempt.id,
            delivery_id: attempt.delivery_id,
            event_id: attempt.event_id,
            event_type: attempt.event_type,
            attempt: attempt.attempt,
            status_code: attempt.status_code,
            latency_ms: attempt.latency_ms,
            error_kind: attempt.error_kind,
            excerpt: attempt.excerpt,
            at: attempt.at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeliveryLog {
    /// The last 50, newest first.
    pub attempts: Vec<DeliveryAttempt>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WebhookFilter {
    /// Only the webhooks of this identity.
    pub user_id: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/webhooks",
    tag = "webhooks",
    summary = "List outgoing webhooks",
    description = "The workspace's outgoing webhooks the caller may manage, oldest first: \
                   every one for its owners and admins, for everyone else their own and their \
                   agents' and integrations'.",
    params(("id" = Uuid, Path, description = "The workspace's id."), WebhookFilter),
    responses(
        (status = OK, description = "The webhooks.", body = WebhookList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_webhooks(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(filter): Query<WebhookFilter>,
) -> Result<Json<WebhookList>, ApiError> {
    let webhooks = state
        .webhooks
        .list_webhooks(&principal, id, filter.user_id)
        .await?;
    Ok(Json(WebhookList {
        webhooks: webhooks.into_iter().map(Webhook::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWebhook {
    /// The member whose events it delivers: a person, an agent or an integration.
    pub user_id: Uuid,
    /// `http` or `https`, at most 2048 bytes, without a user name or password. Its host must
    /// resolve to addresses webhooks may reach.
    #[schema(example = "https://hooks.example.com/neon")]
    pub url: String,
    /// At least one.
    pub events: Vec<WebhookEvent>,
    /// Told of the identity's own changes too. False unless set.
    #[serde(default)]
    pub include_own: bool,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/webhooks",
    tag = "webhooks",
    summary = "Register an outgoing webhook",
    description = "Registers a webhook that delivers an identity's events: for the \
                   workspace's owners and admins, anyone in it; for everyone else but \
                   guests, themselves and their own agents and integrations. Its signing \
                   secret is in this answer only. Needs secrets.keys on the server.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateWebhook,
    responses(
        (status = CREATED, description = "The webhook, with its secret.", body = WebhookWithSecret),
        (status = BAD_REQUEST, description = "invalid_input, invalid_url, or address_refused: the URL's host does not resolve, or resolves to an address webhooks may not reach.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not for this identity; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace or member.", body = ErrorBody),
        (status = CONFLICT, description = "no_secret_key: secrets.keys is not set; or webhook_limit: the identity has 10 webhooks, or the workspace 200.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateWebhook>,
) -> Result<(StatusCode, Json<WebhookWithSecret>), ApiError> {
    let new = NewWebhook {
        user_id: body.user_id,
        url: body.url,
        events: body
            .events
            .iter()
            .map(|event| event.as_str().to_owned())
            .collect(),
        include_own: body.include_own,
    };
    let (secret, webhook) = state.webhooks.create_webhook(&principal, id, new).await?;
    Ok((
        StatusCode::CREATED,
        Json(WebhookWithSecret {
            webhook: webhook.into(),
            secret: secret.expose().to_owned(),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/webhooks/{webhook_id}",
    tag = "webhooks",
    summary = "An outgoing webhook",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("webhook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The webhook.", body = Webhook),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Webhook>, ApiError> {
    Ok(Json(
        state
            .webhooks
            .webhook(&principal, id, webhook_id)
            .await?
            .into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWebhook {
    #[serde(default)]
    #[schema(example = "https://hooks.example.com/neon")]
    pub url: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<WebhookEvent>>,
    #[serde(default)]
    pub include_own: Option<bool>,
    /// True turns it on and clears its failures; false turns it off.
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[utoipa::path(
    patch,
    path = "/workspaces/{id}/webhooks/{webhook_id}",
    tag = "webhooks",
    summary = "Change an outgoing webhook",
    description = "Changes what the body names: its URL, checked as when it was registered, \
                   its events, whether it hears the identity's own changes, and whether it is \
                   on. Turning it on clears its failures and why it was off. Setting its URL \
                   binds it to the caller: it is told only of rows both they and its identity \
                   may see, while they may register webhooks for its identity.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("webhook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    request_body = UpdateWebhook,
    responses(
        (status = OK, description = "The webhook.", body = Webhook),
        (status = BAD_REQUEST, description = "invalid_input, invalid_url or address_refused.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: may not set its URL for this identity; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "url_setter_lapsed: turning it on, but whoever set its URL may no longer register it; set its URL again.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, webhook_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateWebhook>,
) -> Result<Json<Webhook>, ApiError> {
    let update = WebhookUpdate {
        url: body.url,
        events: body.events.map(|events| {
            events
                .iter()
                .map(|event| event.as_str().to_owned())
                .collect()
        }),
        include_own: body.include_own,
        enabled: body.enabled,
    };
    Ok(Json(
        state
            .webhooks
            .update_webhook(&principal, id, webhook_id, update)
            .await?
            .into(),
    ))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/webhooks/{webhook_id}",
    tag = "webhooks",
    summary = "Delete an outgoing webhook",
    description = "With its log. Deliveries still queued for it are dropped.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("webhook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .webhooks
        .delete_webhook(&principal, id, webhook_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/webhooks/{webhook_id}/secret",
    tag = "webhooks",
    summary = "Rotate a webhook's secret",
    description = "Makes a new signing secret, in this answer only. For 24 hours deliveries \
                   are signed with the one before too, a second `v1=` in \
                   `Neon-Webhook-Signature`, so a receiver can move to the new one; one \
                   rotated earlier stops at once.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("webhook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The webhook, with its new secret.", body = WebhookWithSecret),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "no_secret_key: secrets.keys is not set.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn rotate_webhook_secret(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<WebhookWithSecret>, ApiError> {
    let (secret, webhook) = state
        .webhooks
        .rotate_webhook_secret(&principal, id, webhook_id)
        .await?;
    Ok(Json(WebhookWithSecret {
        webhook: webhook.into(),
        secret: secret.expose().to_owned(),
    }))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/webhooks/{webhook_id}/deliveries",
    tag = "webhooks",
    summary = "A webhook's delivery log",
    description = "Its last 50 attempts, newest first: how each went, the receiver's status, \
                   how long it took and the start of its answer, never what was sent.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("webhook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The log.", body = DeliveryLog),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn webhook_deliveries(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<DeliveryLog>, ApiError> {
    let attempts = state
        .webhooks
        .webhook_attempts(&principal, id, webhook_id)
        .await?;
    Ok(Json(DeliveryLog {
        attempts: attempts.into_iter().map(DeliveryAttempt::from).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/webhooks/{webhook_id}/test",
    tag = "webhooks",
    summary = "Send a test event",
    description = "Sends a `webhook.test` event now, signed as deliveries are, whether the \
                   webhook is on or off, and answers how it went. It is not retried and counts \
                   for nothing toward turning the webhook off; the log keeps it.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("webhook_id" = Uuid, Path, description = "The webhook's id."),
    ),
    responses(
        (status = OK, description = "The attempt.", body = DeliveryAttempt),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited: too many test events from this person; see Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn test_webhook(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<DeliveryAttempt>, ApiError> {
    Ok(Json(
        state
            .webhooks
            .test_webhook(&principal, id, webhook_id)
            .await?
            .into(),
    ))
}
