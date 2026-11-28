//! API tokens, found by the hash of their secret: a person's, across the instance or bound
//! to one workspace, or an agent's.

use neon_domain::agents::Standing;
use neon_domain::auth::{ApiToken, Scope, User};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode;
use crate::users::Row as UserRow;

struct Row {
    id: Uuid,
    user_id: Uuid,
    workspace_id: Option<Uuid>,
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
            workspace_id: row.workspace_id,
            name: row.name,
            prefix: row.prefix,
            scopes: parse_scopes(&row.scopes)?,
            created_at: row.created_at,
            expires_at: row.expires_at,
            last_used_at: row.last_used_at,
        })
    }
}

/// The scopes of a `scopes` column, an API token's or an OAuth grant's, code's or token's.
pub(crate) fn parse_scopes(texts: &[String]) -> sqlx::Result<Vec<Scope>> {
    texts
        .iter()
        .map(|text| decode("scope", text, Scope::parse))
        .collect()
}

pub async fn insert(
    executor: impl PgExecutor<'_>,
    token: &ApiToken,
    token_hash: &[u8],
) -> sqlx::Result<()> {
    let scopes: Vec<String> = token.scopes.iter().map(|s| s.as_str().to_owned()).collect();
    sqlx::query!(
        "INSERT INTO api_tokens (id, user_id, workspace_id, name, prefix, token_hash, scopes,
                                 created_at, expires_at, last_used_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        token.id,
        token.user_id,
        token.workspace_id,
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

/// A token, whom it acts as, and, for an agent, what decides whether it works.
#[derive(Debug, Clone)]
pub struct Found {
    pub token: ApiToken,
    pub user: User,
    /// For an agent or integration; for a person it says nothing.
    pub standing: Standing,
}

/// The token of a hash with whom it acts as, in one query, as every request with it asks.
pub async fn by_token_hash(
    executor: impl PgExecutor<'_>,
    token_hash: &[u8],
) -> sqlx::Result<Option<Found>> {
    sqlx::query_as!(
        FoundRow,
        r#"SELECT t.id, t.user_id, t.workspace_id, t.name, t.prefix, t.scopes, t.created_at,
                  t.expires_at, t.last_used_at,
                  u.kind, u.email, u.display_name, u.role, u.hue, u.email_verified_at,
                  u.disabled_at, u.created_at AS user_created_at,
                  u.updated_at AS user_updated_at, u.workspace_id AS user_workspace_id,
                  u.owner_id, u.deleted_at IS NOT NULL AS "deleted!",
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users o ON o.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND m.role <> 'guest' AND o.disabled_at IS NULL
                  ) AS "owner_active!",
                  COALESCE(w.agents_enabled, false) AS "workspace_agents_on!"
           FROM api_tokens t
           JOIN users u ON u.id = t.user_id
           LEFT JOIN workspaces w ON w.id = u.workspace_id
           WHERE t.token_hash = $1"#,
        token_hash,
    )
    .fetch_optional(executor)
    .await?
    .map(FoundRow::split)
    .transpose()
}

/// A token by its id with whom it acts as, to tell whether it still holds.
pub async fn by_id(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Found>> {
    sqlx::query_as!(
        FoundRow,
        r#"SELECT t.id, t.user_id, t.workspace_id, t.name, t.prefix, t.scopes, t.created_at,
                  t.expires_at, t.last_used_at,
                  u.kind, u.email, u.display_name, u.role, u.hue, u.email_verified_at,
                  u.disabled_at, u.created_at AS user_created_at,
                  u.updated_at AS user_updated_at, u.workspace_id AS user_workspace_id,
                  u.owner_id, u.deleted_at IS NOT NULL AS "deleted!",
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users o ON o.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND m.role <> 'guest' AND o.disabled_at IS NULL
                  ) AS "owner_active!",
                  COALESCE(w.agents_enabled, false) AS "workspace_agents_on!"
           FROM api_tokens t
           JOIN users u ON u.id = t.user_id
           LEFT JOIN workspaces w ON w.id = u.workspace_id
           WHERE t.id = $1"#,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(FoundRow::split)
    .transpose()
}

/// A token's row with its user's.
struct FoundRow {
    id: Uuid,
    user_id: Uuid,
    workspace_id: Option<Uuid>,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: OffsetDateTime,
    expires_at: Option<OffsetDateTime>,
    last_used_at: Option<OffsetDateTime>,
    kind: String,
    email: Option<String>,
    display_name: String,
    role: String,
    hue: Option<i16>,
    email_verified_at: Option<OffsetDateTime>,
    disabled_at: Option<OffsetDateTime>,
    user_created_at: OffsetDateTime,
    user_updated_at: OffsetDateTime,
    user_workspace_id: Option<Uuid>,
    owner_id: Option<Uuid>,
    deleted: bool,
    owner_active: bool,
    workspace_agents_on: bool,
}

impl FoundRow {
    fn split(self) -> sqlx::Result<Found> {
        let user = User::try_from(UserRow {
            id: self.user_id,
            kind: self.kind,
            email: self.email,
            display_name: self.display_name,
            role: self.role,
            hue: self.hue,
            email_verified_at: self.email_verified_at,
            disabled_at: self.disabled_at,
            created_at: self.user_created_at,
            updated_at: self.user_updated_at,
            workspace_id: self.user_workspace_id,
            owner_id: self.owner_id,
        })?;
        let token = ApiToken::try_from(Row {
            id: self.id,
            user_id: self.user_id,
            workspace_id: self.workspace_id,
            name: self.name,
            prefix: self.prefix,
            scopes: self.scopes,
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
        })?;
        Ok(Found {
            token,
            user,
            standing: Standing {
                deleted: self.deleted,
                owner_active: self.owner_active,
                workspace_agents_on: self.workspace_agents_on,
            },
        })
    }
}

/// The tokens that act as a person or an agent, oldest first.
pub async fn list_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<Vec<ApiToken>> {
    sqlx::query_as!(
        Row,
        "SELECT id, user_id, workspace_id, name, prefix, scopes, created_at, expires_at,
                last_used_at
         FROM api_tokens WHERE user_id = $1 ORDER BY id",
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(ApiToken::try_from)
    .collect()
}

/// Deletes one of the tokens that act as a person or an agent. False when there is none
/// such.
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

/// Deletes every token that acts as a person or an agent, and says how many there were.
pub async fn delete_for_user(executor: impl PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM api_tokens WHERE user_id = $1", user_id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
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
