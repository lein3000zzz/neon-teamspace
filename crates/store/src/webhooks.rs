//! Outgoing webhooks, and the log of their deliveries' attempts.

use neon_domain::workspaces::Person;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::users::PersonColumns;

/// How many attempts each webhook's log keeps: the latest.
pub const LOG_LENGTH: i64 = 50;

/// An outgoing webhook as stored. Its secrets are sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Webhook {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The identity whose events it delivers.
    pub user_id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    pub include_own: bool,
    pub secret: Vec<u8>,
    pub previous_secret: Option<Vec<u8>>,
    pub previous_expires_at: Option<OffsetDateTime>,
    pub disabled_at: Option<OffsetDateTime>,
    pub disabled_reason: Option<String>,
    pub failures: i32,
    pub created_by: Option<Uuid>,
    /// Who set its URL last.
    pub url_set_by: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A webhook with the identity it delivers the events of, without their address, and who
/// made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub webhook: Webhook,
    pub identity: Person,
    pub creator: Option<(Uuid, String)>,
}

pub async fn insert(executor: impl PgExecutor<'_>, webhook: &Webhook) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO webhooks (id, workspace_id, user_id, url, events, include_own, secret,
                               previous_secret, previous_expires_at, disabled_at,
                               disabled_reason, failures, created_by, url_set_by, created_at,
                               updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
        webhook.id,
        webhook.workspace_id,
        webhook.user_id,
        webhook.url,
        &webhook.events,
        webhook.include_own,
        webhook.secret,
        webhook.previous_secret,
        webhook.previous_expires_at,
        webhook.disabled_at,
        webhook.disabled_reason,
        webhook.failures,
        webhook.created_by,
        webhook.url_set_by,
        webhook.created_at,
        webhook.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Saves what may change of a webhook: its URL and who set it, its events, secrets, whether
/// it is off and why, and its count of failures.
pub async fn update(executor: impl PgExecutor<'_>, webhook: &Webhook) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE webhooks
         SET url = $2, events = $3, include_own = $4, secret = $5, previous_secret = $6,
             previous_expires_at = $7, disabled_at = $8, disabled_reason = $9, failures = $10,
             url_set_by = $11, updated_at = $12
         WHERE id = $1",
        webhook.id,
        webhook.url,
        &webhook.events,
        webhook.include_own,
        webhook.secret,
        webhook.previous_secret,
        webhook.previous_expires_at,
        webhook.disabled_at,
        webhook.disabled_reason,
        webhook.failures,
        webhook.url_set_by,
        webhook.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM webhooks WHERE workspace_id = $1 AND id = $2",
        workspace_id,
        id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// How many webhooks a workspace has, and how many of them are of `user_id`.
pub async fn counts(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<(i64, i64)> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "all!", count(*) FILTER (WHERE user_id = $2) AS "theirs!"
           FROM webhooks WHERE workspace_id = $1"#,
        workspace_id,
        user_id,
    )
    .fetch_one(executor)
    .await?;
    Ok((row.all, row.theirs))
}

/// A webhook by its id alone, for its deliveries.
pub async fn by_id(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Webhook>> {
    sqlx::query_as!(
        Webhook,
        "SELECT id, workspace_id, user_id, url, events, include_own, secret, previous_secret,
                previous_expires_at, disabled_at, disabled_reason, failures, created_by,
                url_set_by, created_at, updated_at
         FROM webhooks WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await
}

/// A webhook of a workspace, locked until the transaction ends.
pub async fn lock(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Webhook>> {
    sqlx::query_as!(
        Webhook,
        "SELECT id, workspace_id, user_id, url, events, include_own, secret, previous_secret,
                previous_expires_at, disabled_at, disabled_reason, failures, created_by,
                url_set_by, created_at, updated_at
         FROM webhooks WHERE workspace_id = $1 AND id = $2
         FOR UPDATE",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await
}

/// A workspace's webhooks, oldest first, with whose events each delivers.
pub async fn list(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<Vec<Listed>> {
    sqlx::query_as!(
        ListedRow,
        r#"SELECT w.id, w.workspace_id, w.user_id, w.url, w.events, w.include_own, w.secret,
                  w.previous_secret, w.previous_expires_at, w.disabled_at, w.disabled_reason,
                  w.failures, w.created_by, w.url_set_by, w.created_at, w.updated_at,
                  u.kind, u.display_name, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  c.display_name AS "creator_name?"
           FROM webhooks w
           JOIN users u ON u.id = w.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           LEFT JOIN users c ON c.id = w.created_by
           WHERE w.workspace_id = $1
           ORDER BY w.created_at, w.id"#,
        workspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Listed::try_from)
    .collect()
}

/// One webhook of a workspace, with whose events it delivers.
pub async fn get(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Listed>> {
    sqlx::query_as!(
        ListedRow,
        r#"SELECT w.id, w.workspace_id, w.user_id, w.url, w.events, w.include_own, w.secret,
                  w.previous_secret, w.previous_expires_at, w.disabled_at, w.disabled_reason,
                  w.failures, w.created_by, w.url_set_by, w.created_at, w.updated_at,
                  u.kind, u.display_name, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  c.display_name AS "creator_name?"
           FROM webhooks w
           JOIN users u ON u.id = w.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           LEFT JOIN users c ON c.id = w.created_by
           WHERE w.workspace_id = $1 AND w.id = $2"#,
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Listed::try_from)
    .transpose()
}

/// A webhook a change is told to: the identity it is for, and whether it is told of that
/// identity's own changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subscriber {
    pub webhook_id: Uuid,
    pub user_id: Uuid,
    pub include_own: bool,
}

/// The webhooks of a workspace that are on and ask for `event`, of the identities `users`.
pub async fn subscribers(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    users: &[Uuid],
    event: &str,
) -> sqlx::Result<Vec<Subscriber>> {
    sqlx::query_as!(
        Subscriber,
        r#"SELECT id AS webhook_id, user_id, include_own FROM webhooks
           WHERE workspace_id = $1 AND user_id = ANY($2) AND disabled_at IS NULL
             AND $3 = ANY(events)
           ORDER BY id"#,
        workspace_id,
        users,
        event,
    )
    .fetch_all(executor)
    .await
}

/// Turns a webhook off, saying why, unless it is off already. False when it was.
pub async fn turn_off(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    reason: &str,
    at: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE webhooks SET disabled_at = $3, disabled_reason = $2, updated_at = $3
         WHERE id = $1 AND disabled_at IS NULL",
        id,
        reason,
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// A webhook that is on: whose events it delivers, and who set its URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    pub id: Uuid,
    pub user_id: Uuid,
    pub url_set_by: Option<Uuid>,
}

/// The webhooks of a workspace that are on, whose URL `user_id` set or whose events are
/// theirs.
pub async fn registrations_of(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<Registration>> {
    sqlx::query_as!(
        Registration,
        "SELECT id, user_id, url_set_by FROM webhooks
         WHERE workspace_id = $1 AND disabled_at IS NULL
           AND (url_set_by = $2 OR user_id = $2)
         ORDER BY id",
        workspace_id,
        user_id,
    )
    .fetch_all(executor)
    .await
}

/// A delivery went through: the count of failures starts again.
pub async fn delivered(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE webhooks SET failures = 0 WHERE id = $1 AND failures <> 0",
        id
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// What giving up on a delivery did to its webhook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaveUp {
    /// Deliveries given up on in a row now.
    pub failures: i32,
    /// This one turned the webhook off.
    pub disabled: bool,
}

/// A delivery was given up on: one more failure in a row, which turns the webhook off with
/// `reason` once there are `limit`. `None` when the webhook is gone.
pub async fn gave_up(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    limit: i32,
    reason: &str,
    at: OffsetDateTime,
) -> sqlx::Result<Option<GaveUp>> {
    let row = sqlx::query!(
        r#"WITH before AS (SELECT disabled_at FROM webhooks WHERE id = $1 FOR UPDATE)
           UPDATE webhooks w
           SET failures = w.failures + 1,
               disabled_at = CASE WHEN w.disabled_at IS NULL AND w.failures + 1 >= $2
                                  THEN $4 ELSE w.disabled_at END,
               disabled_reason = CASE WHEN w.disabled_at IS NULL AND w.failures + 1 >= $2
                                      THEN $3 ELSE w.disabled_reason END
           FROM before
           WHERE w.id = $1
           RETURNING w.failures, (before.disabled_at IS NULL AND w.disabled_at IS NOT NULL)
                     AS "disabled!""#,
        id,
        limit,
        reason,
        at,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|row| GaveUp {
        failures: row.failures,
        disabled: row.disabled,
    }))
}

/// One attempt at a delivery, as its webhook's log keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub delivery_id: Option<Uuid>,
    pub event_id: Uuid,
    pub event_type: String,
    pub attempt: i32,
    pub outcome: String,
    pub status_code: Option<i32>,
    pub latency_ms: Option<i32>,
    pub error_kind: Option<String>,
    pub excerpt: Option<String>,
    pub at: OffsetDateTime,
}

/// Adds an attempt to its webhook's log, which then keeps its last [`LOG_LENGTH`].
pub async fn record_attempt(conn: &mut sqlx::PgConnection, attempt: &Attempt) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO webhook_attempts (id, webhook_id, delivery_id, event_id, event_type,
                                       attempt, outcome, status_code, latency_ms, error_kind,
                                       excerpt, at)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12
         WHERE EXISTS (SELECT 1 FROM webhooks WHERE id = $2)",
        attempt.id,
        attempt.webhook_id,
        attempt.delivery_id,
        attempt.event_id,
        attempt.event_type,
        attempt.attempt,
        attempt.outcome,
        attempt.status_code,
        attempt.latency_ms,
        attempt.error_kind,
        attempt.excerpt,
        attempt.at,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "DELETE FROM webhook_attempts
         WHERE webhook_id = $1 AND id NOT IN (
             SELECT id FROM webhook_attempts WHERE webhook_id = $1
             ORDER BY at DESC, id DESC LIMIT $2
         )",
        attempt.webhook_id,
        LOG_LENGTH,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// A webhook's log, newest first.
pub async fn attempts(
    executor: impl PgExecutor<'_>,
    webhook_id: Uuid,
) -> sqlx::Result<Vec<Attempt>> {
    sqlx::query_as!(
        Attempt,
        "SELECT id, webhook_id, delivery_id, event_id, event_type, attempt, outcome, status_code,
                latency_ms, error_kind, excerpt, at
         FROM webhook_attempts WHERE webhook_id = $1
         ORDER BY at DESC, id DESC
         LIMIT $2",
        webhook_id,
        LOG_LENGTH,
    )
    .fetch_all(executor)
    .await
}

struct ListedRow {
    id: Uuid,
    workspace_id: Uuid,
    user_id: Uuid,
    url: String,
    events: Vec<String>,
    include_own: bool,
    secret: Vec<u8>,
    previous_secret: Option<Vec<u8>>,
    previous_expires_at: Option<OffsetDateTime>,
    disabled_at: Option<OffsetDateTime>,
    disabled_reason: Option<String>,
    failures: i32,
    created_by: Option<Uuid>,
    url_set_by: Option<Uuid>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    kind: String,
    display_name: String,
    hue: Option<i16>,
    owner_id: Option<Uuid>,
    owner_name: Option<String>,
    owner_hue: Option<i16>,
    creator_name: Option<String>,
}

impl TryFrom<ListedRow> for Listed {
    type Error = sqlx::Error;

    fn try_from(row: ListedRow) -> sqlx::Result<Self> {
        let identity = Person::try_from(PersonColumns {
            id: row.user_id,
            kind: row.kind,
            display_name: row.display_name,
            email: None,
            hue: row.hue,
            owner_id: row.owner_id,
            owner_name: row.owner_name,
            owner_hue: row.owner_hue,
        })?;
        Ok(Self {
            creator: row.created_by.zip(row.creator_name),
            webhook: Webhook {
                id: row.id,
                workspace_id: row.workspace_id,
                user_id: row.user_id,
                url: row.url,
                events: row.events,
                include_own: row.include_own,
                secret: row.secret,
                previous_secret: row.previous_secret,
                previous_expires_at: row.previous_expires_at,
                disabled_at: row.disabled_at,
                disabled_reason: row.disabled_reason,
                failures: row.failures,
                created_by: row.created_by,
                url_set_by: row.url_set_by,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            identity,
        })
    }
}
