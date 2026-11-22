//! API tokens, found by the hash of their secret.

use neon_domain::auth::{ApiToken, Email, Scope, User};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;
use crate::users::{parse_hue, parse_role};

struct Row {
    id: Uuid,
    user_id: Uuid,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: OffsetDateTime,
    expires_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
}

impl TryFrom<Row> for ApiToken {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            user_id: row.user_id,
            name: row.name,
            prefix: row.prefix,
            scopes: parse_scopes(&row.scopes)?,
            created_at: row.created_at,
            expires_at: row.expires_at,
            last_used_at: row.last_used_at,
        })
    }
}

fn parse_scopes(texts: &[String]) -> sqlx::Result<Vec<Scope>> {
    texts
        .iter()
        .map(|text| {
            Scope::parse(text).ok_or_else(|| decode_error(format!("unknown scope {text:?}")))
        })
        .collect()
}

pub async fn insert(
    executor: impl PgExecutor<'_>,
    token: &ApiToken,
    token_hash: &[u8],
) -> sqlx::Result<()> {
    let scopes: Vec<String> = token.scopes.iter().map(|s| s.as_str().to_owned()).collect();
    sqlx::query!(
        "INSERT INTO api_tokens (id, user_id, name, prefix, token_hash, scopes, created_at,
                                 expires_at, last_used_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        token.id,
        token.user_id,
        token.name,
        token.prefix,
        token_hash,
        &scopes,
        token.created_at,
        token.expires_at,
        token.last_used_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The token of a hash with its owner, in one query, as every request with it asks.
pub async fn by_token_hash(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
) -> sqlx::Result<Option<(ApiToken, User)>> {
    sqlx::query_as!(
        Found,
        "SELECT t.id, t.user_id, t.name, t.prefix, t.scopes, t.created_at, t.expires_at,
                t.last_used_at,
                u.email, u.display_name, u.role, u.hue, u.email_verified_at, u.disabled_at,
                u.created_at AS user_created_at, u.updated_at AS user_updated_at
         FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = $1",
        token_hash,
    )
    .fetch_optional(executor)
    .await?
    .map(Found::split)
    .transpose()
}

/// A token by its id with its owner, to tell whether it still holds.
pub async fn by_id(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<(ApiToken, User)>> {
    sqlx::query_as!(
        Found,
        "SELECT t.id, t.user_id, t.name, t.prefix, t.scopes, t.created_at, t.expires_at,
                t.last_used_at,
                u.email, u.display_name, u.role, u.hue, u.email_verified_at, u.disabled_at,
                u.created_at AS user_created_at, u.updated_at AS user_updated_at
         FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Found::split)
    .transpose()
}

/// A token's row with its owner's.
struct Found {
    id: Uuid,
    user_id: Uuid,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: OffsetDateTime,
    expires_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    email: String,
    display_name: String,
    role: String,
    hue: Option<i16>,
    email_verified_at: Option<OffsetDateTime>,
    disabled_at: Option<OffsetDateTime>,
    user_created_at: OffsetDateTime,
    user_updated_at: OffsetDateTime,
}

impl Found {
    fn split(self) -> sqlx::Result<(ApiToken, User)> {
        let user = User {
            id: self.user_id,
            email: Email::from_stored(self.email),
            display_name: self.display_name,
            role: parse_role(&self.role)?,
            hue_override: parse_hue(self.hue)?,
            email_verified_at: self.email_verified_at,
            disabled_at: self.disabled_at,
            created_at: self.user_created_at,
            updated_at: self.user_updated_at,
        };
        let token = ApiToken::try_from(Row {
            id: self.id,
            user_id: self.user_id,
            name: self.name,
            prefix: self.prefix,
            scopes: self.scopes,
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
        })?;
        Ok((token, user))
    }
}

/// A user's tokens, oldest first.
pub async fn list_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<Vec<ApiToken>> {
    sqlx::query_as!(
        Row,
        "SELECT id, user_id, name, prefix, scopes, created_at, expires_at, last_used_at
         FROM api_tokens WHERE user_id = $1 ORDER BY id",
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(ApiToken::try_from)
    .collect()
}

/// Deletes one of a user's tokens. False when the user has none with that id.
pub async fn delete(executor: impl PgExecutor<'_>, user_id: Uuid, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM api_tokens WHERE id = $1 AND user_id = $2",
        id,
        user_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Records when a token was last used.
pub async fn touch(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE api_tokens SET last_used_at = $2 WHERE id = $1",
        id,
        at
    )
    .execute(executor)
    .await?;
    Ok(())
}
