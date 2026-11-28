//! Workspaces, their members, people, agents and integrations alike, and their limits.

use neon_domain::agents::AgentCreation;
use neon_domain::workspaces::{LimitOverrides, Member, Person, Workspace, WorkspaceRole};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode;
use crate::users::{PersonColumns, parse_hue, stored_hue};

/// The constraint an update breaks when another workspace has the slug.
pub const SLUG_TAKEN: &str = "workspaces_slug_key";

/// The constraint an insert breaks when the account is in the workspace already.
pub const ALREADY_MEMBER: &str = "workspace_members_pkey";

/// A `workspaces` row without its limits.
pub(crate) struct Row {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) icon: Option<String>,
    pub(crate) hue: Option<i16>,
    pub(crate) members_can_create_teamspaces: bool,
    pub(crate) agents_enabled: bool,
    pub(crate) agent_creation: String,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) created_at: OffsetDateTime,
    pub(crate) updated_at: OffsetDateTime,
}

impl TryFrom<Row> for Workspace {
    type Error = sqlx::Error;

    fn try_from(row: Row) -> sqlx::Result<Self> {
        Ok(Self {
            id: row.id,
            name: row.name,
            slug: row.slug,
            icon: row.icon,
            hue_override: parse_hue(row.hue)?,
            members_can_create_teamspaces: row.members_can_create_teamspaces,
            agents_enabled: row.agents_enabled,
            agent_creation: decode("agent creation", &row.agent_creation, AgentCreation::parse)?,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

pub(crate) fn parse_role(text: &str) -> sqlx::Result<WorkspaceRole> {
    decode("workspace role", text, WorkspaceRole::parse)
}

/// Saves a new workspace, unless another has its slug: then it saves nothing and says so,
/// without failing, so the transaction can try another slug.
pub async fn insert(executor: impl PgExecutor<'_>, workspace: &Workspace) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "INSERT INTO workspaces (id, name, slug, icon, hue, members_can_create_teamspaces,
                                 agents_enabled, agent_creation, created_by, created_at,
                                 updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (slug) DO NOTHING",
        workspace.id,
        workspace.name,
        workspace.slug,
        workspace.icon,
        stored_hue(workspace.hue_override),
        workspace.members_can_create_teamspaces,
        workspace.agents_enabled,
        workspace.agent_creation.as_str(),
        workspace.created_by,
        workspace.created_at,
        workspace.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn get(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Workspace>> {
    sqlx::query_as!(
        Row,
        "SELECT id, name, slug, icon, hue, members_can_create_teamspaces, agents_enabled,
                agent_creation, created_by, created_at, updated_at
         FROM workspaces WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Workspace::try_from)
    .transpose()
}

/// Like [`get`], locking the row until the transaction ends. Every change to a workspace's
/// members takes this lock first, so they run one at a time: the last owner and the member
/// limit are checked against the members as the change before left them. It does not hold
/// back inserts that only refer to the workspace, such as a new teamspace's.
pub async fn lock(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<Option<Workspace>> {
    sqlx::query_as!(
        Row,
        "SELECT id, name, slug, icon, hue, members_can_create_teamspaces, agents_enabled,
                agent_creation, created_by, created_at, updated_at
         FROM workspaces WHERE id = $1 FOR NO KEY UPDATE",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(Workspace::try_from)
    .transpose()
}

/// Saves its name, slug, icon, hue and settings. Fails on [`SLUG_TAKEN`].
pub async fn update(executor: impl PgExecutor<'_>, workspace: &Workspace) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE workspaces
         SET name = $2, slug = $3, icon = $4, hue = $5, members_can_create_teamspaces = $6,
             agents_enabled = $7, agent_creation = $8, updated_at = $9
         WHERE id = $1",
        workspace.id,
        workspace.name,
        workspace.slug,
        workspace.icon,
        stored_hue(workspace.hue_override),
        workspace.members_can_create_teamspaces,
        workspace.agents_enabled,
        workspace.agent_creation.as_str(),
        workspace.updated_at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Deletes a workspace with everything in it. False when there is none.
pub async fn delete(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM workspaces WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// The limits an instance admin set for a workspace; `None` when there is no workspace.
pub async fn limits(
    executor: impl PgExecutor<'_>,
    id: Uuid,
) -> sqlx::Result<Option<LimitOverrides>> {
    Ok(sqlx::query!(
        "SELECT limit_members, limit_storage_bytes, limit_agents FROM workspaces WHERE id = $1",
        id,
    )
    .fetch_optional(executor)
    .await?
    .map(|row| LimitOverrides {
        members: row.limit_members,
        storage_bytes: row.limit_storage_bytes,
        agents: row.limit_agents,
    }))
}

/// Replaces a workspace's limits. False when there is no workspace.
pub async fn set_limits(
    executor: impl PgExecutor<'_>,
    id: Uuid,
    limits: LimitOverrides,
    now: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE workspaces
         SET limit_members = $2, limit_storage_bytes = $3, limit_agents = $4, updated_at = $5
         WHERE id = $1",
        id,
        limits.members,
        limits.storage_bytes,
        limits.agents,
        now,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The workspaces a user is in, with their role in each, by name.
pub async fn list_for_user(
    executor: impl PgExecutor<'_>,
    user_id: Uuid,
) -> sqlx::Result<Vec<(Workspace, WorkspaceRole)>> {
    sqlx::query!(
        "SELECT w.id, w.name, w.slug, w.icon, w.hue, w.members_can_create_teamspaces,
                w.agents_enabled, w.agent_creation, w.created_by, w.created_at, w.updated_at,
                m.role
         FROM workspaces w JOIN workspace_members m ON m.workspace_id = w.id
         WHERE m.user_id = $1
         ORDER BY lower(w.name), w.id",
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let workspace = Workspace::try_from(Row {
            id: row.id,
            name: row.name,
            slug: row.slug,
            icon: row.icon,
            hue: row.hue,
            members_can_create_teamspaces: row.members_can_create_teamspaces,
            agents_enabled: row.agents_enabled,
            agent_creation: row.agent_creation,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })?;
        Ok((workspace, parse_role(&row.role)?))
    })
    .collect()
}

/// A workspace as instance admins list it: its limits, how many people are in it, and how
/// many agents and integrations it has.
#[derive(Debug, Clone)]
pub struct Listed {
    pub workspace: Workspace,
    pub limits: LimitOverrides,
    pub people: i64,
    pub agents: i64,
}

/// Workspaces in id order, the order they were made in, after `after`, with their limits
/// and what they hold: what instance admins see.
pub async fn list_all(
    executor: impl PgExecutor<'_>,
    after: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<Vec<Listed>> {
    sqlx::query!(
        r#"SELECT w.id, w.name, w.slug, w.icon, w.hue, w.members_can_create_teamspaces,
                  w.agents_enabled, w.agent_creation, w.created_by, w.created_at,
                  w.updated_at, w.limit_members, w.limit_storage_bytes, w.limit_agents,
                  (SELECT count(*) FROM workspace_members m
                   WHERE m.workspace_id = w.id AND m.kind = 'human') AS "people!",
                  (SELECT count(*) FROM users a
                   WHERE a.workspace_id = w.id AND a.deleted_at IS NULL) AS "agents!"
           FROM workspaces w
           WHERE $1::uuid IS NULL OR w.id > $1
           ORDER BY w.id LIMIT $2"#,
        after,
        limit,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let workspace = Workspace::try_from(Row {
            id: row.id,
            name: row.name,
            slug: row.slug,
            icon: row.icon,
            hue: row.hue,
            members_can_create_teamspaces: row.members_can_create_teamspaces,
            agents_enabled: row.agents_enabled,
            agent_creation: row.agent_creation,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })?;
        let limits = LimitOverrides {
            members: row.limit_members,
            storage_bytes: row.limit_storage_bytes,
            agents: row.limit_agents,
        };
        Ok(Listed {
            workspace,
            limits,
            people: row.people,
            agents: row.agents,
        })
    })
    .collect()
}

/// The owners of each of `workspaces`, by workspace, oldest member first.
pub async fn owners_of(
    executor: impl PgExecutor<'_>,
    workspaces: &[Uuid],
) -> sqlx::Result<Vec<(Uuid, Person)>> {
    sqlx::query!(
        "SELECT m.workspace_id, u.id, u.kind, u.display_name, u.email, u.hue
         FROM workspace_members m JOIN users u ON u.id = m.user_id
         WHERE m.workspace_id = ANY($1) AND m.role = 'owner'
         ORDER BY m.workspace_id, m.joined_at, u.id",
        workspaces,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let person = Person::try_from(PersonColumns {
            id: row.id,
            kind: row.kind,
            display_name: row.display_name,
            email: row.email,
            hue: row.hue,
            owner_id: None,
            owner_name: None,
            owner_hue: None,
        })?;
        Ok((row.workspace_id, person))
    })
    .collect()
}

/// Adds a person's account to a workspace. Fails on [`ALREADY_MEMBER`] when it is in it.
/// Agents and integrations join theirs as they are made.
pub async fn add_member(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
    role: WorkspaceRole,
    joined_at: OffsetDateTime,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO workspace_members (workspace_id, user_id, role, joined_at)
         VALUES ($1, $2, $3, $4)",
        workspace_id,
        user_id,
        role.as_str(),
        joined_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A user's role in a workspace; `None` when they are not in it.
pub async fn member_role(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<WorkspaceRole>> {
    sqlx::query_scalar!(
        "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
        workspace_id,
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(|role| parse_role(&role))
    .transpose()
}

pub async fn set_member_role(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
    role: WorkspaceRole,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE workspace_members SET role = $3 WHERE workspace_id = $1 AND user_id = $2",
        workspace_id,
        user_id,
        role.as_str(),
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Takes a user out of a workspace and, by the cascade, out of its teamspaces. False when
/// they were not in it.
pub async fn remove_member(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
        workspace_id,
        user_id,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// How many people are in a workspace, guests included, and agents and integrations not.
pub async fn count_members(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM workspace_members
           WHERE workspace_id = $1 AND kind = 'human'"#,
        workspace_id,
    )
    .fetch_one(executor)
    .await
}

pub async fn count_owners(executor: impl PgExecutor<'_>, workspace_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM workspace_members
           WHERE workspace_id = $1 AND role = 'owner'"#,
        workspace_id,
    )
    .fetch_one(executor)
    .await
}

/// A workspace's members, by role, owners first, then by name, agents and integrations
/// with the people.
pub async fn list_members(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<Member>> {
    sqlx::query_as!(
        MemberRow,
        r#"SELECT u.id, u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  m.role, m.joined_at,
                  (u.kind <> 'human' AND NOT EXISTS (
                      SELECT 1 FROM workspace_members om JOIN users ou ON ou.id = om.user_id
                      WHERE om.workspace_id = m.workspace_id AND om.user_id = u.owner_id
                        AND om.role <> 'guest' AND ou.disabled_at IS NULL
                  )) AS "suspended!"
           FROM workspace_members m
           JOIN users u ON u.id = m.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE m.workspace_id = $1
           ORDER BY array_position(ARRAY['owner', 'admin', 'member', 'guest'], m.role),
                    lower(u.display_name), u.id"#,
        workspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Member::try_from)
    .collect()
}

/// One member of a workspace; `None` when the user is not in it.
pub async fn member(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<Member>> {
    sqlx::query_as!(
        MemberRow,
        r#"SELECT u.id, u.kind, u.display_name, u.email, u.hue,
                  o.id AS "owner_id?", o.display_name AS "owner_name?", o.hue AS owner_hue,
                  m.role, m.joined_at,
                  (u.kind <> 'human' AND NOT EXISTS (
                      SELECT 1 FROM workspace_members om JOIN users ou ON ou.id = om.user_id
                      WHERE om.workspace_id = m.workspace_id AND om.user_id = u.owner_id
                        AND om.role <> 'guest' AND ou.disabled_at IS NULL
                  )) AS "suspended!"
           FROM workspace_members m
           JOIN users u ON u.id = m.user_id
           LEFT JOIN users o ON o.id = u.owner_id
           WHERE m.workspace_id = $1 AND m.user_id = $2"#,
        workspace_id,
        user_id,
    )
    .fetch_optional(executor)
    .await?
    .map(Member::try_from)
    .transpose()
}

/// A member's row, with their owner's, for an agent or integration.
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
    suspended: bool,
}

impl TryFrom<MemberRow> for Member {
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
            suspended: row.suspended,
        })
    }
}
