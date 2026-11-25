//! Teamspaces and their members. Every query names the workspace too, so a teamspace is
//! only ever found through the workspace it belongs to, but for [`find`], for the routes
//! that name a teamspace alone.

use neon_domain::workspaces::{Person, Teamspace, TeamspaceAccess, TeamspaceMember, TeamspaceRole};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;
use crate::users::{PersonColumns, parse_hue, stored_hue};

/// The constraint an insert breaks when the user is in the teamspace already.
pub const ALREADY_MEMBER: &str = "teamspace_members_pkey";

struct Row {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    icon: Option<String>,
    hue: Option<i16>,
    access: String,
    created_by: Option<Uuid>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<Row> for Teamspace {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            workspace_id: row.workspace_id,
            name: row.name,
            icon: row.icon,
            hue_override: parse_hue(row.hue)?,
            access: TeamspaceAccess::parse(&row.access)
                .ok_or_else(|| decode_error(format!("unknown access {:?}", row.access)))?,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

pub(crate) fn parse_role(text: &str) -> sqlx::Result<TeamspaceRole> {
    TeamspaceRole::parse(text)
        .ok_or_else(|| decode_error(format!("unknown teamspace role {text:?}")))
}

/// Saves a new teamspace. Fails on the unique index `teamspaces_one_default` for a second
/// default one.
pub async fn insert(executor: impl PgExecutor<'_>, teamspace: &Teamspace) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO teamspaces (id, workspace_id, name, icon, hue, access, created_by,
                                 created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        teamspace.id,
        teamspace.workspace_id,
        teamspace.name,
        teamspace.icon,
        stored_hue(teamspace.hue_override),
        teamspace.access.as_str(),
        teamspace.created_by,
        teamspace.created_at,
        teamspace.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Teamspace>> {
    sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, name, icon, hue, access, created_by, created_at, updated_at
         FROM teamspaces WHERE workspace_id = $1 AND id = $2",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Teamspace::try_from)
    .transpose()
}

/// A teamspace by its id alone, whatever its workspace, which the caller then checks they
/// are in.
pub async fn find(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Teamspace>> {
    sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, name, icon, hue, access, created_by, created_at, updated_at
         FROM teamspaces WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Teamspace::try_from)
    .transpose()
}

/// Like [`get`], locking the row until the transaction ends. Changes to a teamspace's
/// members take this lock first, so it keeps an owner as each change before left it.
pub async fn lock(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Teamspace>> {
    sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, name, icon, hue, access, created_by, created_at, updated_at
         FROM teamspaces WHERE workspace_id = $1 AND id = $2 FOR NO KEY UPDATE",
        workspace_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Teamspace::try_from)
    .transpose()
}

/// Saves its name, icon, hue and access.
pub async fn update(executor: impl PgExecutor<'_>, teamspace: &Teamspace) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE teamspaces SET name = $3, icon = $4, hue = $5, access = $6, updated_at = $7
         WHERE workspace_id = $1 AND id = $2",
        teamspace.workspace_id,
        teamspace.id,
        teamspace.name,
        teamspace.icon,
        stored_hue(teamspace.hue_override),
        teamspace.access.as_str(),
        teamspace.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Deletes a teamspace that is not the default one. False when there is none such.
pub async fn delete(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM teamspaces WHERE workspace_id = $1 AND id = $2 AND access <> 'default'",
        workspace_id,
        id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Every teamspace of a workspace, the default one first then by name, with the user's
/// role in each and how many members each has. Which the user may see is the rules'
/// business, not this query's.
pub async fn list_with_membership(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<(Teamspace, Option<TeamspaceRole>, i64)>> {
    sqlx::query!(
        r#"SELECT t.id, t.workspace_id, t.name, t.icon, t.hue, t.access, t.created_by,
                  t.created_at, t.updated_at, mine.role AS "role?",
                  (SELECT count(*) FROM teamspace_members c WHERE c.teamspace_id = t.id)
                      AS "members!"
           FROM teamspaces t
           LEFT JOIN teamspace_members mine ON mine.teamspace_id = t.id AND mine.user_id = $2
           WHERE t.workspace_id = $1
           ORDER BY t.access <> 'default', lower(t.name), t.id"#,
        workspace_id,
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let teamspace = Teamspace::try_from(Row {
            id: row.id,
            workspace_id: row.workspace_id,
            name: row.name,
            icon: row.icon,
            hue: row.hue,
            access: row.access,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })?;
        let role = row.role.as_deref().map(parse_role).transpose()?;
        Ok((teamspace, role, row.members))
    })
    .collect()
}

/// Adds a member of the workspace to one of its teamspaces. Fails on [`ALREADY_MEMBER`]
/// when they are in it, and on the foreign key when they are not in the workspace.
pub async fn add_member(
    executor: impl PgExecutor<'_>,
    teamspace: &Teamspace,
    user_id: Uuid,
    role: TeamspaceRole,
    joined_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO teamspace_members (teamspace_id, workspace_id, user_id, role, joined_at)
         VALUES ($1, $2, $3, $4, $5)",
        teamspace.id,
        teamspace.workspace_id,
        user_id,
        role.as_str(),
        joined_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Puts a member of the workspace in its default teamspace, unless they are in it.
pub async fn join_default(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
    role: TeamspaceRole,
    joined_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO teamspace_members (teamspace_id, workspace_id, user_id, role, joined_at)
         SELECT t.id, t.workspace_id, $2, $3, $4
         FROM teamspaces t WHERE t.workspace_id = $1 AND t.access = 'default'
         ON CONFLICT (teamspace_id, user_id) DO NOTHING",
        workspace_id,
        user_id,
        role.as_str(),
        joined_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Takes a member out of every teamspace of the workspace, as when they become a guest.
pub async fn leave_all(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM teamspace_members WHERE workspace_id = $1 AND user_id = $2",
        workspace_id,
        user_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// A user's role in a teamspace; `None` when they are not in it.
pub async fn member_role(
    executor: impl PgExecutor<'_>,
    teamspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<TeamspaceRole>> {
    sqlx::query_scalar!(
        "SELECT role FROM teamspace_members WHERE teamspace_id = $1 AND user_id = $2",
        teamspace_id,
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(|role| parse_role(&role))
    .transpose()
}

pub async fn set_member_role(
    executor: impl PgExecutor<'_>,
    teamspace_id: Uuid,
    user_id: Uuid,
    role: TeamspaceRole,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE teamspace_members SET role = $3 WHERE teamspace_id = $1 AND user_id = $2",
        teamspace_id,
        user_id,
        role.as_str(),
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn remove_member(
    executor: impl PgExecutor<'_>,
    teamspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM teamspace_members WHERE teamspace_id = $1 AND user_id = $2",
        teamspace_id,
        user_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn count_owners(executor: impl PgExecutor<'_>, teamspace_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM teamspace_members
           WHERE teamspace_id = $1 AND role = 'owner'"#,
        teamspace_id,
    )
    .fetch_one(executor)
    .await
}

/// How many owners each teamspace of the workspace that the user owns has, the user
/// included.
pub async fn owners_where_owner(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM teamspace_members o
                   WHERE o.teamspace_id = m.teamspace_id AND o.role = 'owner') AS "owners!"
           FROM teamspace_members m
           WHERE m.workspace_id = $1 AND m.user_id = $2 AND m.role = 'owner'"#,
        workspace_id,
        user_id,
    )
    .fetch_all(executor)
    .await
}

pub async fn count_members(executor: impl PgExecutor<'_>, teamspace_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM teamspace_members WHERE teamspace_id = $1"#,
        teamspace_id,
    )
    .fetch_one(executor)
    .await
}

/// A teamspace's members, owners first, then by name, agents and integrations with the
/// people.
pub async fn list_members(
    executor: impl PgExecutor<'_>,
    teamspace_id: Uuid,
) -> sqlx::Result<Vec<TeamspaceMember>> {
    sqlx::query_as!(
        MemberRow,
        r#"SELECT u.id, u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  m.role, m.joined_at
           FROM teamspace_members m
           JOIN users u ON u.id = m.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE m.teamspace_id = $1
           ORDER BY m.role <> 'owner', lower(u.display_name), u.id"#,
        teamspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(TeamspaceMember::try_from)
    .collect()
}

/// One member of a teamspace; `None` when the user is not in it.
pub async fn member(
    executor: impl PgExecutor<'_>,
    teamspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<TeamspaceMember>> {
    sqlx::query_as!(
        MemberRow,
        r#"SELECT u.id, u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  m.role, m.joined_at
           FROM teamspace_members m
           JOIN users u ON u.id = m.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE m.teamspace_id = $1 AND m.user_id = $2"#,
        teamspace_id,
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(TeamspaceMember::try_from)
    .transpose()
}

/// A teamspace member's row, with their owner's, for an agent or integration.
struct MemberRow {
    id: Uuid,
    kind: String,
    display_name: String,
    email: Option<String>,
    hue: Option<i16>,
    owner_id: Option<Uuid>,
    owner_name: Option<String>,
    owner_hue: Option<i16>,
    role: String,
    joined_at: OffsetDateTime,
}

impl TryFrom<MemberRow> for TeamspaceMember {
    type Error = sqlx::Error;

    fn try_from(row: MemberRow) -> sqlx::Result<Self> {
        Ok(Self {
            person: Person::try_from(PersonColumns {
                id: row.id,
                kind: row.kind,
                display_name: row.display_name,
                email: row.email,
                hue: row.hue,
                owner_id: row.owner_id,
                owner_name: row.owner_name,
                owner_hue: row.owner_hue,
            })?,
            role: parse_role(&row.role)?,
            joined_at: row.joined_at,
        })
    }
}
