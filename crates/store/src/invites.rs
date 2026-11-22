//! Instance invites and who signed up with them.

use neon_domain::auth::{Invite, InviteUse};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// The constraint an insert breaks when another invite has the code, in any case.
pub const CODE_TAKEN: &str = "invites_code_hash_key";

struct Row {
    id: Uuid,
    created_by: Option<Uuid>,
    email: Option<String>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    max_uses: i32,
    uses: i32,
    revoked_at: Option<OffsetDateTime>,
    revoked_by: Option<Uuid>,
    code_kept: bool,
}

impl From<Row> for Invite {
    fn from(row: Row) -> Self {
        Self {
            id: row.id,
            created_by: row.created_by,
            email: row.email,
            created_at: row.created_at,
            expires_at: row.expires_at,
            max_uses: row.max_uses,
            uses: row.uses,
            revoked_at: row.revoked_at,
            revoked_by: row.revoked_by,
            code_kept: row.code_kept,
        }
    }
}

/// Saves a new invite with the hash of its code's lookup form and, when a key was there to
/// seal it, the sealed code. Fails on [`CODE_TAKEN`] when another invite has the code.
pub async fn insert(
    executor: impl PgExecutor<'_>,
    invite: &Invite,
    code_hash: &[u8],
    code_sealed: Option<&[u8]>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO invites (id, code_hash, code_sealed, email, max_uses, uses, created_by,
                              created_at, expires_at, revoked_at, revoked_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        invite.id,
        code_hash,
        code_sealed,
        invite.email,
        invite.max_uses,
        invite.uses,
        invite.created_by,
        invite.created_at,
        invite.expires_at,
        invite.revoked_at,
        invite.revoked_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Whether any invite, open or not, has a code of this hash.
pub async fn code_taken(executor: impl PgExecutor<'_>, code_hash: &[u8]) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM invites WHERE code_hash = $1) AS "taken!""#,
        code_hash
    )
    .fetch_one(executor)
    .await
}

/// The invite of a code hash, if it still admits sign-ups at `now`.
pub async fn open_by_code(
    executor: impl PgExecutor<'_>,
    code_hash: &[u8],
    now: OffsetDateTime,
) -> sqlx::Result<Option<Invite>> {
    Ok(sqlx::query_as!(
        Row,
        r#"SELECT id, created_by, email, created_at, expires_at, max_uses, uses, revoked_at,
                  revoked_by, code_sealed IS NOT NULL AS "code_kept!"
           FROM invites
           WHERE code_hash = $1 AND revoked_at IS NULL AND expires_at > $2 AND uses < max_uses"#,
        code_hash,
        now,
    )
    .fetch_optional(executor)
    .await?
    .map(Invite::from))
}

/// Takes one use of an invite for a new account, in one conditional update so sign-ups
/// racing for its last use cannot both have it, then records who took it. False when the
/// invite no longer admits sign-ups at `now`. Run it in the transaction that makes the
/// account.
pub async fn consume(
    executor: &mut sqlx::PgConnection,
    id: Uuid,
    user_id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    let taken = sqlx::query!(
        "UPDATE invites SET uses = uses + 1
         WHERE id = $1 AND revoked_at IS NULL AND expires_at > $2 AND uses < max_uses",
        id,
        now,
    )
    .execute(&mut *executor)
    .await?;
    if taken.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query!(
        "INSERT INTO invite_uses (invite_id, user_id, used_at) VALUES ($1, $2, $3)",
        id,
        user_id,
        now,
    )
    .execute(&mut *executor)
    .await?;
    Ok(true)
}

/// Ends an invite. One revoked before keeps when and by whom. False when there is none.
pub async fn revoke(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    by: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE invites
         SET revoked_at = COALESCE(revoked_at, $3), revoked_by = COALESCE(revoked_by, $2)
         WHERE id = $1",
        id,
        by,
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Every invite, newest first.
pub async fn list(executor: impl PgExecutor<'_>) -> sqlx::Result<Vec<Invite>> {
    Ok(sqlx::query_as!(
        Row,
        r#"SELECT id, created_by, email, created_at, expires_at, max_uses, uses, revoked_at,
                  revoked_by, code_sealed IS NOT NULL AS "code_kept!"
           FROM invites ORDER BY created_at DESC, id DESC"#,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Invite::from)
    .collect())
}

pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Invite>> {
    Ok(sqlx::query_as!(
        Row,
        r#"SELECT id, created_by, email, created_at, expires_at, max_uses, uses, revoked_at,
                  revoked_by, code_sealed IS NOT NULL AS "code_kept!"
           FROM invites WHERE id = $1"#,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Invite::from))
}

/// Every sign-up with an invite, by invite, oldest first, with the account's address while
/// the account exists.
pub async fn uses(executor: impl PgExecutor<'_>) -> sqlx::Result<Vec<(Uuid, InviteUse)>> {
    let rows = sqlx::query!(
        r#"SELECT iu.invite_id, iu.user_id, u.email AS "email?", iu.used_at
           FROM invite_uses iu LEFT JOIN users u ON u.id = iu.user_id
           ORDER BY iu.used_at, iu.invite_id"#
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let signup = InviteUse {
                user_id: row.user_id,
                email: row.email,
                used_at: row.used_at,
            };
            (row.invite_id, signup)
        })
        .collect())
}

/// An invite's sealed code: `None` without such an invite, `Some(None)` when its code was
/// not kept.
pub async fn sealed_code(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<Option<Vec<u8>>>> {
    sqlx::query_scalar!("SELECT code_sealed FROM invites WHERE id = $1", id)
        .fetch_optional(executor)
        .await
}

/// Deletes the invites that expired, or were revoked, before `before`.
pub async fn delete_ended(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM invites WHERE expires_at < $1 OR revoked_at < $1",
        before
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
