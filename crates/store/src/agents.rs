//! Agents and integrations: rows of users, each with its workspace and owner, and a member
//! of that workspace. A deleted one keeps its row, out of its workspace.

use neon_domain::agents::{AgentOwner, Standing};
use neon_domain::auth::UserKind;
use sqlx::{PgConnection, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::users::{owner, parse_hue, parse_kind, stored_hue};

/// An agent or integration as stored, with what decides whether it works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub id: Uuid,
    pub kind: UserKind,
    pub workspace_id: Uuid,
    pub name: String,
    pub hue_override: Option<u16>,
    pub owner: Option<AgentOwner>,
    pub standing: Standing,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

/// A new agent or integration.
#[derive(Debug, Clone)]
pub struct New<'a> {
    pub id: Uuid,
    pub kind: UserKind,
    pub workspace_id: Uuid,
    pub owner_id: Uuid,
    pub name: &'a str,
    pub hue_override: Option<u16>,
    pub at: OffsetDateTime,
}

/// Saves a new agent or integration, a member of its workspace. Run it on the transaction
/// that locked the workspace, then put it in the workspace's default teamspace.
pub async fn insert(conn: &mut PgConnection, new: &New<'_>) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO users (id, kind, display_name, role, hue, created_at, updated_at,
                            workspace_id, owner_id)
         VALUES ($1, $2, $3, 'user', $4, $5, $5, $6, $7)",
        new.id,
        new.kind.as_str(),
        new.name,
        stored_hue(new.hue_override),
        new.at,
        new.workspace_id,
        new.owner_id,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO workspace_members (workspace_id, user_id, kind, role, joined_at)
         VALUES ($1, $2, $3, 'member', $4)",
        new.workspace_id,
        new.id,
        new.kind.as_str(),
        new.at,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// A workspace's agents and integrations, deleted ones too, those that are not first, each
/// group by name.
pub async fn list(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<Vec<Stored>> {
    sqlx::query_as!(
        Row,
        r#"SELECT u.id, u.kind, u.workspace_id AS "workspace_id!", u.display_name, u.hue,
                  u.created_at, u.updated_at, u.deleted_at,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users d ON d.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND d.disabled_at IS NULL
                  ) AS "owner_active!",
                  w.agents_enabled
           FROM users u
           JOIN workspaces w ON w.id = u.workspace_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE u.workspace_id = $1
           ORDER BY u.deleted_at IS NOT NULL, lower(u.display_name), u.id"#,
        workspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Stored::try_from)
    .collect()
}

/// One of a workspace's agents or integrations, deleted or not.
pub async fn get(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Stored>> {
    sqlx::query_as!(
        Row,
        r#"SELECT u.id, u.kind, u.workspace_id AS "workspace_id!", u.display_name, u.hue,
                  u.created_at, u.updated_at, u.deleted_at,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  EXISTS (
                      SELECT 1 FROM workspace_members m JOIN users d ON d.id = m.user_id
                      WHERE m.workspace_id = u.workspace_id AND m.user_id = u.owner_id
                        AND d.disabled_at IS NULL
                  ) AS "owner_active!",
                  w.agents_enabled
           FROM users u
           JOIN workspaces w ON w.id = u.workspace_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE u.workspace_id = $1 AND u.id = $2"#,
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Stored::try_from)
    .transpose()
}

/// Locks an agent or integration of a workspace until the transaction ends, and says
/// whether there is one, deleted or not. Changes to one take this lock after the
/// workspace's.
pub async fn lock(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM users WHERE workspace_id = $1 AND id = $2 FOR UPDATE",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .is_some())
}

/// How many agents and integrations a workspace has, deleted ones left out: what its
/// agent limit counts.
pub async fn count_live(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM users
           WHERE workspace_id = $1 AND deleted_at IS NULL"#,
        workspace_id,
    )
    .fetch_one(executor)
    .await
}

/// Renames one, pins or unpins its hue, or both: each part `None` stays as it is, and
/// `Some(None)` unpins the hue.
pub async fn update(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    name: Option<&str>,
    hue: Option<Option<u16>>,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users
         SET display_name = COALESCE($2, display_name),
             hue = CASE WHEN $3 THEN $4 ELSE hue END,
             updated_at = $5
         WHERE id = $1 AND kind <> 'human'",
        id,
        name,
        hue.is_some(),
        stored_hue(hue.flatten()),
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Hands one to another owner.
pub async fn set_owner(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    owner_id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET owner_id = $2, updated_at = $3 WHERE id = $1 AND kind <> 'human'",
        id,
        owner_id,
        at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Marks one deleted and takes it out of its workspace, and so out of its teamspaces,
/// shares and favorites. Its row stays, for what names it.
pub async fn mark_deleted(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    id: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE users SET deleted_at = $2, updated_at = $2 WHERE id = $1 AND kind <> 'human'",
        id,
        at,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
        workspace_id,
        id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

struct Row {
    id: Uuid,
    kind: String,
    workspace_id: Uuid,
    display_name: String,
    hue: Option<i16>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    deleted_at: Option<OffsetDateTime>,
    owner_id: Option<Uuid>,
    owner_name: Option<String>,
    owner_hue: Option<i16>,
    owner_active: bool,
    agents_enabled: bool,
}

impl TryFrom<Row> for Stored {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            kind: parse_kind(&row.kind)?,
            workspace_id: row.workspace_id,
            name: row.display_name,
            hue_override: parse_hue(row.hue)?,
            owner: owner(row.owner_id, row.owner_name, row.owner_hue)?,
            standing: Standing {
                deleted: row.deleted_at.is_some(),
                owner_active: row.owner_active,
                workspace_agents_on: row.agents_enabled,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
        })
    }
}
