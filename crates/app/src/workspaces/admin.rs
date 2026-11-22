//! What instance admins do with workspaces: list them, set their limits, delete them. They
//! see names, owners, counts, limits and usage, and never what is inside.

use neon_domain::auth::Principal;
use neon_domain::workspaces::{
    Action, LimitOverrides, LimitValue, Limits, Person, Resource, Workspace, authorize, check_limit,
};
use neon_store::workspaces as repo;
use tracing::info;
use uuid::Uuid;

use super::{AccessChange, NO_WORKSPACE, WorkspaceError, WorkspaceService, actor};

/// The most workspaces one page lists, and how many it lists unless told.
pub const MAX_PAGE: i64 = 200;
pub const DEFAULT_PAGE: i64 = 50;

/// What a workspace holds, against its limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub members: i64,
    /// Bytes of attachments, which do not exist yet: always 0 for now.
    pub storage_bytes: i64,
    /// Agents, which do not exist yet: always 0 for now.
    pub agents: i64,
}

impl Usage {
    fn of(members: i64) -> Self {
        Self {
            members,
            storage_bytes: 0,
            agents: 0,
        }
    }
}

/// A workspace as instance admins see it.
#[derive(Debug, Clone)]
pub struct AdminWorkspace {
    pub workspace: Workspace,
    /// Oldest first.
    pub owners: Vec<Person>,
    /// The limits set for this workspace alone.
    pub overrides: LimitOverrides,
    /// The limits that hold for it.
    pub limits: Limits,
    pub usage: Usage,
}

/// A page of workspaces in the order they were made.
#[derive(Debug, Clone)]
pub struct AdminWorkspacePage {
    pub workspaces: Vec<AdminWorkspace>,
    /// Pass it as `after` for the next page; unset on the last one.
    pub next: Option<Uuid>,
}

/// What an instance admin changes about a workspace's limits. Each part left out stays
/// as it is; `Some(None)` goes back to the instance's default, and `Some(Some(0))` lifts
/// the limit for this workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimitsUpdate {
    pub members: Option<Option<LimitValue>>,
    pub storage_bytes: Option<Option<LimitValue>>,
    pub agents: Option<Option<LimitValue>>,
}

impl WorkspaceService {
    /// Every workspace on the instance, a page at a time, for instance admins.
    pub async fn admin_workspaces(
        &self,
        principal: &Principal,
        after: Option<Uuid>,
        limit: Option<i64>,
    ) -> Result<AdminWorkspacePage, WorkspaceError> {
        self.observe("admin_list_workspaces", async {
            self.check_instance(principal, Action::ListAllWorkspaces)?;
            let limit = limit.unwrap_or(DEFAULT_PAGE);
            if !(1..=MAX_PAGE).contains(&limit) {
                return Err(WorkspaceError::InvalidInput(format!(
                    "a page lists 1 to {MAX_PAGE} workspaces"
                )));
            }
            let mut conn = self.pool().acquire().await?;
            let mut listed = repo::list_all(&mut *conn, after, limit + 1).await?;
            let page = usize::try_from(limit).unwrap_or(usize::MAX);
            let more = listed.len() > page;
            listed.truncate(page);
            let ids: Vec<Uuid> = listed
                .iter()
                .map(|(workspace, _, _)| workspace.id)
                .collect();
            let mut owners = repo::owners_of(&mut *conn, &ids).await?;
            let workspaces: Vec<AdminWorkspace> = listed
                .into_iter()
                .map(|(workspace, overrides, members)| {
                    let (theirs, rest) = owners
                        .drain(..)
                        .partition(|(workspace_id, _)| *workspace_id == workspace.id);
                    owners = rest;
                    AdminWorkspace {
                        owners: theirs.into_iter().map(|(_, person)| person).collect(),
                        limits: self.limits(overrides),
                        overrides,
                        usage: Usage::of(members),
                        workspace,
                    }
                })
                .collect();
            let next = more
                .then(|| workspaces.last().map(|entry| entry.workspace.id))
                .flatten();
            Ok(AdminWorkspacePage { workspaces, next })
        })
        .await
    }

    /// Sets a workspace's own limits, for instance admins. A workspace already past a new
    /// limit keeps everyone in it, and takes no one more.
    pub async fn set_workspace_limits(
        &self,
        principal: &Principal,
        id: Uuid,
        update: LimitsUpdate,
    ) -> Result<AdminWorkspace, WorkspaceError> {
        self.observe("set_workspace_limits", async {
            self.check_instance(principal, Action::SetLimits)?;
            let checked = |value: Option<Option<LimitValue>>| {
                value
                    .map(|value| value.map(check_limit).transpose())
                    .transpose()
                    .map_err(WorkspaceError::invalid)
            };
            let (members, storage_bytes, agents) = (
                checked(update.members)?,
                checked(update.storage_bytes)?,
                checked(update.agents)?,
            );
            let mut tx = self.pool().begin().await?;
            let workspace = repo::lock(&mut *tx, id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_WORKSPACE))?;
            let mut overrides = repo::limits(&mut *tx, id).await?.unwrap_or_default();
            if let Some(value) = members {
                overrides.members = value;
            }
            if let Some(value) = storage_bytes {
                overrides.storage_bytes = value;
            }
            if let Some(value) = agents {
                overrides.agents = value;
            }
            repo::set_limits(&mut *tx, id, overrides, self.now()).await?;
            let members = repo::count_members(&mut *tx, id).await?;
            let owners = repo::owners_of(&mut *tx, &[id])
                .await?
                .into_iter()
                .map(|(_, person)| person)
                .collect();
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %id,
                members = ?overrides.members,
                storage_bytes = ?overrides.storage_bytes,
                agents = ?overrides.agents,
                "workspace limits set"
            );
            Ok(AdminWorkspace {
                workspace,
                owners,
                limits: self.limits(overrides),
                overrides,
                usage: Usage::of(members),
            })
        })
        .await
    }

    /// Deletes a workspace with everything in it, for instance admins, who need not be in
    /// it. Only from a browser session.
    pub async fn admin_delete_workspace(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("admin_delete_workspace", async {
            self.check_instance(principal, Action::DeleteAnyWorkspace)?;
            if !repo::delete(self.pool(), id).await? {
                return Err(WorkspaceError::NotFound(NO_WORKSPACE));
            }
            info!(actor_id = %principal.user.id, workspace_id = %id, "workspace deleted by an instance admin");
            self.changed(AccessChange::Workspace {
                workspace_id: id,
                user_id: None,
            });
            Ok(())
        })
        .await
    }

    fn check_instance(&self, principal: &Principal, action: Action) -> Result<(), WorkspaceError> {
        let instance = Resource::Instance {
            creation: self.creation(),
        };
        authorize(actor(principal, None), action, instance)
            .map_err(|denied| WorkspaceError::denied(denied, NO_WORKSPACE))
    }
}
