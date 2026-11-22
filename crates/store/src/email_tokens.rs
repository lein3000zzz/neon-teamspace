//! Links mailed to accounts, found by the hash of their token.

use neon_domain::auth::EmailTokenPurpose;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

pub async fn insert(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
    user_id: Uuid,
    purpose: EmailTokenPurpose,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
        token_hash,
        user_id,
        purpose.as_str(),
        created_at,
        expires_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// How many links of `purpose` the user got at `since` or later, used or not.
pub async fn count_since(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    purpose: EmailTokenPurpose,
    since: OffsetDateTime,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM email_tokens
           WHERE user_id = $1 AND purpose = $2 AND created_at >= $3"#,
        user_id,
        purpose.as_str(),
        since,
    )
    .fetch_one(executor)
    .await
}

/// Uses the link of a token hash, in one conditional update so it works once, and returns
/// its account. `None` when it was used, expired, is for another purpose, or never existed.
pub async fn take(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
    purpose: EmailTokenPurpose,
    now: OffsetDateTime,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "UPDATE email_tokens SET used_at = $3
         WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > $3
         RETURNING user_id",
        token_hash,
        purpose.as_str(),
        now,
    )
    .fetch_optional(executor)
    .await
}

/// Marks every unused link of `purpose` of the user used, so older links stop working
/// once one of them did its job.
pub async fn use_all(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    purpose: EmailTokenPurpose,
    now: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE email_tokens SET used_at = $3
         WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL",
        user_id,
        purpose.as_str(),
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the links that expired, or were used, before `before`.
pub async fn delete_spent(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM email_tokens WHERE expires_at < $1 OR used_at < $1",
        before
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
