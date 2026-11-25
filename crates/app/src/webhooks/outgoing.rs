//! Registering outgoing webhooks, changing, rotating and deleting them, their logs, and
//! test events.

use neon_domain::auth::Principal;
use neon_domain::settings::Secret;
use neon_domain::webhooks::{EventType, check_events, check_url};
use neon_domain::workspaces::{Action, Person};
use neon_store::webhooks::{self as repo, Attempt, Listed, Webhook};
use neon_store::workspaces;
use time::OffsetDateTime;
use tracing::info;
use url::Url;
use uuid::Uuid;

use super::signature::SECRET_PREFIX;
use super::transport::SendError;
use super::{NO_WEBHOOK, ROTATION_OVERLAP, WebhookError, WebhookService, aad, check, known_events};
use crate::crypto::new_secret;

const NO_MEMBER: &str = "no such member";

/// Who made a webhook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creator {
    pub id: Uuid,
    pub display_name: String,
}

/// An outgoing webhook as the caller sees it, without its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingWebhook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The identity whose events it delivers.
    pub identity: Person,
    pub url: String,
    pub events: Vec<EventType>,
    /// Whether it is told of the identity's own changes too.
    pub include_own: bool,
    /// Set while it is off, with why.
    pub disabled_at: Option<OffsetDateTime>,
    pub disabled_reason: Option<String>,
    /// Deliveries given up on in a row since the last that went through.
    pub failures: i32,
    /// Until when the secret before the last rotation still signs deliveries too.
    pub previous_secret_expires_at: Option<OffsetDateTime>,
    pub created_by: Option<Creator>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A webhook to register.
#[derive(Debug, Clone, Default)]
pub struct NewWebhook {
    /// The identity whose events it delivers.
    pub user_id: Uuid,
    pub url: String,
    /// Event types, such as `row.assigned`: at least one.
    pub events: Vec<String>,
    pub include_own: bool,
}

/// What a change to a webhook sets; each part left out stays.
#[derive(Debug, Clone, Default)]
pub struct WebhookUpdate {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub include_own: Option<bool>,
    /// Turns it on, clearing its failures, or off.
    pub enabled: Option<bool>,
}

/// One attempt of a webhook's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAttempt {
    pub id: Uuid,
    /// The delivery it is an attempt at; none for a test event.
    pub delivery_id: Option<Uuid>,
    pub event_id: Uuid,
    pub event_type: String,
    pub attempt: i32,
    /// `delivered`, `retrying`, `failed` or `skipped`.
    pub outcome: String,
    pub status_code: Option<i32>,
    pub latency_ms: Option<i32>,
    /// Why it failed or was skipped, such as `timeout` or `no_access`.
    pub error_kind: Option<String>,
    /// The start of the answer's body.
    pub excerpt: Option<String>,
    pub at: OffsetDateTime,
}

impl From<Attempt> for DeliveryAttempt {
    fn from(attempt: Attempt) -> Self {
        Self {
            id: attempt.id,
            delivery_id: attempt.delivery_id,
            event_id: attempt.event_id,
            event_type: attempt.event_type,
            attempt: attempt.attempt,
            outcome: attempt.outcome,
            status_code: attempt.status_code,
            latency_ms: attempt.latency_ms,
            error_kind: attempt.error_kind,
            excerpt: attempt.excerpt,
            at: attempt.at,
        }
    }
}

fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|db| db.kind() == sqlx::error::ErrorKind::ForeignKeyViolation)
}

fn view(listed: Listed) -> OutgoingWebhook {
    let webhook = listed.webhook;
    OutgoingWebhook {
        events: known_events(&webhook.events),
        id: webhook.id,
        workspace_id: webhook.workspace_id,
        identity: listed.identity,
        url: webhook.url,
        include_own: webhook.include_own,
        disabled_at: webhook.disabled_at,
        disabled_reason: webhook.disabled_reason,
        failures: webhook.failures,
        previous_secret_expires_at: webhook.previous_expires_at,
        created_by: listed
            .creator
            .map(|(id, display_name)| Creator { id, display_name }),
        created_at: webhook.created_at,
        updated_at: webhook.updated_at,
    }
}

/// A refused address or a host that does not resolve, as the caller is told.
pub(super) fn address_refused(error: &SendError) -> WebhookError {
    WebhookError::AddressRefused(match error {
        SendError::Refused(blocked) => format!(
            "the URL's host is {blocked}, which webhooks may not reach: see \
             webhooks.allow_private"
        ),
        SendError::Unresolvable => "the URL's host does not resolve".to_owned(),
        other => other.to_string(),
    })
}

impl WebhookService {
    /// The workspace's webhooks the principal may manage, oldest first: every one for its
    /// owners and admins, for everyone else their own and their agents' and integrations'.
    /// `user_id` keeps those of one identity.
    pub async fn list_webhooks(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        user_id: Option<Uuid>,
    ) -> Result<Vec<OutgoingWebhook>, WebhookError> {
        self.observe("list_webhooks", async {
            let mut conn = self.pool().acquire().await?;
            let standing = self.standing(&mut conn, principal, workspace_id).await?;
            Ok(repo::list(&mut *conn, workspace_id)
                .await?
                .into_iter()
                .filter(|listed| user_id.is_none_or(|id| listed.identity.id == id))
                .filter(|listed| {
                    check(
                        &standing,
                        principal,
                        &listed.identity,
                        Action::ManageWebhook,
                    )
                    .is_ok()
                })
                .map(view)
                .collect())
        })
        .await
    }

    /// One webhook the principal may manage.
    pub async fn webhook(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<OutgoingWebhook, WebhookError> {
        self.observe("get_webhook", async {
            let mut conn = self.pool().acquire().await?;
            let listed = self
                .manageable(&mut conn, principal, workspace_id, id)
                .await?;
            Ok(view(listed))
        })
        .await
    }

    /// Registers a webhook that delivers an identity's events, for those the rules let:
    /// the workspace's owners and admins for anyone in it, everyone else but guests for
    /// themselves and their own agents and integrations. Its URL must resolve to addresses
    /// webhooks may reach. Its signing secret is made here and returned once; it is kept
    /// sealed.
    pub async fn create_webhook(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        new: NewWebhook,
    ) -> Result<(Secret, OutgoingWebhook), WebhookError> {
        self.observe("create_webhook", async {
            let events = check_events(&new.events).map_err(WebhookError::InvalidInput)?;
            let url =
                check_url(&new.url).map_err(|error| WebhookError::InvalidUrl(error.to_string()))?;
            {
                let mut conn = self.pool().acquire().await?;
                let standing = self.standing(&mut conn, principal, workspace_id).await?;
                let member = workspaces::member(&mut *conn, workspace_id, new.user_id)
                    .await?
                    .ok_or(WebhookError::NotFound(NO_MEMBER))?;
                check(
                    &standing,
                    principal,
                    &member.person,
                    Action::RegisterWebhook,
                )
                .map_err(|error| match error {
                    WebhookError::NotFound(_) => WebhookError::NotFound(NO_MEMBER),
                    other => other,
                })?;
            }
            if !self.inner.envelope.has_key() {
                return Err(WebhookError::NoSecretKey);
            }
            self.reachable(&url).await?;
            let id = Uuid::now_v7();
            let secret = new_secret(SECRET_PREFIX);
            let now = self.now();
            let webhook = Webhook {
                id,
                workspace_id,
                user_id: new.user_id,
                url: url.to_string(),
                events: events
                    .iter()
                    .map(|event| event.as_str().to_owned())
                    .collect(),
                include_own: new.include_own,
                secret: self.seal(id, &secret)?,
                previous_secret: None,
                previous_expires_at: None,
                disabled_at: None,
                disabled_reason: None,
                failures: 0,
                created_by: Some(principal.user.id),
                created_at: now,
                updated_at: now,
            };
            let mut tx = self.pool().begin().await?;
            // The identity must still be in the workspace, as the membership's key holds.
            repo::insert(&mut *tx, &webhook).await.map_err(|error| {
                if is_foreign_key_violation(&error) {
                    WebhookError::NotFound(NO_MEMBER)
                } else {
                    WebhookError::Database(error)
                }
            })?;
            let listed = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                webhook_id = %id,
                identity_id = %new.user_id,
                events = ?webhook.events,
                "webhook registered"
            );
            Ok((Secret::new(secret), view(listed)))
        })
        .await
    }

    /// Changes a webhook's URL, events or whether it hears the identity's own changes, or
    /// turns it on or off. Turning it on clears its failures and why it was off.
    pub async fn update_webhook(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        update: WebhookUpdate,
    ) -> Result<OutgoingWebhook, WebhookError> {
        self.observe("update_webhook", async {
            let events = update
                .events
                .as_deref()
                .map(check_events)
                .transpose()
                .map_err(WebhookError::InvalidInput)?;
            let url = update
                .url
                .as_deref()
                .map(check_url)
                .transpose()
                .map_err(|error| WebhookError::InvalidUrl(error.to_string()))?;
            {
                let mut conn = self.pool().acquire().await?;
                self.manageable(&mut conn, principal, workspace_id, id)
                    .await?;
            }
            if let Some(url) = &url {
                self.reachable(url).await?;
            }
            let mut tx = self.pool().begin().await?;
            let mut webhook = repo::lock(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
            self.manageable(&mut tx, principal, workspace_id, id)
                .await?;
            if let Some(url) = url {
                webhook.url = url.to_string();
            }
            if let Some(events) = events {
                webhook.events = events
                    .iter()
                    .map(|event| event.as_str().to_owned())
                    .collect();
            }
            if let Some(include_own) = update.include_own {
                webhook.include_own = include_own;
            }
            let now = self.now();
            match update.enabled {
                Some(true) => {
                    webhook.disabled_at = None;
                    webhook.disabled_reason = None;
                    webhook.failures = 0;
                }
                Some(false) if webhook.disabled_at.is_none() => {
                    webhook.disabled_at = Some(now);
                    webhook.disabled_reason =
                        Some(format!("turned off by {}", principal.user.display_name));
                }
                _ => {}
            }
            webhook.updated_at = now;
            repo::update(&mut *tx, &webhook).await?;
            let listed = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                webhook_id = %id,
                enabled = webhook.disabled_at.is_none(),
                "webhook changed"
            );
            Ok(view(listed))
        })
        .await
    }

    /// Makes a new signing secret, returned once. The one before keeps signing deliveries
    /// beside it for [`ROTATION_OVERLAP`]; one rotated earlier stops at once.
    pub async fn rotate_webhook_secret(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(Secret, OutgoingWebhook), WebhookError> {
        self.observe("rotate_webhook_secret", async {
            if !self.inner.envelope.has_key() {
                return Err(WebhookError::NoSecretKey);
            }
            let mut tx = self.pool().begin().await?;
            let mut webhook = repo::lock(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
            self.manageable(&mut tx, principal, workspace_id, id)
                .await?;
            let secret = new_secret(SECRET_PREFIX);
            let now = self.now();
            let sealed = self.seal(id, &secret)?;
            webhook.previous_secret = Some(std::mem::replace(&mut webhook.secret, sealed));
            webhook.previous_expires_at = Some(now + ROTATION_OVERLAP);
            webhook.updated_at = now;
            repo::update(&mut *tx, &webhook).await?;
            let listed = repo::get(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                webhook_id = %id,
                "webhook secret rotated"
            );
            Ok((Secret::new(secret), view(listed)))
        })
        .await
    }

    /// Deletes a webhook, with its log. Deliveries queued for it are dropped as they come
    /// up.
    pub async fn delete_webhook(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WebhookError> {
        self.observe("delete_webhook", async {
            let mut tx = self.pool().begin().await?;
            repo::lock(&mut *tx, workspace_id, id)
                .await?
                .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
            self.manageable(&mut tx, principal, workspace_id, id)
                .await?;
            repo::delete(&mut *tx, workspace_id, id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                webhook_id = %id,
                "webhook deleted"
            );
            Ok(())
        })
        .await
    }

    /// A webhook's log: its last attempts, newest first.
    pub async fn webhook_attempts(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Vec<DeliveryAttempt>, WebhookError> {
        self.observe("list_webhook_attempts", async {
            let mut conn = self.pool().acquire().await?;
            self.manageable(&mut conn, principal, workspace_id, id)
                .await?;
            Ok(repo::attempts(&mut *conn, id)
                .await?
                .into_iter()
                .map(DeliveryAttempt::from)
                .collect())
        })
        .await
    }

    /// Sends a test event to a webhook now, on or off, signed as deliveries are, and says
    /// how it went; its log keeps the attempt. It is not retried, and counts for nothing
    /// toward turning the webhook off.
    pub async fn test_webhook(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<DeliveryAttempt, WebhookError> {
        self.observe("test_webhook", async {
            let listed = {
                let mut conn = self.pool().acquire().await?;
                self.manageable(&mut conn, principal, workspace_id, id)
                    .await?
            };
            let attempt = self.send_test(&listed).await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                webhook_id = %id,
                outcome = %attempt.outcome,
                status = ?attempt.status_code,
                "webhook test event sent"
            );
            Ok(attempt)
        })
        .await
    }

    /// A webhook of the workspace, once the rules let the principal manage it.
    async fn manageable(
        &self,
        conn: &mut sqlx::PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Listed, WebhookError> {
        let standing = self.standing(&mut *conn, principal, workspace_id).await?;
        let listed = repo::get(&mut *conn, workspace_id, id)
            .await?
            .ok_or(WebhookError::NotFound(NO_WEBHOOK))?;
        check(
            &standing,
            principal,
            &listed.identity,
            Action::ManageWebhook,
        )?;
        Ok(listed)
    }

    /// Refuses a URL whose host does not resolve, or resolves to an address webhooks may not
    /// reach now.
    async fn reachable(&self, url: &Url) -> Result<(), WebhookError> {
        self.inner
            .transport
            .check(url, self.allow_private())
            .await
            .map_err(|error| address_refused(&error))
    }

    fn seal(&self, id: Uuid, secret: &str) -> Result<Vec<u8>, WebhookError> {
        self.inner
            .envelope
            .seal(secret.as_bytes(), &aad(id))
            .map_err(|_| WebhookError::NoSecretKey)
    }
}
