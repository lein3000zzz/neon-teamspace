//! Each account's favorite pages, in the order it keeps them. Changes to an account's
//! favorites take [`lock`] first, so each sees the order the one before left.

use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// The class of the advisory locks on favorites, one per account: "favs" in ASCII.
const FAVORITES_LOCK: i32 = 0x6661_7673;

/// The constraint an insert breaks when the page is a favorite already.
pub const ALREADY_FAVORITE: &str = "favorites_pkey";

/// Locks an account's favorites until the transaction ends.
pub async fn lock(executor: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<()> {
    sqlx::query_scalar!(
        r#"SELECT true AS "locked!" FROM pg_advisory_xact_lock($1, hashtext($2::text))"#,
        FAVORITES_LOCK,
        user_id.to_string(),
    )
    .fetch_one(executor)
    .await?;
    Ok(())
}

/// Adds a favorite. Fails on [`ALREADY_FAVORITE`], and on the foreign key when the account
/// is not in the page's workspace.
pub async fn insert(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    page_id: Uuid,
    workspace_id: Uuid,
    position: &str,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO favorites (user_id, page_id, workspace_id, position, created_at)
         VALUES ($1, $2, $3, $4, $5)",
        user_id,
        page_id,
        workspace_id,
        position,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Takes a page off an account's favorites, if it is among those of `workspace_id` when
/// that is set, and says whether it was.
pub async fn delete(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    page_id: Uuid,
    workspace_id: Option<Uuid>,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM favorites
         WHERE user_id = $1 AND page_id = $2 AND ($3::uuid IS NULL OR workspace_id = $3)",
        user_id,
        page_id,
        workspace_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The pages an account keeps as favorites and their positions, in order, in every
/// workspace or in one, but for `except`.
pub async fn positions(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    workspace_id: Option<Uuid>,
    except: Option<Uuid>,
) -> sqlx::Result<Vec<(Uuid, String)>> {
    Ok(sqlx::query!(
        "SELECT page_id, position FROM favorites
         WHERE user_id = $1 AND ($2::uuid IS NULL OR workspace_id = $2)
           AND ($3::uuid IS NULL OR page_id <> $3)
         ORDER BY position, page_id",
        user_id,
        workspace_id,
        except,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.page_id, row.position))
    .collect())
}

/// Gives each of an account's favorites `pages` the position at the same index of
/// `positions`.
pub async fn set_positions(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
    pages: &[Uuid],
    positions: &[String],
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE favorites f SET position = u.position
         FROM UNNEST($2::uuid[], $3::text[]) AS u (page_id, position)
         WHERE f.user_id = $1 AND f.page_id = u.page_id",
        user_id,
        pages,
        positions,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
