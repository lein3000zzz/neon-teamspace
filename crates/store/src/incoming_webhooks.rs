//! Incoming webhooks, and the log of the requests they took.

use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// How many requests each incoming webhook's log keeps: the latest.
pub const LOG_LENGTH: i64 = 50;

/// An incoming webhook as stored, with its integration's name. Its signing secrets are
/// sealed; of its URL's token only the hash is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingWebhook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub integration_id: Uuid,
    /// Its integration's name.
    pub name: String,
    pub target_id: Uuid,
    /// `database` or `page`.
    pub target_kind: String,
    pub signing_secret: Option<Vec<u8>>,
    pub previous_signing_secret: Option<Vec<u8>>,
    pub previous_expires_at: Option<OffsetDateTime>,
    pub disabled_at: Option<OffsetDateTime>,
    pub created_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A new incoming webhook.
#[derive(Debug, Clone)]
pub struct New<'a> {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub integration_id: Uuid,
    pub target_id: Uuid,
    pub target_kind: &'a str,
    pub token_hash: &'a [u8],
    pub signing_secret: Option<&'a [u8]>,
    pub created_by: Uuid,
    pub at: OffsetDateTime,
}

pub async fn insert(executor: impl PgExecutor<'_>, new: &New<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO incoming_webhooks (id, workspace_id, integration_id, target_id, target_kind,
                                        token_hash, signing_secret, created_by, created_at,
                                        updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
        new.id,
        new.workspace_id,
        new.integration_id,
        new.target_id,
        new.target_kind,
        new.token_hash,
        new.signing_secret,
        new.created_by,
        new.at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A workspace's incoming webhooks, oldest first.
pub async fn list(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<IncomingWebhook>> {
    sqlx::query_as!(
        IncomingWebhook,
        "SELECT h.id, h.workspace_id, h.integration_id, u.display_name AS name, h.target_id,
                h.target_kind, h.signing_secret, h.previous_signing_secret,
                h.previous_expires_at, h.disabled_at, h.created_by, h.created_at, h.updated_at
         FROM incoming_webhooks h
         JOIN users u ON u.id = h.integration_id
         WHERE h.workspace_id = $1
         ORDER BY h.created_at, h.id",
        workspace_id,
    )
    .fetch_all(executor)
    .await
}

/// One incoming webhook of a workspace.
pub async fn get(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<IncomingWebhook>> {
    sqlx::query_as!(
        IncomingWebhook,
        "SELECT h.id, h.workspace_id, h.integration_id, u.display_name AS name, h.target_id,
                h.target_kind, h.signing_secret, h.previous_signing_secret,
                h.previous_expires_at, h.disabled_at, h.created_by, h.created_at, h.updated_at
         FROM incoming_webhooks h
         JOIN users u ON u.id = h.integration_id
         WHERE h.workspace_id = $1 AND h.id = $2",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await
}

/// One incoming webhook of a workspace, locked until the transaction ends.
pub async fn lock(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM incoming_webhooks WHERE workspace_id = $1 AND id = $2 FOR UPDATE",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .is_some())
}

/// The incoming webhook whose URL's token has this hash.
pub async fn by_token_hash(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
) -> sqlx::Result<Option<IncomingWebhook>> {
    sqlx::query_as!(
        IncomingWebhook,
        "SELECT h.id, h.workspace_id, h.integration_id, u.display_name AS name, h.target_id,
                h.target_kind, h.signing_secret, h.previous_signing_secret,
                h.previous_expires_at, h.disabled_at, h.created_by, h.created_at, h.updated_at
         FROM incoming_webhooks h
         JOIN users u ON u.id = h.integration_id
         WHERE h.token_hash = $1",
        token_hash,
    )
    .fetch_optional(executor)
    .await
}

/// Turns a webhook on, or off since `disabled_at`.
pub async fn set_disabled(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    disabled_at: Option<OffsetDateTime>,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE incoming_webhooks SET disabled_at = $2, updated_at = $3 WHERE id = $1",
        id,
        disabled_at,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Gives a webhook a new URL token, the old one working no more.
pub async fn set_token(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    token_hash: &[u8],
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE incoming_webhooks SET token_hash = $2, updated_at = $3 WHERE id = $1",
        id,
        token_hash,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Sets a webhook's signing secrets: the current one, unset to take unsigned requests, and
/// the one before with until when it still verifies.
pub async fn set_signing(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    secret: Option<&[u8]>,
    previous: Option<(&[u8], OffsetDateTime)>,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    let (previous, expires) = previous.unzip();
    sqlx::query!(
        "UPDATE incoming_webhooks
         SET signing_secret = $2, previous_signing_secret = $3, previous_expires_at = $4,
             updated_at = $5
         WHERE id = $1",
        id,
        secret,
        previous,
        expires,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Forgets the signing secrets rotated out whose overlap ended at `now` or before, saying how
/// many.
pub async fn forget_rotated(
    executor: impl PgExecutor<'_>,
    now: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE incoming_webhooks
         SET previous_signing_secret = NULL, previous_expires_at = NULL
         WHERE previous_expires_at <= $1",
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

pub async fn delete(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM incoming_webhooks WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// A request an incoming webhook took, as its log keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: Uuid,
    pub hook_id: Uuid,
    pub at: OffsetDateTime,
    pub status_code: i32,
    pub outcome: String,
    pub action: Option<String>,
    pub client_ip: String,
    pub latency_ms: i32,
}

/// Adds a request to its webhook's log, which then keeps its last [`LOG_LENGTH`].
pub async fn record_request(conn: &mut sqlx::PgConnection, request: &Request) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO incoming_webhook_requests (id, hook_id, at, status_code, outcome, action,
                                                client_ip, latency_ms)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8
         WHERE EXISTS (SELECT 1 FROM incoming_webhooks WHERE id = $2)",
        request.id,
        request.hook_id,
        request.at,
        request.status_code,
        request.outcome,
        request.action,
        request.client_ip,
        request.latency_ms,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "DELETE FROM incoming_webhook_requests
         WHERE hook_id = $1 AND id NOT IN (
             SELECT id FROM incoming_webhook_requests WHERE hook_id = $1
             ORDER BY at DESC, id DESC LIMIT $2
         )",
        request.hook_id,
        LOG_LENGTH,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// A webhook's log, newest first.
pub async fn requests(executor: impl PgExecutor<'_>, hook_id: Uuid) -> sqlx::Result<Vec<Request>> {
    sqlx::query_as!(
        Request,
        "SELECT id, hook_id, at, status_code, outcome, action, client_ip, latency_ms
         FROM incoming_webhook_requests WHERE hook_id = $1
         ORDER BY at DESC, id DESC
         LIMIT $2",
        hook_id,
        LOG_LENGTH,
    )
    .fetch_all(executor)
    .await
}
