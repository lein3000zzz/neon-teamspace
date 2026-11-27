//! Requests to join.

use neon_domain::auth::JoinRequest;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// The index an insert breaks when the address has an open request already.
pub const OPEN_FOR_EMAIL: &str = "join_requests_open";

/// The class of the advisory locks on requests to join: "join" in ASCII.
const JOIN_REQUESTS_LOCK: i32 = 0x6a6f_696e;

/// The key of the lock confirmations take, so that each counts those before it.
const CONFIRMING: &str = "confirming";

/// Locks the requests nobody confirmed from `source` until the transaction ends, so that
/// each new one counts those before it.
pub async fn lock_source(executor: impl PgExecutor<'_>, source: &str) -> sqlx::Result<()> {
    lock(executor, &format!("source:{source}")).await
}

/// Locks confirming requests until the transaction ends, so that each confirmation counts
/// those before it.
pub async fn lock_confirming(executor: impl PgExecutor<'_>) -> sqlx::Result<()> {
    lock(executor, CONFIRMING).await
}

async fn lock(executor: impl PgExecutor<'_>, key: &str) -> sqlx::Result<()> {
    sqlx::query_scalar!(
        r#"SELECT true AS "locked!" FROM pg_advisory_xact_lock($1, hashtext($2::text))"#,
        JOIN_REQUESTS_LOCK,
        key,
    )
    .fetch_one(executor)
    .await?;
    Ok(())
}

/// Saves a new request sent from `source`, with its confirmation link's token hash, unless
/// `max_unconfirmed` requests from the source wait to be confirmed already. False when they
/// do. Fails on [`OPEN_FOR_EMAIL`] when the address has an open request. Take
/// [`lock_source`] first.
pub async fn insert_capped(
    executor: impl PgExecutor<'_>,
    request: &JoinRequest,
    email_normalized: &str,
    token_hash: &[u8],
    source: &str,
    max_unconfirmed: i64,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "INSERT INTO join_requests (id, email, email_normalized, note, token_hash,
                                    link_expires_at, mails, mailed_at, created_at, source)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
         WHERE (SELECT count(*) FROM join_requests
                WHERE source = $10 AND confirmed_at IS NULL AND decided_at IS NULL) < $11",
        request.id,
        request.email,
        email_normalized,
        request.note,
        token_hash,
        request.link_expires_at,
        request.mails,
        request.mailed_at,
        request.created_at,
        source,
        max_unconfirmed,
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
/// confirms once, while fewer than `max_open` confirmed requests wait for the admins. `None`
/// when the link was used, expired, or never existed, or when that many wait:
/// [`link_works`] tells which. Take [`lock_confirming`] first.
pub async fn confirm(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
    max_open: i64,
) -> sqlx::Result<Option<JoinRequest>> {
    sqlx::query_as!(
        JoinRequest,
        "UPDATE join_requests SET confirmed_at = $2, token_hash = NULL
         WHERE token_hash = $1 AND confirmed_at IS NULL AND decided_at IS NULL
               AND link_expires_at > $2
               AND (SELECT count(*) FROM join_requests
                    WHERE confirmed_at IS NOT NULL AND decided_at IS NULL) < $3
         RETURNING id, email, note, created_at, confirmed_at, decided_at, decided_by, approved,
                   invite_id, link_expires_at, mails, mailed_at",
        token_hash,
        now,
        max_open,
    )
    .fetch_optional(executor)
    .await
}

/// Whether the confirmation link of a token hash works at `now`.
pub async fn link_works(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM join_requests
               WHERE token_hash = $1 AND confirmed_at IS NULL AND decided_at IS NULL
                     AND link_expires_at > $2
           ) AS "works!""#,
        token_hash,
        now,
    )
    .fetch_one(executor)
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
