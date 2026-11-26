//! OAuth: the clients that registered themselves, people's grants to them, and the codes
//! and tokens of each grant, found by the SHA-256 of their secret.

use neon_domain::agents::Standing;
use neon_domain::auth::{Scope, User};
use neon_domain::oauth::{Audience, Client, Grant, GrantStanding, TokenKind};
use neon_domain::workspaces::Person;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;
use crate::users::{PersonColumns, Row as UserRow};
use crate::workspaces::parse_role;

fn parse_scopes(texts: &[String]) -> sqlx::Result<Vec<Scope>> {
    texts
        .iter()
        .map(|text| {
            Scope::parse(text).ok_or_else(|| decode_error(format!("unknown scope {text:?}")))
        })
        .collect()
}

fn scope_texts(scopes: &[Scope]) -> Vec<String> {
    scopes
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect()
}

fn parse_audience(text: &str) -> sqlx::Result<Audience> {
    Audience::parse(text).ok_or_else(|| decode_error(format!("unknown audience {text:?}")))
}

/// Keeps a client that registered from `source`, as
/// [`registration_source`](neon_domain::oauth::registration_source) names it.
pub async fn insert_client(
    executor: impl PgExecutor<'_>,
    client: &Client,
    source: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO oauth_clients (id, name, redirect_uris, created_at, last_used_at, source)
         VALUES ($1, $2, $3, $4, $5, $6)",
        client.id,
        client.name,
        &client.redirect_uris,
        client.created_at,
        client.last_used_at,
        source,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn client(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Client>> {
    sqlx::query_as!(
        Client,
        "SELECT id, name, redirect_uris, created_at, last_used_at FROM oauth_clients
         WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await
}

/// Notes that a client was connected, or used a token, at `at`.
pub async fn touch_client(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE oauth_clients SET last_used_at = $2 WHERE id = $1",
        id,
        at
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// How many clients no grant uses: what registering more is bounded by.
pub async fn count_unused_clients(executor: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM oauth_clients c
           WHERE NOT EXISTS (SELECT 1 FROM oauth_grants g WHERE g.client_id = c.id)"#,
    )
    .fetch_one(executor)
    .await
}

/// How many clients no grant uses registered from `source`: what one source registering
/// more is bounded by.
pub async fn count_unused_clients_from(
    executor: impl PgExecutor<'_>,
    source: &str,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM oauth_clients c
           WHERE c.source = $1
             AND NOT EXISTS (SELECT 1 FROM oauth_grants g WHERE g.client_id = c.id)"#,
        source,
    )
    .fetch_one(executor)
    .await
}

/// Removes the clients no grant uses that were neither registered nor used since
/// `before`, and says how many.
pub async fn delete_unused_clients(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM oauth_clients c
         WHERE COALESCE(c.last_used_at, c.created_at) < $1
           AND NOT EXISTS (SELECT 1 FROM oauth_grants g WHERE g.client_id = c.id)",
        before,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// A grant to keep: a new one, or the scopes and audience that replace those of the
/// person's grant to the same client, workspace and identity.
#[derive(Debug, Clone)]
pub struct NewGrant {
    pub id: Uuid,
    pub client_id: Uuid,
    pub person_id: Uuid,
    pub user_id: Uuid,
    pub workspace_id: Uuid,
    pub scopes: Vec<Scope>,
    pub audience: Audience,
    pub at: OffsetDateTime,
}

/// Keeps the grant, replacing the scopes and audience of the person's grant to the same
/// client, workspace and identity if there is one, and gives it as kept.
pub async fn save_grant(executor: impl PgExecutor<'_>, new: &NewGrant) -> sqlx::Result<Grant> {
    let row = sqlx::query_as!(
        GrantRow,
        "INSERT INTO oauth_grants (id, client_id, person_id, user_id, workspace_id, scopes,
                                   audience, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
         ON CONFLICT ON CONSTRAINT oauth_grants_consent DO UPDATE
         SET scopes = EXCLUDED.scopes, audience = EXCLUDED.audience,
             updated_at = EXCLUDED.updated_at
         RETURNING id, client_id, person_id, user_id, workspace_id, scopes, audience,
                   created_at, updated_at, last_used_at",
        new.id,
        new.client_id,
        new.person_id,
        new.user_id,
        new.workspace_id,
        &scope_texts(&new.scopes),
        new.audience.as_str(),
        new.at,
    )
    .fetch_one(executor)
    .await?;
    Grant::try_from(row)
}

struct GrantRow {
    id: Uuid,
    client_id: Uuid,
    person_id: Uuid,
    user_id: Uuid,
    workspace_id: Uuid,
    scopes: Vec<String>,
    audience: String,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    last_used_at: Option<OffsetDateTime>,
}

impl TryFrom<GrantRow> for Grant {
    type Error = sqlx::Error;

    fn try_from(row: GrantRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            client_id: row.client_id,
            person_id: row.person_id,
            user_id: row.user_id,
            workspace_id: row.workspace_id,
            scopes: parse_scopes(&row.scopes)?,
            audience: parse_audience(&row.audience)?,
            created_at: row.created_at,
            updated_at: row.updated_at,
            last_used_at: row.last_used_at,
        })
    }
}

/// Notes that a grant's token was used at `at`.
pub async fn touch_grant(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE oauth_grants SET last_used_at = $2 WHERE id = $1",
        id,
        at
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Deletes a grant, its codes and tokens with it, and gives it as it was: `None` when
/// there was none.
pub async fn delete_grant(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Grant>> {
    sqlx::query_as!(
        GrantRow,
        "DELETE FROM oauth_grants WHERE id = $1
         RETURNING id, client_id, person_id, user_id, workspace_id, scopes, audience,
                   created_at, updated_at, last_used_at",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Grant::try_from)
    .transpose()
}

/// A grant by its id, unchanged.
pub async fn grant(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Grant>> {
    sqlx::query_as!(
        GrantRow,
        "SELECT id, client_id, person_id, user_id, workspace_id, scopes, audience, created_at,
                updated_at, last_used_at
         FROM oauth_grants WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Grant::try_from)
    .transpose()
}

/// Deletes every grant that acts as `user_id`, an agent handed to another owner or
/// deleted, and gives how many there were.
pub async fn delete_grants_of_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM oauth_grants WHERE user_id = $1", user_id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

/// Removes the grants nothing can use any more, none of their codes or tokens unexpired at
/// `now`, last consented to before `before`, and says how many.
pub async fn delete_spent_grants(
    executor: impl PgExecutor<'_>,
    now: OffsetDateTime,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM oauth_grants g
         WHERE g.updated_at < $2
           AND NOT EXISTS (SELECT 1 FROM oauth_tokens t
                           WHERE t.grant_id = g.id AND t.expires_at > $1)
           AND NOT EXISTS (SELECT 1 FROM oauth_codes c
                           WHERE c.grant_id = g.id AND c.expires_at > $1)",
        now,
        before,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// A grant as the lists of connected apps show it: its client's name, its workspace's,
/// whom it acts as, and who consented.
#[derive(Debug, Clone)]
pub struct Listed {
    pub grant: Grant,
    pub client_name: String,
    pub workspace_name: String,
    pub workspace_slug: String,
    pub identity: Person,
    pub person_name: String,
}

/// The grants a person consented to, as themselves or as their agents, last used first.
pub async fn grants_of_person(
    executor: impl PgExecutor<'_>,
    person_id: Uuid,
) -> sqlx::Result<Vec<Listed>> {
    sqlx::query_as!(
        ListedRow,
        r#"SELECT g.id, g.client_id, g.person_id, g.user_id, g.workspace_id, g.scopes,
                  g.audience, g.created_at, g.updated_at, g.last_used_at,
                  c.name AS client_name, w.name AS workspace_name, w.slug AS workspace_slug,
                  u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  p.display_name AS person_name
           FROM oauth_grants g
           JOIN oauth_clients c ON c.id = g.client_id
           JOIN workspaces w ON w.id = g.workspace_id
           JOIN users u ON u.id = g.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           JOIN users p ON p.id = g.person_id
           WHERE g.person_id = $1
           ORDER BY COALESCE(g.last_used_at, g.updated_at) DESC, g.id"#,
        person_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Listed::try_from)
    .collect()
}

/// The grants that act as agents of a workspace, last used first.
pub async fn agent_grants_of_workspace(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<Listed>> {
    sqlx::query_as!(
        ListedRow,
        r#"SELECT g.id, g.client_id, g.person_id, g.user_id, g.workspace_id, g.scopes,
                  g.audience, g.created_at, g.updated_at, g.last_used_at,
                  c.name AS client_name, w.name AS workspace_name, w.slug AS workspace_slug,
                  u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  p.display_name AS person_name
           FROM oauth_grants g
           JOIN oauth_clients c ON c.id = g.client_id
           JOIN workspaces w ON w.id = g.workspace_id
           JOIN users u ON u.id = g.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           JOIN users p ON p.id = g.person_id
           WHERE g.workspace_id = $1 AND g.user_id <> g.person_id
           ORDER BY COALESCE(g.last_used_at, g.updated_at) DESC, g.id"#,
        workspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Listed::try_from)
    .collect()
}

struct ListedRow {
    id: Uuid,
    client_id: Uuid,
    person_id: Uuid,
    user_id: Uuid,
    workspace_id: Uuid,
    scopes: Vec<String>,
    audience: String,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    last_used_at: Option<OffsetDateTime>,
    client_name: String,
    workspace_name: String,
    workspace_slug: String,
    kind: String,
    display_name: String,
    email: Option<String>,
    hue: Option<i16>,
    owner_id: Option<Uuid>,
    owner_name: Option<String>,
    owner_hue: Option<i16>,
    person_name: String,
}

impl TryFrom<ListedRow> for Listed {
    type Error = sqlx::Error;

    fn try_from(row: ListedRow) -> sqlx::Result<Self> {
        let identity = Person::try_from(PersonColumns {
            id: row.user_id,
            kind: row.kind,
            display_name: row.display_name,
            email: row.email,
            hue: row.hue,
            owner_id: row.owner_id,
            owner_name: row.owner_name,
            owner_hue: row.owner_hue,
        })?;
        let grant = Grant::try_from(GrantRow {
            id: row.id,
            client_id: row.client_id,
            person_id: row.person_id,
            user_id: row.user_id,
            workspace_id: row.workspace_id,
            scopes: row.scopes,
            audience: row.audience,
            created_at: row.created_at,
            updated_at: row.updated_at,
            last_used_at: row.last_used_at,
        })?;
        Ok(Self {
            grant,
            client_name: row.client_name,
            workspace_name: row.workspace_name,
            workspace_slug: row.workspace_slug,
            identity,
            person_name: row.person_name,
        })
    }
}

/// An authorization code, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub grant_id: Uuid,
    pub redirect_uri: String,
    pub challenge: String,
    pub scopes: Vec<Scope>,
    pub audience: Audience,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    /// Set once it was exchanged.
    pub used_at: Option<OffsetDateTime>,
}

pub async fn insert_code(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
    code: &Code,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO oauth_codes (hash, grant_id, redirect_uri, challenge, scopes, audience,
                                  created_at, expires_at, used_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        hash,
        code.grant_id,
        code.redirect_uri,
        code.challenge,
        &scope_texts(&code.scopes),
        code.audience.as_str(),
        code.created_at,
        code.expires_at,
        code.used_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Locks the grant of the code of a hash until the transaction ends, and gives its id:
/// `None` when there is no such code, or its grant is gone.
///
/// Whatever changes a grant together with its codes and tokens locks the grant first:
/// deleting it does, and so do exchanging a code and a refresh token, with this and
/// [`lock_grant_of_refresh`], before they lock the code or token. Each then waits for the
/// other, and none waits for one that waits for it.
pub async fn lock_grant_of_code(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "SELECT g.id FROM oauth_grants g
         WHERE g.id = (SELECT c.grant_id FROM oauth_codes c WHERE c.hash = $1)
         FOR UPDATE",
        hash,
    )
    .fetch_optional(executor)
    .await
}

/// Locks the grant of the refresh token of a hash until the transaction ends, as
/// [`lock_grant_of_code`] does a code's.
pub async fn lock_grant_of_refresh(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "SELECT g.id FROM oauth_grants g
         WHERE g.id = (SELECT t.grant_id FROM oauth_tokens t
                       WHERE t.hash = $1 AND t.kind = 'refresh')
         FOR UPDATE",
        hash,
    )
    .fetch_optional(executor)
    .await
}

/// A code by its hash, locked until the transaction ends, so that two exchanges of it
/// take turns and the second finds it used. Lock its grant first, with
/// [`lock_grant_of_code`].
pub async fn lock_code(executor: impl PgExecutor<'_>, hash: &[u8]) -> sqlx::Result<Option<Code>> {
    sqlx::query!(
        "SELECT grant_id, redirect_uri, challenge, scopes, audience, created_at, expires_at,
                used_at
         FROM oauth_codes WHERE hash = $1 FOR UPDATE",
        hash,
    )
    .fetch_optional(executor)
    .await?
    .map(|row| {
        Ok(Code {
            grant_id: row.grant_id,
            redirect_uri: row.redirect_uri,
            challenge: row.challenge,
            scopes: parse_scopes(&row.scopes)?,
            audience: parse_audience(&row.audience)?,
            created_at: row.created_at,
            expires_at: row.expires_at,
            used_at: row.used_at,
        })
    })
    .transpose()
}

/// Marks a code exchanged at `at`.
pub async fn use_code(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE oauth_codes SET used_at = $2 WHERE hash = $1",
        hash,
        at
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// An access or refresh token, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub id: Uuid,
    pub grant_id: Uuid,
    pub kind: TokenKind,
    pub scopes: Vec<Scope>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    /// Set once a refresh token was exchanged.
    pub used_at: Option<OffsetDateTime>,
}

pub async fn insert_token(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
    token: &Token,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO oauth_tokens (id, hash, grant_id, kind, scopes, created_at, expires_at,
                                   used_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        token.id,
        hash,
        token.grant_id,
        token.kind.as_str(),
        &scope_texts(&token.scopes),
        token.created_at,
        token.expires_at,
        token.used_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

struct TokenRow {
    id: Uuid,
    grant_id: Uuid,
    kind: String,
    scopes: Vec<String>,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    used_at: Option<OffsetDateTime>,
}

impl TryFrom<TokenRow> for Token {
    type Error = sqlx::Error;

    fn try_from(row: TokenRow) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            grant_id: row.grant_id,
            kind: TokenKind::parse(&row.kind)
                .ok_or_else(|| decode_error(format!("unknown token kind {:?}", row.kind)))?,
            scopes: parse_scopes(&row.scopes)?,
            created_at: row.created_at,
            expires_at: row.expires_at,
            used_at: row.used_at,
        })
    }
}

/// A refresh token by its hash, locked until the transaction ends, as [`lock_code`] locks
/// codes. Lock its grant first, with [`lock_grant_of_refresh`].
pub async fn lock_refresh(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
) -> sqlx::Result<Option<Token>> {
    sqlx::query_as!(
        TokenRow,
        "SELECT id, grant_id, kind, scopes, created_at, expires_at, used_at
         FROM oauth_tokens WHERE hash = $1 AND kind = 'refresh' FOR UPDATE",
        hash,
    )
    .fetch_optional(executor)
    .await?
    .map(Token::try_from)
    .transpose()
}

/// Marks a refresh token exchanged at `at`. It stays until it expires, so that a second
/// use is caught.
pub async fn use_token(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!("UPDATE oauth_tokens SET used_at = $2 WHERE id = $1", id, at)
        .execute(executor)
        .await?;
    Ok(())
}

/// A token of either kind by its hash.
pub async fn token(executor: impl PgExecutor<'_>, hash: &[u8]) -> sqlx::Result<Option<Token>> {
    sqlx::query_as!(
        TokenRow,
        "SELECT id, grant_id, kind, scopes, created_at, expires_at, used_at
         FROM oauth_tokens WHERE hash = $1",
        hash,
    )
    .fetch_optional(executor)
    .await?
    .map(Token::try_from)
    .transpose()
}

/// Removes the codes that expired before `before`, and says how many.
pub async fn delete_expired_codes(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM oauth_codes WHERE expires_at < $1", before)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

/// Removes the tokens that expired before `before`, and says how many.
pub async fn delete_expired_tokens(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM oauth_tokens WHERE expires_at < $1", before)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

/// A grant with whom it acts as, its client's name, and what decides whether it works.
#[derive(Debug, Clone)]
pub struct FoundGrant {
    pub grant: Grant,
    pub client_name: String,
    /// Whom it acts as.
    pub user: User,
    pub standing: GrantStanding,
}

/// An access token with its grant, as each request with it needs.
#[derive(Debug, Clone)]
pub struct FoundAccess {
    pub token: Token,
    pub found: FoundGrant,
}

/// The access token of a hash with its grant, in one query.
pub async fn access_by_hash(
    executor: impl PgExecutor<'_>,
    hash: &[u8],
) -> sqlx::Result<Option<FoundAccess>> {
    sqlx::query_as!(
        AccessRow,
        r#"SELECT t.id AS token_id, t.kind AS token_kind, t.scopes AS token_scopes,
                  t.created_at AS token_created_at, t.expires_at AS token_expires_at,
                  t.used_at AS token_used_at,
                  g.id, g.client_id, g.person_id, g.user_id, g.workspace_id, g.scopes,
                  g.audience, g.created_at, g.updated_at, g.last_used_at,
                  c.name AS client_name,
                  u.kind, u.email, u.display_name, u.role, u.hue, u.email_verified_at,
                  u.disabled_at, u.created_at AS user_created_at,
                  u.updated_at AS user_updated_at, u.workspace_id AS user_workspace_id,
                  u.owner_id, u.deleted_at IS NOT NULL AS "deleted!",
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users o ON o.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND m.role <> 'guest' AND o.disabled_at IS NULL
                  ) AS "owner_active!",
                  COALESCE(w.agents_enabled, false) AS "workspace_agents_on!",
                  p.disabled_at IS NOT NULL AS "person_disabled!",
                  pm.role AS "person_role?"
           FROM oauth_tokens t
           JOIN oauth_grants g ON g.id = t.grant_id
           JOIN oauth_clients c ON c.id = g.client_id
           JOIN users u ON u.id = g.user_id
           JOIN users p ON p.id = g.person_id
           LEFT JOIN workspaces w ON w.id = u.workspace_id
           LEFT JOIN workspace_members pm
                  ON pm.workspace_id = g.workspace_id AND pm.user_id = g.person_id
           WHERE t.hash = $1 AND t.kind = 'access'"#,
        hash,
    )
    .fetch_optional(executor)
    .await?
    .map(AccessRow::split)
    .transpose()
}

/// The access token of an id with its grant, to tell whether it still holds.
pub async fn access_by_id(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<FoundAccess>> {
    sqlx::query_as!(
        AccessRow,
        r#"SELECT t.id AS token_id, t.kind AS token_kind, t.scopes AS token_scopes,
                  t.created_at AS token_created_at, t.expires_at AS token_expires_at,
                  t.used_at AS token_used_at,
                  g.id, g.client_id, g.person_id, g.user_id, g.workspace_id, g.scopes,
                  g.audience, g.created_at, g.updated_at, g.last_used_at,
                  c.name AS client_name,
                  u.kind, u.email, u.display_name, u.role, u.hue, u.email_verified_at,
                  u.disabled_at, u.created_at AS user_created_at,
                  u.updated_at AS user_updated_at, u.workspace_id AS user_workspace_id,
                  u.owner_id, u.deleted_at IS NOT NULL AS "deleted!",
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users o ON o.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND m.role <> 'guest' AND o.disabled_at IS NULL
                  ) AS "owner_active!",
                  COALESCE(w.agents_enabled, false) AS "workspace_agents_on!",
                  p.disabled_at IS NOT NULL AS "person_disabled!",
                  pm.role AS "person_role?"
           FROM oauth_tokens t
           JOIN oauth_grants g ON g.id = t.grant_id
           JOIN oauth_clients c ON c.id = g.client_id
           JOIN users u ON u.id = g.user_id
           JOIN users p ON p.id = g.person_id
           LEFT JOIN workspaces w ON w.id = u.workspace_id
           LEFT JOIN workspace_members pm
                  ON pm.workspace_id = g.workspace_id AND pm.user_id = g.person_id
           WHERE t.id = $1 AND t.kind = 'access'"#,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(AccessRow::split)
    .transpose()
}

/// A grant with whom it acts as and what decides whether it works, for the token
/// endpoint.
pub async fn found_grant(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<FoundGrant>> {
    sqlx::query_as!(
        GrantStandingRow,
        r#"SELECT g.id, g.client_id, g.person_id, g.user_id, g.workspace_id, g.scopes,
                  g.audience, g.created_at, g.updated_at, g.last_used_at,
                  c.name AS client_name,
                  u.kind, u.email, u.display_name, u.role, u.hue, u.email_verified_at,
                  u.disabled_at, u.created_at AS user_created_at,
                  u.updated_at AS user_updated_at, u.workspace_id AS user_workspace_id,
                  u.owner_id, u.deleted_at IS NOT NULL AS "deleted!",
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users o ON o.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND m.role <> 'guest' AND o.disabled_at IS NULL
                  ) AS "owner_active!",
                  COALESCE(w.agents_enabled, false) AS "workspace_agents_on!",
                  p.disabled_at IS NOT NULL AS "person_disabled!",
                  pm.role AS "person_role?"
           FROM oauth_grants g
           JOIN oauth_clients c ON c.id = g.client_id
           JOIN users u ON u.id = g.user_id
           JOIN users p ON p.id = g.person_id
           LEFT JOIN workspaces w ON w.id = u.workspace_id
           LEFT JOIN workspace_members pm
                  ON pm.workspace_id = g.workspace_id AND pm.user_id = g.person_id
           WHERE g.id = $1"#,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(GrantStandingRow::split)
    .transpose()
}

/// A grant's row with whom it acts as and what decides whether it works.
struct GrantStandingRow {
    id: Uuid,
    client_id: Uuid,
    person_id: Uuid,
    user_id: Uuid,
    workspace_id: Uuid,
    scopes: Vec<String>,
    audience: String,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    last_used_at: Option<OffsetDateTime>,
    client_name: String,
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
    person_disabled: bool,
    person_role: Option<String>,
}

impl GrantStandingRow {
    fn split(self) -> sqlx::Result<FoundGrant> {
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
        let grant = Grant::try_from(GrantRow {
            id: self.id,
            client_id: self.client_id,
            person_id: self.person_id,
            user_id: self.user_id,
            workspace_id: self.workspace_id,
            scopes: self.scopes,
            audience: self.audience,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_used_at: self.last_used_at,
        })?;
        let standing = GrantStanding {
            person_disabled: self.person_disabled,
            person_role: self.person_role.as_deref().map(parse_role).transpose()?,
            kind: user.kind,
            owner_id: user.owner_id,
            identity_workspace_id: user.workspace_id,
            agent: Standing {
                deleted: self.deleted,
                owner_active: self.owner_active,
                workspace_agents_on: self.workspace_agents_on,
            },
        };
        Ok(FoundGrant {
            grant,
            client_name: self.client_name,
            user,
            standing,
        })
    }
}

/// An access token's row with its grant's.
struct AccessRow {
    token_id: Uuid,
    token_kind: String,
    token_scopes: Vec<String>,
    token_created_at: OffsetDateTime,
    token_expires_at: OffsetDateTime,
    token_used_at: Option<OffsetDateTime>,
    id: Uuid,
    client_id: Uuid,
    person_id: Uuid,
    user_id: Uuid,
    workspace_id: Uuid,
    scopes: Vec<String>,
    audience: String,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    last_used_at: Option<OffsetDateTime>,
    client_name: String,
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
    person_disabled: bool,
    person_role: Option<String>,
}

impl AccessRow {
    fn split(self) -> sqlx::Result<FoundAccess> {
        let token = Token::try_from(TokenRow {
            id: self.token_id,
            grant_id: self.id,
            kind: self.token_kind,
            scopes: self.token_scopes,
            created_at: self.token_created_at,
            expires_at: self.token_expires_at,
            used_at: self.token_used_at,
        })?;
        let found = GrantStandingRow {
            id: self.id,
            client_id: self.client_id,
            person_id: self.person_id,
            user_id: self.user_id,
            workspace_id: self.workspace_id,
            scopes: self.scopes,
            audience: self.audience,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_used_at: self.last_used_at,
            client_name: self.client_name,
            kind: self.kind,
            email: self.email,
            display_name: self.display_name,
            role: self.role,
            hue: self.hue,
            email_verified_at: self.email_verified_at,
            disabled_at: self.disabled_at,
            user_created_at: self.user_created_at,
            user_updated_at: self.user_updated_at,
            user_workspace_id: self.user_workspace_id,
            owner_id: self.owner_id,
            deleted: self.deleted,
            owner_active: self.owner_active,
            workspace_agents_on: self.workspace_agents_on,
            person_disabled: self.person_disabled,
            person_role: self.person_role,
        }
        .split()?;
        Ok(FoundAccess { token, found })
    }
}
