//! Requests to join.

use neon_domain::auth::JoinRequest;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// The index an insert breaks when the address has an open request already.
pub const OPEN_FOR_EMAIL: &str = "join_requests_open";

/// Saves a new request with its confirmation link's token hash, unless `max_open` requests
/// are open already. False when they are. Fails on [`OPEN_FOR_EMAIL`] when the address has
/// an open request.
pub async fn insert_capped(
    executor: impl PgExecutor<'_>,
    request: &JoinRequest,
    email_normalized: &str,
    token_hash: &[u8],
    max_open: i64,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "INSERT INTO join_requests (id, email, email_normalized, note, token_hash,
                                    link_expires_at, mails, mailed_at, created_at)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
         WHERE (SELECT count(*) FROM join_requests WHERE decided_at IS NULL) < $10",
        request.id,
        request.email,
        email_normalized,
        request.note,
        token_hash,
        request.link_expires_at,
        request.mails,
        request.mailed_at,
        request.created_at,
        max_open,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The address's undecided request, confirmed or not.
pub async fn open_for(
    executor: impl PgExecutor<'_>,
    email_normalized: &str,
) -> sqlx::Result<Option<JoinRequest>> {
    sqlx::query_as!(
        JoinRequest,
        "SELECT id, email, note, created_at, confirmed_at, decided_at, decided_by, approved,
                invite_id, link_expires_at, mails, mailed_at
         FROM join_requests WHERE email_normalized = $1 AND decided_at IS NULL",
        email_normalized,
    )
    .fetch_optional(executor)
    .await
}

pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<JoinRequest>> {
    sqlx::query_as!(
        JoinRequest,
        "SELECT id, email, note, created_at, confirmed_at, decided_at, decided_by, approved,
                invite_id, link_expires_at, mails, mailed_at
         FROM join_requests WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await
}

/// Whether the address was denied at `since` or later.
pub async fn denied_since(
    executor: impl PgExecutor<'_>,
    email_normalized: &str,
    since: OffsetDateTime,
) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM join_requests
               WHERE email_normalized = $1 AND approved = false AND decided_at >= $2
           ) AS "denied!""#,
        email_normalized,
        since,
    )
    .fetch_one(executor)
    .await
}

/// Gives an unconfirmed request a new link, counting the mail. False when it is confirmed.
pub async fn remail(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    token_hash: &[u8],
    link_expires_at: OffsetDateTime,
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE join_requests
         SET token_hash = $2, link_expires_at = $3, mailed_at = $4, mails = mails + 1
         WHERE id = $1 AND confirmed_at IS NULL AND decided_at IS NULL",
        id,
        token_hash,
        link_expires_at,
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Confirms the request of a link's token hash, in one conditional update so a link
/// confirms once. `None` when the link was used, expired, or never existed.
pub async fn confirm(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
) -> sqlx::Result<Option<JoinRequest>> {
    sqlx::query_as!(
        JoinRequest,
        "UPDATE join_requests SET confirmed_at = $2, token_hash = NULL
         WHERE token_hash = $1 AND confirmed_at IS NULL AND decided_at IS NULL
               AND link_expires_at > $2
         RETURNING id, email, note, created_at, confirmed_at, decided_at, decided_by, approved,
                   invite_id, link_expires_at, mails, mailed_at",
        token_hash,
        now,
    )
    .fetch_optional(executor)
    .await
}

/// The confirmed requests admins see: those waiting first, then those decided at
/// `decided_since` or later, each newest first.
pub async fn list(
    executor: impl PgExecutor<'_>,
    decided_since: OffsetDateTime,
) -> sqlx::Result<Vec<JoinRequest>> {
    sqlx::query_as!(
        JoinRequest,
        "SELECT id, email, note, created_at, confirmed_at, decided_at, decided_by, approved,
                invite_id, link_expires_at, mails, mailed_at
         FROM join_requests
         WHERE confirmed_at IS NOT NULL AND (decided_at IS NULL OR decided_at >= $1)
         ORDER BY decided_at IS NULL DESC, confirmed_at DESC, id DESC",
        decided_since,
    )
    .fetch_all(executor)
    .await
}

/// Decides a confirmed request still waiting. False when it is not waiting.
pub async fn decide(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    approved: bool,
    by: Uuid,
    now: OffsetDateTime,
    invite_id: Option<Uuid>,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE join_requests
         SET decided_at = $4, decided_by = $3, approved = $2, invite_id = $5
         WHERE id = $1 AND confirmed_at IS NOT NULL AND decided_at IS NULL",
        id,
        approved,
        by,
        now,
        invite_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Deletes the requests never confirmed whose last link stopped working before `before`.
pub async fn prune_unconfirmed(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM join_requests
         WHERE confirmed_at IS NULL AND decided_at IS NULL AND link_expires_at < $1",
        before
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the requests decided before `before`.
pub async fn delete_decided(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM join_requests WHERE decided_at < $1", before)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}
