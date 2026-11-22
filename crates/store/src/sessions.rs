//! Browser sessions, found by the hash of their token.

use neon_domain::auth::{Email, Session, User};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::users::parse_role;

/// Saves a new session, if its account still has the password hash `password_hash` and
/// is not disabled, and says whether it did. The account's row is locked for share while
/// it is checked, so a change of password or a disable either commits first, and no
/// session is made, or waits for this one, and ends it with the others.
pub async fn insert(
    executor: impl PgExecutor<'_>,
    session: &Session,
    token_hash: &[u8],
    password_hash: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "INSERT INTO sessions (id, user_id, token_hash, user_agent, ip, created_at, last_seen_at,
                               expires_at, absolute_expires_at)
         SELECT $1, u.id, $3, $4, $5, $6, $7, $8, $9
         FROM users u
         WHERE u.id = $2 AND u.password_hash = $10 AND u.disabled_at IS NULL
         FOR SHARE",
        session.id,
        session.user_id,
        token_hash,
        session.user_agent,
        session.ip,
        session.created_at,
        session.last_seen_at,
        session.expires_at,
        session.absolute_expires_at,
        password_hash,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The session of a token hash with its user, in one query, as every request asks.
pub async fn by_token_hash(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
) -> sqlx::Result<Option<(Session, User)>> {
    let row = sqlx::query!(
        "SELECT s.id, s.user_id, s.user_agent, s.ip, s.created_at, s.last_seen_at, s.expires_at,
                s.absolute_expires_at,
                u.email, u.display_name, u.role, u.email_verified_at, u.disabled_at,
                u.created_at AS user_created_at, u.updated_at AS user_updated_at
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1",
        token_hash,
    )
    .fetch_optional(executor)
    .await?;
    row.map(|row| {
        let user = User {
            id: row.user_id,
            email: Email::from_stored(row.email),
            display_name: row.display_name,
            role: parse_role(&row.role)?,
            email_verified_at: row.email_verified_at,
            disabled_at: row.disabled_at,
            created_at: row.user_created_at,
            updated_at: row.user_updated_at,
        };
        let session = Session {
            id: row.id,
            user_id: row.user_id,
            user_agent: row.user_agent,
            ip: row.ip,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            expires_at: row.expires_at,
            absolute_expires_at: row.absolute_expires_at,
        };
        Ok((session, user))
    })
    .transpose()
}

/// Records a use: when it was, and the idle expiry it slides to.
pub async fn touch(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    last_seen_at: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE sessions SET last_seen_at = $2, expires_at = $3 WHERE id = $1",
        id,
        last_seen_at,
        expires_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The user's sessions not yet expired at `now`, newest first.
pub async fn list_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<Vec<Session>> {
    sqlx::query_as!(
        Session,
        "SELECT id, user_id, user_agent, ip, created_at, last_seen_at, expires_at,
                absolute_expires_at
         FROM sessions
         WHERE user_id = $1 AND expires_at > $2 AND absolute_expires_at > $2
         ORDER BY id DESC",
        user_id,
        now,
    )
    .fetch_all(executor)
    .await
}

/// Ends a session. False when there was none.
pub async fn delete(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM sessions WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Ends one of a user's sessions. False when the user has none with that id.
pub async fn delete_of_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2",
        id,
        user_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Ends every session of a user but `except`, and returns how many ended.
pub async fn delete_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    except: Option<Uuid>,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE user_id = $1 AND ($2::uuid IS NULL OR id <> $2)",
        user_id,
        except,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the sessions past either expiry at `now`.
pub async fn delete_expired(
    executor: impl PgExecutor<'_>,
    now: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE expires_at <= $1 OR absolute_expires_at <= $1",
        now
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
