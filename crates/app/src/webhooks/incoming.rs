//! Incoming webhooks: making them, each with the integration it acts as; renaming them and
//! turning them off; their URL's token and signing secret; deleting them; their logs; and
//! the requests they take.
//!
//! - **Who.** The workspace's owners and admins make and manage them, as the rules say.
//!   Making one makes an integration the maker owns, which counts against the workspace's
//!   agent limit and shows as `name · via Owner` where it writes.
//! - **Least access.** Its integration joins no teamspace. The webhook shares its target
//!   with it to edit, which its maker must be allowed to do, so what it may change is its
//!   target, and for a database its rows, and no more than its owner may: the rules ask
//!   both, as for every agent. Its requests reach nothing else, as each action names its
//!   target alone.
//! - **The URL** is `/hooks/in/` and a token of 256 random bits, shown once; only its
//!   SHA-256 is kept. A new one replaces it at once. A webhook may also check signatures,
//!   as [`signature`](super::signature) describes, with a secret it makes, shows once and
//!   keeps sealed; a rotated secret still verifies for a day.
//! - **Requests** are limited per client address and per webhook, read up to
//!   [`MAX_INCOMING_BODY`](super::MAX_INCOMING_BODY), and refused while the webhook is off or its integration does not
//!   work. What they ask, in the format [`payload`](super::payload) describes, goes through
//!   the database and sync services as the integration. Each answer but a rate limit's is
//!   logged: its status, action, client address and time, never the body.

use std::future::Future;
use std::time::Instant;

use bytes::Bytes;
use neon_domain::agents::{AgentOwner, AgentStatus};
use neon_domain::auth::{Credential, Principal, Scope, UserKind};
use neon_domain::pages::{PageAccess, PageKind};
use neon_domain::settings::Secret;
use neon_domain::webhooks::check_name;
use neon_domain::workspaces::{Action, Resource};
use neon_store::incoming_webhooks::{self as repo, IncomingWebhook as Stored, Request};
use neon_store::{agents, users};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use super::error::{IncomingError, WebhookError};
use super::outgoing::Creator;
use super::payload::{self, Order, TargetKind};
use super::signature::{self, SECRET_PREFIX};
use super::{NO_WORKSPACE, ROTATION_OVERLAP, WebhookService};
use crate::crypto::{Envelope, hash_secret, new_secret};
use crate::databases::NewRow;
use crate::pages::Facts;
use crate::ratelimit::Bucket;
use crate::standing::Standing;
use crate::workspaces::WorkspaceError;

/// Starts every incoming webhook's URL token.
pub const TOKEN_PREFIX: &str = "nwh_";

const NO_HOOK: &str = "no such webhook";
const NO_PAGE: &str = "no such page";

/// The longest token looked up: a prefix and 43 characters, and some room.
const MAX_TOKEN: usize = 128;

/// An incoming webhook's integration, and whether it works now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integration {
    pub id: Uuid,
    pub status: AgentStatus,
    /// Unset once the owner's account is gone.
    pub owner: Option<AgentOwner>,
}

/// What an incoming webhook works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: Uuid,
    pub kind: TargetKind,
    /// Its title, for those who may see it.
    pub title: Option<String>,
}

/// An incoming webhook as its managers see it, without its token or secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingWebhook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Its integration's name.
    pub name: String,
    pub integration: Integration,
    pub target: Target,
    /// Whether it takes only signed requests.
    pub signed: bool,
    /// Until when the secret before the last rotation still verifies.
    pub previous_secret_expires_at: Option<OffsetDateTime>,
    pub disabled_at: Option<OffsetDateTime>,
    pub created_by: Option<Creator>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// An incoming webhook to make.
#[derive(Debug, Clone)]
pub struct NewIncomingWebhook {
    /// Its integration's name.
    pub name: String,
    /// A database or a page of the workspace, which the maker may share.
    pub target_id: Uuid,
    /// Whether it takes only signed requests, with a secret made now.
    pub signed: bool,
}

/// What a change to an incoming webhook sets; each part left out stays.
#[derive(Debug, Clone, Default)]
pub struct IncomingUpdate {
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

/// A new incoming webhook, with its URL's token and, when it checks signatures, its
/// secret: shown this once.
#[derive(Debug)]
pub struct CreatedIncoming {
    pub hook: IncomingWebhook,
    pub token: Secret,
    pub signing_secret: Option<Secret>,
}

/// A request an incoming webhook took, as its log keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedRequest {
    pub id: Uuid,
    pub at: OffsetDateTime,
    pub status_code: i32,
    /// `ok`, or the code of the error it was answered with.
    pub outcome: String,
    pub action: Option<String>,
    pub client_ip: String,
    pub latency_ms: i32,
}

/// A request to an incoming webhook's URL, as the server got it, but for its body.
#[derive(Debug, Clone, Copy)]
pub struct IncomingRequest<'a> {
    /// The token in its URL.
    pub token: &'a str,
    pub client_ip: &'a str,
    /// Whether its `Content-Type` says JSON.
    pub json: bool,
    /// Its `Neon-Webhook-Signature` and `Neon-Webhook-Timestamp`, if it sent them.
    pub signature: Option<&'a str>,
    pub timestamp: Option<&'a str>,
}

/// Why a request's body was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    /// It is larger than [`MAX_INCOMING_BODY`](super::MAX_INCOMING_BODY).
    TooLarge,
    /// The connection failed before it ended.
    Unreadable,
}

/// What a request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    /// `create_row`, `update_row` or `append`.
    pub action: &'static str,
    /// The row made or changed: its id and unique id.
    pub row: Option<(Uuid, String)>,
}

impl WebhookService {
    /// The workspace's incoming webhooks, oldest first, for its owners and admins.
    pub async fn list_incoming(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<IncomingWebhook>, WebhookError> {
        self.observe("list_incoming", async {
            let mut conn = self.pool().acquire().await?;
            self.managing(&mut conn, principal, workspace_id).await?;
            let mut hooks = Vec::new();
            for stored in repo::list(&mut *conn, workspace_id).await? {
                hooks.push(self.incoming_view(&mut conn, principal, stored).await?);
            }
            Ok(hooks)
        })
        .await
    }

    /// One incoming webhook, for the workspace's owners and admins.
    pub async fn incoming(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<IncomingWebhook, WebhookError> {
        self.observe("get_incoming", async {
            let mut conn = self.pool().acquire().await?;
            let stored = self.managed(&mut conn, principal, workspace_id, id).await?;
            self.incoming_view(&mut conn, principal, stored).await
        })
        .await
    }

    /// Makes an incoming webhook, for the workspace's owners and admins, on a database or a
    /// page they may share: with its integration, which the target is shared with to edit,
    /// within the agent limit. Its token, and its signing secret when it checks signatures,
    /// are returned once.
    pub async fn create_incoming(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        new: NewIncomingWebhook,
    ) -> Result<CreatedIncoming, WebhookError> {
        self.observe("create_incoming", async {
            let name = check_name(&new.name).map_err(WebhookError::invalid)?;
            if new.signed && !self.inner.envelope.has_key() {
                return Err(WebhookError::NoSecretKey);
            }
            let mut tx = self.pool().begin().await?;
            let integration = self
                .inner
                .agents
                .add_integration(&mut tx, principal, workspace_id, &name)
                .await?;
            let context = self
                .inner
                .pages
                .context(&mut tx, principal, new.target_id)
                .await
                .map_err(|error| match error {
                    crate::pages::PageError::NotFound(_) => WebhookError::NotFound(NO_PAGE),
                    other => other.into(),
                })?;
            if context.page().workspace_id != workspace_id {
                return Err(WebhookError::NotFound(NO_PAGE));
            }
            let kind = match context.page().kind {
                PageKind::Database => TargetKind::Database,
                PageKind::Page => TargetKind::Page,
                PageKind::Row => {
                    return Err(WebhookError::InvalidInput(
                        "a row is no webhook's target: name its database, or a page".to_owned(),
                    ));
                }
            };
            // Its maker shares the target with its integration, which they must be allowed.
            context
                .check(Action::SharePage)
                .map_err(|error| match error {
                    crate::pages::PageError::NotFound(_) => WebhookError::NotFound(NO_PAGE),
                    other => other.into(),
                })?;
            let now = self.now();
            let (_, shared) = self
                .inner
                .pages
                .insert_share(&mut tx, principal, &context, integration, PageAccess::Edit)
                .await?;
            let id = Uuid::now_v7();
            let token = new_secret(TOKEN_PREFIX);
            let secret = new.signed.then(|| new_secret(SECRET_PREFIX));
            let sealed = secret
                .as_deref()
                .map(|secret| self.seal_incoming(id, secret))
                .transpose()?;
            repo::insert(
                &mut *tx,
                &repo::New {
                    id,
                    workspace_id,
                    integration_id: integration,
                    target_id: new.target_id,
                    target_kind: kind.as_str(),
                    token_hash: &hash_secret(&token),
                    signing_secret: sealed.as_deref(),
                    created_by: principal.user.id,
                    at: now,
                },
            )
            .await?;
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_HOOK))?;
            let hook = self.incoming_view(&mut tx, principal, stored).await?;
            tx.commit().await?;
            self.inner.pages.report_access(shared);
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                hook_id = %id,
                integration_id = %integration,
                target_id = %new.target_id,
                signed = new.signed,
                "incoming webhook created"
            );
            Ok(CreatedIncoming {
                hook,
                token: Secret::new(token),
                signing_secret: secret.map(Secret::new),
            })
        })
        .await
    }

    /// Renames an incoming webhook and its integration, or turns it on or off.
    pub async fn update_incoming(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        update: IncomingUpdate,
    ) -> Result<IncomingWebhook, WebhookError> {
        self.observe("update_incoming", async {
            let name = update
                .name
                .as_deref()
                .map(check_name)
                .transpose()
                .map_err(WebhookError::invalid)?;
            let mut tx = self.pool().begin().await?;
            let stored = self.locked(&mut tx, principal, workspace_id, id).await?;
            let now = self.now();
            if let Some(name) = &name {
                agents::update(&mut *tx, stored.integration_id, Some(name), None, now).await?;
            }
            match update.enabled {
                Some(true) => repo::set_disabled(&mut *tx, id, None, now).await?,
                Some(false) if stored.disabled_at.is_none() => {
                    repo::set_disabled(&mut *tx, id, Some(now), now).await?;
                }
                _ => {}
            }
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_HOOK))?;
            let hook = self.incoming_view(&mut tx, principal, stored).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                hook_id = %id,
                enabled = hook.disabled_at.is_none(),
                "incoming webhook changed"
            );
            Ok(hook)
        })
        .await
    }

    /// Gives an incoming webhook a new URL token, returned once; the old URL stops working
    /// at once.
    pub async fn rotate_incoming_token(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(Secret, IncomingWebhook), WebhookError> {
        self.observe("rotate_incoming_token", async {
            let mut tx = self.pool().begin().await?;
            self.locked(&mut tx, principal, workspace_id, id).await?;
            let token = new_secret(TOKEN_PREFIX);
            repo::set_token(&mut *tx, id, &hash_secret(&token), self.now()).await?;
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_HOOK))?;
            let hook = self.incoming_view(&mut tx, principal, stored).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                hook_id = %id,
                "incoming webhook URL rotated"
            );
            Ok((Secret::new(token), hook))
        })
        .await
    }

    /// Makes an incoming webhook a new signing secret, returned once, so it takes only
    /// signed requests. The one before still verifies for [`ROTATION_OVERLAP`].
    pub async fn rotate_incoming_secret(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(Secret, IncomingWebhook), WebhookError> {
        self.observe("rotate_incoming_secret", async {
            if !self.inner.envelope.has_key() {
                return Err(WebhookError::NoSecretKey);
            }
            let mut tx = self.pool().begin().await?;
            let stored = self.locked(&mut tx, principal, workspace_id, id).await?;
            let secret = new_secret(SECRET_PREFIX);
            let sealed = self.seal_incoming(id, &secret)?;
            let now = self.now();
            let previous = stored
                .signing_secret
                .as_deref()
                .map(|old| (old, now + ROTATION_OVERLAP));
            repo::set_signing(&mut *tx, id, Some(&sealed), previous, now).await?;
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_HOOK))?;
            let hook = self.incoming_view(&mut tx, principal, stored).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                hook_id = %id,
                "incoming webhook signing secret rotated"
            );
            Ok((Secret::new(secret), hook))
        })
        .await
    }

    /// Stops checking an incoming webhook's signatures: it takes unsigned requests again,
    /// and its secrets are forgotten.
    pub async fn remove_incoming_secret(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<IncomingWebhook, WebhookError> {
        self.observe("remove_incoming_secret", async {
            let mut tx = self.pool().begin().await?;
            self.locked(&mut tx, principal, workspace_id, id).await?;
            repo::set_signing(&mut *tx, id, None, None, self.now()).await?;
            let stored = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_HOOK))?;
            let hook = self.incoming_view(&mut tx, principal, stored).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                hook_id = %id,
                "incoming webhook signatures no longer checked"
            );
            Ok(hook)
        })
        .await
    }

    /// Deletes an incoming webhook and its integration, which leaves the workspace, and so
    /// its target's share; what it made still names it.
    pub async fn delete_incoming(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WebhookError> {
        self.observe("delete_incoming", async {
            let mut tx = self.pool().begin().await?;
            let stored = self.locked(&mut tx, principal, workspace_id, id).await?;
            repo::delete(&mut *tx, id).await?;
            self.inner
                .agents
                .remove_integration(&mut tx, workspace_id, stored.integration_id)
                .await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                hook_id = %id,
                integration_id = %stored.integration_id,
                "incoming webhook deleted"
            );
            self.inner.agents.integration_ended(stored.integration_id);
            Ok(())
        })
        .await
    }

    /// An incoming webhook's log: its last requests, newest first.
    pub async fn incoming_requests(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Vec<LoggedRequest>, WebhookError> {
        self.observe("list_incoming_requests", async {
            let mut conn = self.pool().acquire().await?;
            self.managed(&mut conn, principal, workspace_id, id).await?;
            Ok(repo::requests(&mut *conn, id)
                .await?
                .into_iter()
                .map(|request| LoggedRequest {
                    id: request.id,
                    at: request.at,
                    status_code: request.status_code,
                    outcome: request.outcome,
                    action: request.action,
                    client_ip: request.client_ip,
                    latency_ms: request.latency_ms,
                })
                .collect())
        })
        .await
    }

    /// Takes a request to an incoming webhook's URL: limits its rate by client and by
    /// webhook, reads its body with `body`, which fails past [`MAX_INCOMING_BODY`](super::MAX_INCOMING_BODY), checks
    /// it, and does what it asks as the webhook's integration. See the module docs.
    pub async fn receive_incoming<F>(
        &self,
        request: IncomingRequest<'_>,
        body: F,
    ) -> Result<Accepted, IncomingError>
    where
        F: Future<Output = Result<Bytes, BodyError>> + Send,
    {
        let span = info_span!(
            "webhooks",
            otel.name = "webhooks receive",
            webhooks.operation = "receive",
            hook.id = tracing::field::Empty,
        );
        let guard = self.inner.red.start("receive");
        let result = self
            .receive(request, body, &span)
            .instrument(span.clone())
            .await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                neon_observability::errors::mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                neon_observability::errors::record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }

    async fn receive<F>(
        &self,
        request: IncomingRequest<'_>,
        body: F,
        span: &tracing::Span,
    ) -> Result<Accepted, IncomingError>
    where
        F: Future<Output = Result<Bytes, BodyError>> + Send,
    {
        let started = Instant::now();
        self.limit(Bucket::IncomingClient, request.client_ip)
            .await?;
        if request.token.len() > MAX_TOKEN || !request.token.starts_with(TOKEN_PREFIX) {
            return Err(IncomingError::UnknownHook);
        }
        let hook = repo::by_token_hash(self.pool(), &hash_secret(request.token))
            .await?
            .ok_or(IncomingError::UnknownHook)?;
        span.record("hook.id", tracing::field::display(hook.id));
        self.limit(Bucket::IncomingHook, &hook.id.to_string())
            .await?;
        let mut action = None;
        let result = self.take(&hook, &request, body, &mut action).await;
        let (status, outcome) = match &result {
            Ok(_) => (200, "ok"),
            Err(error) => (error.status(), error.code()),
        };
        let logged = Request {
            id: Uuid::now_v7(),
            hook_id: hook.id,
            at: self.now(),
            status_code: i32::from(status),
            outcome: outcome.to_owned(),
            action: action.map(str::to_owned),
            client_ip: request.client_ip.to_owned(),
            latency_ms: i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX),
        };
        let mut conn = self.pool().acquire().await?;
        repo::record_request(&mut conn, &logged).await?;
        info!(
            hook_id = %hook.id,
            status,
            outcome,
            action = action.unwrap_or(""),
            "incoming webhook request"
        );
        result
    }

    /// Checks a request to a webhook it found, and does what it asks; `action` is set once
    /// its payload reads.
    async fn take<F>(
        &self,
        hook: &Stored,
        request: &IncomingRequest<'_>,
        body: F,
        action: &mut Option<&'static str>,
    ) -> Result<Accepted, IncomingError>
    where
        F: Future<Output = Result<Bytes, BodyError>> + Send,
    {
        if hook.disabled_at.is_some() {
            return Err(IncomingError::Disabled);
        }
        let Some((user, standing)) = users::identity(self.pool(), hook.integration_id).await?
        else {
            return Err(IncomingError::Deleted);
        };
        match standing.status(UserKind::Integration, self.inner.workspaces.agents_on()) {
            AgentStatus::Active => {}
            AgentStatus::Deleted => return Err(IncomingError::Deleted),
            AgentStatus::Suspended | AgentStatus::Disabled => {
                return Err(IncomingError::Suspended);
            }
        }
        if !request.json {
            return Err(IncomingError::NotJson);
        }
        let body = body.await.map_err(|error| match error {
            BodyError::TooLarge => IncomingError::TooLarge,
            BodyError::Unreadable => IncomingError::Unreadable,
        })?;
        if let Some(sealed) = &hook.signing_secret {
            let secrets = self.incoming_secrets(hook, sealed)?;
            signature::verify(
                &secrets,
                request.signature,
                request.timestamp,
                &body,
                self.now(),
            )?;
        }
        let kind = TargetKind::parse(&hook.target_kind).ok_or_else(|| {
            IncomingError::Internal(format!("a hook's target kind: {}", hook.target_kind))
        })?;
        let order = payload::read(&body, kind).map_err(IncomingError::InvalidPayload)?;
        *action = Some(order.action());
        let principal = Principal {
            user,
            credential: Credential::Token {
                id: hook.id,
                name: hook.name.clone(),
                scopes: vec![Scope::Read, Scope::Write],
                workspace_id: Some(hook.workspace_id),
            },
        };
        match order {
            Order::CreateRow { values } => {
                let row = self
                    .inner
                    .databases
                    .create_row(
                        &principal,
                        hook.target_id,
                        NewRow {
                            values,
                            ..NewRow::default()
                        },
                    )
                    .await?;
                Ok(Accepted {
                    action: "create_row",
                    row: Some((row.id, row.unique_id)),
                })
            }
            Order::UpdateRow { unique_id, values } => {
                let found = self
                    .inner
                    .databases
                    .row_by_unique_id(&principal, hook.target_id, &unique_id)
                    .await?;
                let row = self
                    .inner
                    .databases
                    .update_row(&principal, found.id, values)
                    .await?;
                Ok(Accepted {
                    action: "update_row",
                    row: Some((row.id, row.unique_id)),
                })
            }
            Order::Append { blocks } => {
                self.inner
                    .sync
                    .append(&principal, hook.target_id, blocks)
                    .await?;
                Ok(Accepted {
                    action: "append",
                    row: None,
                })
            }
        }
    }

    async fn limit(&self, bucket: Bucket, key: &str) -> Result<(), IncomingError> {
        self.inner
            .limiter
            .check(bucket, key)
            .await
            .map_err(|limited| {
                tracing::warn!(bucket = bucket.prefix(), "rate limited");
                IncomingError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    /// The secrets that verify a webhook's requests now.
    fn incoming_secrets(
        &self,
        hook: &Stored,
        sealed: &[u8],
    ) -> Result<Vec<zeroize::Zeroizing<Vec<u8>>>, IncomingError> {
        let aad = incoming_aad(hook.id);
        let open = |sealed: &[u8]| {
            self.inner.envelope.open(sealed, &aad).map_err(|error| {
                IncomingError::Internal(format!("a webhook's secret does not open: {error}"))
            })
        };
        let mut secrets = vec![open(sealed)?];
        if let (Some(previous), Some(until)) =
            (&hook.previous_signing_secret, hook.previous_expires_at)
            && until > self.now()
        {
            secrets.push(open(previous)?);
        }
        Ok(secrets)
    }

    fn seal_incoming(&self, id: Uuid, secret: &str) -> Result<Vec<u8>, WebhookError> {
        self.inner
            .envelope
            .seal(secret.as_bytes(), &incoming_aad(id))
            .map_err(|_| WebhookError::NoSecretKey)
    }

    /// Where the principal stands in a workspace whose incoming webhooks the rules let
    /// them manage.
    async fn managing(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Standing, WebhookError> {
        let workspace = neon_store::workspaces::get(&mut *conn, workspace_id)
            .await?
            .ok_or(WebhookError::NotFound(NO_WORKSPACE))?;
        let standing = self.standing(&mut *conn, principal, workspace_id).await?;
        standing
            .authorize_same(
                Action::ManageIncomingWebhooks,
                Resource::Workspace {
                    members_can_create_teamspaces: workspace.members_can_create_teamspaces,
                },
            )
            .map_err(|denied| WebhookError::denied(denied, NO_WORKSPACE))?;
        Ok(standing)
    }

    /// An incoming webhook of a workspace the principal manages them in.
    async fn managed(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Stored, WebhookError> {
        self.managing(&mut *conn, principal, workspace_id).await?;
        repo::get(&mut *conn, workspace_id, id)
            .await?
            .ok_or(WebhookError::NotFound(NO_HOOK))
    }

    /// As [`managed`](Self::managed), locking the webhook until the transaction ends.
    async fn locked(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Stored, WebhookError> {
        self.managing(&mut *conn, principal, workspace_id).await?;
        if !repo::lock(&mut *conn, workspace_id, id).await? {
            return Err(WebhookError::NotFound(NO_HOOK));
        }
        repo::get(&mut *conn, workspace_id, id)
            .await?
            .ok_or(WebhookError::NotFound(NO_HOOK))
    }

    /// A webhook as its managers see it: its integration's status, and its target's title
    /// when the principal may see the target.
    async fn incoming_view(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        stored: Stored,
    ) -> Result<IncomingWebhook, WebhookError> {
        let integration = agents::get(&mut *conn, stored.workspace_id, stored.integration_id)
            .await?
            .ok_or_else(|| WebhookError::Internal("a hook's integration is gone".to_owned()))?;
        let title = Facts::read(&mut *conn, principal, &[stored.target_id])
            .await?
            .seen(principal)?
            .remove(&stored.target_id)
            .flatten()
            .map(|view| view.page.title);
        let created_by = match stored.created_by {
            Some(id) => users::by_id(&mut *conn, id).await?.map(|user| Creator {
                id,
                display_name: user.display_name,
            }),
            None => None,
        };
        let kind = TargetKind::parse(&stored.target_kind).ok_or_else(|| {
            WebhookError::Internal(format!("a hook's target kind: {}", stored.target_kind))
        })?;
        Ok(IncomingWebhook {
            integration: Integration {
                id: integration.id,
                status: integration
                    .standing
                    .status(integration.kind, self.inner.workspaces.agents_on()),
                owner: integration.owner,
            },
            target: Target {
                id: stored.target_id,
                kind,
                title,
            },
            signed: stored.signing_secret.is_some(),
            previous_secret_expires_at: stored
                .previous_expires_at
                .filter(|until| stored.signing_secret.is_some() && *until > self.now()),
            id: stored.id,
            workspace_id: stored.workspace_id,
            name: stored.name,
            disabled_at: stored.disabled_at,
            created_by,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
        })
    }
}

/// Gives the incoming webhook an integration delivers a new URL token and, while it checks
/// signatures, a new signing secret, neither shown to anyone, and forgets the secret rotated
/// out before: after the integration was handed to another person, on the transaction that
/// did it. Its URL and secret were shown to whoever made them, and would otherwise go on
/// writing for the new owner; its managers make a new URL, and a new secret, to use it
/// again. Without a key to seal with, the secret it has is kept: it does not open either.
/// Says whether the integration had a webhook.
pub(crate) async fn hand_over(
    conn: &mut PgConnection,
    envelope: &Envelope,
    integration_id: Uuid,
    at: OffsetDateTime,
) -> Result<bool, WorkspaceError> {
    let Some(hook) = repo::lock_of_integration(&mut *conn, integration_id).await? else {
        return Ok(false);
    };
    repo::set_token(
        &mut *conn,
        hook.id,
        &hash_secret(&new_secret(TOKEN_PREFIX)),
        at,
    )
    .await?;
    if let Some(sealed) = &hook.signing_secret {
        let replaced = if envelope.has_key() {
            envelope
                .seal(new_secret(SECRET_PREFIX).as_bytes(), &incoming_aad(hook.id))
                .map_err(|error| {
                    WorkspaceError::Internal(format!("sealing a webhook's secret: {error}"))
                })?
        } else {
            sealed.clone()
        };
        repo::set_signing(&mut *conn, hook.id, Some(&replaced), None, at).await?;
    }
    info!(
        hook_id = %hook.id,
        integration_id = %integration_id,
        "incoming webhook's URL and secret replaced: its integration was handed over"
    );
    Ok(true)
}

/// What an incoming webhook's secrets are sealed with: its id, so a sealed secret copied to
/// another webhook does not open there.
fn incoming_aad(id: Uuid) -> Vec<u8> {
    format!("incoming-webhook:{id}").into_bytes()
}
