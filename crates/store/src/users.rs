//! Accounts, and the agents and integrations that are users too. Everything here about
//! signing in, addresses, passwords, verification and instance roles reads people alone.

use neon_domain::agents::{AgentOwner, Standing};
use neon_domain::auth::{Email, Role, User, UserKind};
use neon_domain::hue::MAX_HUE;
use neon_domain::workspaces::Person;
use serde_json::Value as Json;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;

/// The constraint an insert breaks when an account has the address already.
pub const EMAIL_TAKEN: &str = "users_email_normalized_key";

/// A `users` row without the password hash and the preferences.
pub(crate) struct Row {
    pub(crate) id: Uuid,
    pub(crate) kind: String,
    pub(crate) email: Option<String>,
    pub(crate) display_name: String,
    pub(crate) role: String,
    pub(crate) hue: Option<i16>,
    pub(crate) email_verified_at: Option<OffsetDateTime>,
    pub(crate) disabled_at: Option<OffsetDateTime>,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
    pub(crate) workspace_id: Option<Uuid>,
    pub(crate) owner_id: Option<Uuid>,
}

impl TryFrom<Row> for User {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            kind: parse_kind(&row.kind)?,
            email: row.email.map(Email::from_stored),
            display_name: row.display_name,
            role: parse_role(&row.role)?,
            hue_override: parse_hue(row.hue)?,
            email_verified_at: row.email_verified_at,
            disabled_at: row.disabled_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            workspace_id: row.workspace_id,
            owner_id: row.owner_id,
        })
    }
}

pub(crate) fn parse_role(text: &str) -> sqlx::Result<Role> {
    Role::parse(text).ok_or_else(|| decode_error(format!("unknown role {text:?}")))
}

pub(crate) fn parse_kind(text: &str) -> sqlx::Result<UserKind> {
    UserKind::parse(text).ok_or_else(|| decode_error(format!("unknown kind {text:?}")))
}

/// What lists read of an identity, with its owner's, for an agent or integration: select
/// `u.id, u.kind, u.display_name, u.email, u.hue` and the owner's `o.id`, `o.display_name`
/// and `o.hue`, joining `users o ON o.id = u.owner_id`.
pub(crate) struct PersonColumns {
    pub(crate) id: Uuid,
    pub(crate) kind: String,
    pub(crate) display_name: String,
    pub(crate) email: Option<String>,
    pub(crate) hue: Option<i16>,
    pub(crate) owner_id: Option<Uuid>,
    pub(crate) owner_name: Option<String>,
    pub(crate) owner_hue: Option<i16>,
}

impl TryFrom<PersonColumns> for Person {
    type Error = sqlx::Error;

    fn try_from(columns: PersonColumns) -> sqlx::Result<Self> {
        Ok(Self {
            id: columns.id,
            kind: parse_kind(&columns.kind)?,
            display_name: columns.display_name,
            email: columns.email,
            hue_override: parse_hue(columns.hue)?,
            owner: owner(columns.owner_id, columns.owner_name, columns.owner_hue)?,
        })
    }
}

/// An agent's or integration's owner from an outer join's columns: none without one.
pub(crate) fn owner(
    id: Option<Uuid>,
    name: Option<String>,
    hue: Option<i16>,
) -> sqlx::Result<Option<AgentOwner>> {
    match (id, name) {
        (Some(id), Some(display_name)) => Ok(Some(AgentOwner {
            id,
            display_name,
            hue_override: parse_hue(hue)?,
        })),
        _ => Ok(None),
    }
}

/// A pinned hue as stored: a check constraint keeps it within 0 to 359.
pub(crate) fn parse_hue(hue: Option<i16>) -> sqlx::Result<Option<u16>> {
    hue.map(|hue| {
        u16::try_from(hue)
            .ok()
            .filter(|hue| *hue <= MAX_HUE)
            .ok_or_else(|| decode_error(format!("hue {hue} is out of range")))
    })
    .transpose()
}

/// A pinned hue to store.
pub(crate) fn stored_hue(hue: Option<u16>) -> Option<i16> {
    hue.and_then(|hue| i16::try_from(hue).ok())
}

/// Saves a new person's account. Fails on [`EMAIL_TAKEN`] when an account has its address.
/// Agents and integrations are made with [`agents::insert`](crate::agents::insert).
pub async fn insert(
    executor: impl PgExecutor<'_>,
    user: &User,
    email: &Email,
    password_hash: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO users (id, kind, email, email_normalized, display_name, password_hash,
                            role, hue, email_verified_at, disabled_at, created_at, updated_at)
         VALUES ($1, 'human', $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        user.id,
        email.as_str(),
        email.normalized(),
        user.display_name,
        password_hash,
        user.role.as_str(),
        stored_hue(user.hue_override),
        user.email_verified_at,
        user.disabled_at,
        user.created_at,
        user.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// The person of a normalized address, with their password hash. Only people have
/// addresses.
pub async fn by_email(
    executor: impl PgExecutor<'_>,
    email_normalized: &str,
) -> sqlx::Result<Option<(User, String)>> {
    let row = sqlx::query!(
        r#"SELECT id, kind, email, display_name, role, hue, email_verified_at, disabled_at,
                  created_at, updated_at, workspace_id, owner_id,
                  password_hash AS "password_hash!"
           FROM users WHERE email_normalized = $1 AND kind = 'human'"#,
        email_normalized,
    )
    .fetch_optional(executor)
    .await?;
    row.map(|row| {
        let user = User::try_from(Row {
            id: row.id,
            kind: row.kind,
            email: row.email,
            display_name: row.display_name,
            role: row.role,
            hue: row.hue,
            email_verified_at: row.email_verified_at,
            disabled_at: row.disabled_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            workspace_id: row.workspace_id,
            owner_id: row.owner_id,
        })?;
        Ok((user, row.password_hash))
    })
    .transpose()
}

/// A person's account by id: never an agent or integration, which have none.
pub async fn by_id(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<User>> {
    sqlx::query_as!(
        Row,
        "SELECT id, kind, email, display_name, role, hue, email_verified_at, disabled_at,
                created_at, updated_at, workspace_id, owner_id
         FROM users WHERE id = $1 AND kind = 'human'",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(User::try_from)
    .transpose()
}

/// An identity of any kind by its id: a person, or an agent or integration with what
/// decides whether it works, as [`Standing`] reads it. A person's standing is always one
/// that works; whether their account is disabled is on the user.
pub async fn identity(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<(User, Standing)>> {
    let Some(row) = sqlx::query!(
        r#"SELECT u.id, u.kind, u.email, u.display_name, u.role, u.hue, u.email_verified_at,
                  u.disabled_at, u.created_at, u.updated_at, u.workspace_id, u.owner_id,
                  u.deleted_at IS NOT NULL AS "deleted!",
                  (u.kind = 'human' OR EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users o ON o.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND m.role <> 'guest' AND o.disabled_at IS NULL
                  )) AS "owner_active!",
                  COALESCE(w.agents_enabled, true) AS "workspace_agents_on!"
           FROM users u
           LEFT JOIN workspaces w ON w.id = u.workspace_id
           WHERE u.id = $1"#,
        id,
    )
    .fetch_optional(executor)
    .await?
    else {
        return Ok(None);
    };
    let user = User::try_from(Row {
        id: row.id,
        kind: row.kind,
        email: row.email,
        display_name: row.display_name,
        role: row.role,
        hue: row.hue,
        email_verified_at: row.email_verified_at,
        disabled_at: row.disabled_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
        workspace_id: row.workspace_id,
        owner_id: row.owner_id,
    })?;
    let standing = Standing {
        deleted: row.deleted,
        owner_active: row.owner_active,
        workspace_agents_on: row.workspace_agents_on,
    };
    Ok(Some((user, standing)))
}

/// Like [`by_id`], locking the row until the transaction ends.
pub async fn by_id_for_update(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<User>> {
    sqlx::query_as!(
        Row,
        "SELECT id, kind, email, display_name, role, hue, email_verified_at, disabled_at,
                created_at, updated_at, workspace_id, owner_id
         FROM users WHERE id = $1 AND kind = 'human' FOR UPDATE",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(User::try_from)
    .transpose()
}

/// People's accounts in id order, which is the order they were made in, starting after
/// `after`.
pub async fn list(
    executor: impl PgExecutor<'_>,
    after: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<Vec<User>> {
    sqlx::query_as!(
        Row,
        "SELECT id, kind, email, display_name, role, hue, email_verified_at, disabled_at,
                created_at, updated_at, workspace_id, owner_id
         FROM users WHERE kind = 'human' AND ($1::uuid IS NULL OR id > $1)
         ORDER BY id LIMIT $2",
        after,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(User::try_from)
    .collect()
}

/// A person's password hash; `None` for anyone else.
pub async fn password_hash(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<String>> {
    Ok(
        sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = $1", id)
            .fetch_optional(executor)
            .await?
            .flatten(),
    )
}

/// Replaces the password hash `old` with `new`, the same password hashed again, and says
/// whether it did: not when the password changed since `old` was read.
pub async fn upgrade_password_hash(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    old: &str,
    new: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE users SET password_hash = $3 WHERE id = $1 AND password_hash = $2",
        id,
        old,
        new,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn set_password_hash(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    password_hash: &str,
    now: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET password_hash = $2, updated_at = $3 WHERE id = $1",
        id,
        password_hash,
        now,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Marks the address verified at `at`, unless it is already.
pub async fn set_email_verified(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, $2), updated_at = $2
         WHERE id = $1",
        id,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Changes a person's name, their pinned hue, or both, and returns them as they are now: each
/// part `None` stays as it is, and `Some(None)` unpins the hue. Only these and the time are
/// written, so a change that started before an admin's change to the role or the disable
/// cannot undo it. `None` when there is no such account.
pub async fn update_profile(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    display_name: Option<&str>,
    hue: Option<Option<u16>>,
    now: OffsetDateTime,
) -> sqlx::Result<Option<User>> {
    sqlx::query_as!(
        Row,
        "UPDATE users
         SET display_name = COALESCE($2, display_name),
             hue = CASE WHEN $3 THEN $4 ELSE hue END,
             updated_at = $5
         WHERE id = $1 AND kind = 'human'
         RETURNING id, kind, email, display_name, role, hue, email_verified_at, disabled_at,
                   created_at, updated_at, workspace_id, owner_id",
        id,
        display_name,
        hue.is_some(),
        stored_hue(hue.flatten()),
        now,
    )
    .fetch_optional(executor)
    .await?
    .map(User::try_from)
    .transpose()
}

/// Saves what an admin changes: the role and whether the account is disabled, with the
/// name as it was read. Call it on a row read with [`by_id_for_update`] after
/// [`lock_active_admins`], in the same transaction, so nothing changed it in between.
pub async fn update(executor: impl PgExecutor<'_>, user: &User) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET display_name = $2, role = $3, disabled_at = $4, updated_at = $5
         WHERE id = $1 AND kind = 'human'",
        user.id,
        user.display_name,
        user.role.as_str(),
        user.disabled_at,
        user.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Locks every admin who can sign in, until the transaction ends, and returns their ids.
/// Changes that could leave the instance without one take this lock first, so they run
/// one at a time and each sees the admins as the one before left them.
pub async fn lock_active_admins(executor: impl PgExecutor<'_>) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT id FROM users
         WHERE role = 'admin' AND disabled_at IS NULL AND kind = 'human'
         ORDER BY id FOR UPDATE"
    )
    .fetch_all(executor)
    .await
}

pub async fn count_active_admins(executor: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM users
           WHERE role = 'admin' AND disabled_at IS NULL AND kind = 'human'"#
    )
    .fetch_one(executor)
    .await
}

/// The addresses of the admins who can sign in, for notices meant for them.
pub async fn active_admin_emails(executor: impl PgExecutor<'_>) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        r#"SELECT email AS "email!" FROM users
           WHERE role = 'admin' AND disabled_at IS NULL AND kind = 'human'
           ORDER BY id"#
    )
    .fetch_all(executor)
    .await
}

/// Deletes a person's account that never verified its address. False when there is none
/// such.
pub async fn delete_unverified(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM users WHERE id = $1 AND email_verified_at IS NULL AND kind = 'human'",
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Deletes the people's accounts made before `before` that never verified their address.
/// Agents and integrations verify nothing, and stay.
pub async fn prune_unverified(
    executor: impl PgExecutor<'_>,
    before: OffsetDateTime,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM users WHERE email_verified_at IS NULL AND created_at < $1 AND kind = 'human'",
        before
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// The preferences as stored, `None` without such an account.
pub async fn preferences(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Json>> {
    sqlx::query_scalar!("SELECT preferences FROM users WHERE id = $1", id)
        .fetch_optional(executor)
        .await
}

pub async fn set_preferences(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    preferences: &Json,
    now: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET preferences = $2, updated_at = $3 WHERE id = $1",
        id,
        preferences,
        now,
    )
    .execute(executor)
    .await?;
    Ok(())
}
