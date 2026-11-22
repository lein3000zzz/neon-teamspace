//! Teamspaces: listed as each member may see them, created, changed and deleted, joined
//! and left, and their members managed.

use neon_domain::auth::Principal;
use neon_domain::hue::check_hue;
use neon_domain::workspaces::permissions::check_teamspace_owner_remains;
use neon_domain::workspaces::{
    Action, Resource, Teamspace, TeamspaceAccess, TeamspaceMember, TeamspaceRole, WorkspaceRole,
    authorize, check_icon, check_name,
};
use neon_store::{is_unique_violation, teamspaces as repo, workspaces};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{Membership, WorkspaceError, WorkspaceService, actor, locked_membership, membership};

const NO_TEAMSPACE: &str = "no such teamspace";
const NO_MEMBER: &str = "no such member";

/// What a new teamspace starts with: open, closed or private.
#[derive(Debug, Clone)]
pub struct NewTeamspace {
    pub name: String,
    pub icon: Option<String>,
    pub hue: Option<i64>,
    pub access: TeamspaceAccess,
}

/// What a change to a teamspace sets. Each part left out stays as it is; for the icon and
/// the hue, `Some(None)` removes them.
#[derive(Debug, Clone, Default)]
pub struct TeamspaceUpdate {
    pub name: Option<String>,
    pub icon: Option<Option<String>>,
    pub hue: Option<Option<i64>>,
    pub access: Option<TeamspaceAccess>,
}

/// A teamspace the caller may see, with their role in it and how many members it has.
#[derive(Debug, Clone)]
pub struct TeamspaceEntry {
    pub teamspace: Teamspace,
    pub role: Option<TeamspaceRole>,
    pub members: i64,
}

/// The teamspaces of a workspace as the caller sees them.
#[derive(Debug, Clone, Default)]
pub struct TeamspaceList {
    /// Those they may see, the default one first.
    pub teamspaces: Vec<TeamspaceEntry>,
    /// Private teamspaces they are not in, which as an owner or admin of the workspace
    /// they see by name only, to delete them or join them to manage them.
    pub hidden: Vec<Teamspace>,
}

/// A teamspace, its workspace and the caller's role in each.
struct Context {
    membership: Membership,
    teamspace: Teamspace,
    role: Option<TeamspaceRole>,
}

impl Context {
    fn check(&self, principal: &Principal, action: Action) -> Result<(), WorkspaceError> {
        let resource = Resource::Teamspace {
            access: self.teamspace.access,
            membership: self.role,
        };
        authorize(actor(principal, self.membership.role), action, resource)
            .map_err(|denied| WorkspaceError::denied(denied, NO_TEAMSPACE))
    }

    /// Whether the caller is an owner or admin of the workspace, who manage every
    /// teamspace in it.
    fn manages_workspace(&self) -> bool {
        self.membership.role.is_some_and(WorkspaceRole::manages)
    }
}

impl WorkspaceService {
    /// The teamspaces the principal may see, with their role in each. A guest sees none.
    pub async fn list_teamspaces(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<TeamspaceList, WorkspaceError> {
        self.observe("list_teamspaces", async {
            let mut conn = self.pool().acquire().await?;
            let membership = membership(&mut conn, principal, workspace_id).await?;
            membership.check(principal, Action::ViewWorkspace)?;
            let actor = actor(principal, membership.role);
            let mut list = TeamspaceList::default();
            for (teamspace, role, members) in
                repo::list_with_membership(&mut *conn, workspace_id, principal.user.id).await?
            {
                let resource = Resource::Teamspace {
                    access: teamspace.access,
                    membership: role,
                };
                if authorize(actor, Action::ViewTeamspace, resource).is_ok() {
                    list.teamspaces.push(TeamspaceEntry {
                        teamspace,
                        role,
                        members,
                    });
                } else if authorize(actor, Action::SeeTeamspace, resource).is_ok() {
                    list.hidden.push(teamspace);
                }
            }
            Ok(list)
        })
        .await
    }

    pub async fn teamspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<TeamspaceEntry, WorkspaceError> {
        self.observe("get_teamspace", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .context(&mut conn, principal, workspace_id, id, false)
                .await?;
            context.check(principal, Action::ViewTeamspace)?;
            self.entry(&mut conn, principal, context.teamspace).await
        })
        .await
    }

    /// Makes an open, closed or private teamspace, with its creator as its owner.
    pub async fn create_teamspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        new: NewTeamspace,
    ) -> Result<TeamspaceEntry, WorkspaceError> {
        self.observe("create_teamspace", async {
            let mut tx = self.pool().begin().await?;
            locked_membership(&mut tx, principal, workspace_id)
                .await?
                .check(principal, Action::CreateTeamspace)?;
            if new.access == TeamspaceAccess::Default {
                return Err(WorkspaceError::InvalidInput(
                    "a workspace has one default teamspace: a new one is open, closed or private"
                        .to_owned(),
                ));
            }
            let now = self.now();
            let teamspace = Teamspace {
                id: Uuid::now_v7(),
                workspace_id,
                name: check_name(&new.name).map_err(WorkspaceError::invalid)?,
                icon: match new.icon {
                    Some(icon) => check_icon(&icon).map_err(WorkspaceError::invalid)?,
                    None => None,
                },
                hue_override: new
                    .hue
                    .map(check_hue)
                    .transpose()
                    .map_err(WorkspaceError::invalid)?,
                access: new.access,
                created_by: Some(principal.user.id),
                created_at: now,
                updated_at: now,
            };
            repo::insert(&mut *tx, &teamspace).await?;
            repo::add_member(
                &mut *tx,
                &teamspace,
                principal.user.id,
                TeamspaceRole::Owner,
                now,
            )
            .await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %teamspace.id,
                access = %teamspace.access,
                "teamspace created"
            );
            Ok(TeamspaceEntry {
                teamspace,
                role: Some(TeamspaceRole::Owner),
                members: 1,
            })
        })
        .await
    }

    /// Changes a teamspace's name, icon, hue or access, for its owners and the workspace's
    /// owners and admins, who join a private one first. The default teamspace keeps its
    /// access.
    pub async fn update_teamspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        update: TeamspaceUpdate,
    ) -> Result<TeamspaceEntry, WorkspaceError> {
        self.observe("update_teamspace", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::UpdateTeamspace)?;
            let mut teamspace = context.teamspace.clone();
            if let Some(access) = update.access
                && access != teamspace.access
            {
                context.check(principal, Action::ChangeTeamspaceAccess { to: access })?;
                teamspace.access = access;
            }
            if let Some(name) = update.name {
                teamspace.name = check_name(&name).map_err(WorkspaceError::invalid)?;
            }
            if let Some(icon) = update.icon {
                teamspace.icon = match icon {
                    Some(icon) => check_icon(&icon).map_err(WorkspaceError::invalid)?,
                    None => None,
                };
            }
            if let Some(hue) = update.hue {
                teamspace.hue_override = hue
                    .map(check_hue)
                    .transpose()
                    .map_err(WorkspaceError::invalid)?;
            }
            teamspace.updated_at = self.now();
            repo::update(&mut *tx, &teamspace).await?;
            let entry = self.entry(&mut tx, principal, teamspace).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                access = %entry.teamspace.access,
                "teamspace updated"
            );
            Ok(entry)
        })
        .await
    }

    /// Deletes a teamspace other than the default one, for its owners and the workspace's
    /// owners and admins, private ones they are not in included.
    pub async fn delete_teamspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("delete_teamspace", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::DeleteTeamspace)?;
            repo::delete(&mut *tx, workspace_id, id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                "teamspace deleted"
            );
            Ok(())
        })
        .await
    }

    /// Joins an open teamspace as a member. Owners and admins of the workspace may join
    /// closed and private ones too: it is how they come to manage a private one, which its
    /// members then see them in.
    pub async fn join_teamspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<TeamspaceEntry, WorkspaceError> {
        self.observe("join_teamspace", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::JoinTeamspace)?;
            repo::add_member(
                &mut *tx,
                &context.teamspace,
                principal.user.id,
                TeamspaceRole::Member,
                self.now(),
            )
            .await?;
            let entry = self.entry(&mut tx, principal, context.teamspace).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                "teamspace joined"
            );
            Ok(entry)
        })
        .await
    }

    /// Leaves a teamspace other than the default one. Its last owner may not, unless they
    /// own or administer the workspace, who manage it then.
    pub async fn leave_teamspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("leave_teamspace", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::LeaveTeamspace)?;
            let owners = repo::count_owners(&mut *tx, id).await?;
            check_teamspace_owner_remains(
                owners,
                context.role == Some(TeamspaceRole::Owner),
                false,
                context.manages_workspace(),
            )?;
            repo::remove_member(&mut *tx, id, principal.user.id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                "teamspace left"
            );
            Ok(())
        })
        .await
    }

    /// A teamspace's members, owners first.
    pub async fn teamspace_members(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Vec<TeamspaceMember>, WorkspaceError> {
        self.observe("teamspace_members", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .context(&mut conn, principal, workspace_id, id, false)
                .await?;
            context.check(principal, Action::ListTeamspaceMembers)?;
            Ok(repo::list_members(&mut *conn, id).await?)
        })
        .await
    }

    /// Adds a member of the workspace, not a guest, to a teamspace other than the default
    /// one, which everyone is in. The account's role in the workspace is read only once the
    /// caller may see the teamspace, so a caller who may not learns nothing of either.
    pub async fn add_teamspace_member(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        user_id: Uuid,
        role: TeamspaceRole,
    ) -> Result<TeamspaceMember, WorkspaceError> {
        self.observe("add_teamspace_member", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::ViewTeamspace)?;
            let workspace_role = workspaces::member_role(&mut *tx, workspace_id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            context.check(principal, Action::AddTeamspaceMember { workspace_role })?;
            repo::add_member(&mut *tx, &context.teamspace, user_id, role, self.now())
                .await
                .map_err(|error| {
                    if is_unique_violation(&error, repo::ALREADY_MEMBER) {
                        WorkspaceError::AlreadyMember
                    } else {
                        WorkspaceError::Database(error)
                    }
                })?;
            let member = repo::member(&mut *tx, id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                user_id = %user_id,
                role = %role,
                "teamspace member added"
            );
            Ok(member)
        })
        .await
    }

    /// Changes a teamspace member's role. The last owner stays one unless an owner or admin
    /// of the workspace makes the change.
    pub async fn change_teamspace_role(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        user_id: Uuid,
        role: TeamspaceRole,
    ) -> Result<TeamspaceMember, WorkspaceError> {
        self.observe("change_teamspace_role", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::ChangeTeamspaceRole)?;
            let current = repo::member_role(&mut *tx, id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            if current != role {
                let owners = repo::count_owners(&mut *tx, id).await?;
                check_teamspace_owner_remains(
                    owners,
                    current == TeamspaceRole::Owner,
                    role == TeamspaceRole::Owner,
                    context.manages_workspace(),
                )?;
                repo::set_member_role(&mut *tx, id, user_id, role).await?;
            }
            let member = repo::member(&mut *tx, id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                user_id = %user_id,
                from = %current,
                to = %role,
                "teamspace role changed"
            );
            Ok(member)
        })
        .await
    }

    /// Takes a member out of a teamspace other than the default one. The last owner stays
    /// unless an owner or admin of the workspace removes them.
    pub async fn remove_teamspace_member(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("remove_teamspace_member", async {
            let mut tx = self.pool().begin().await?;
            let context = self
                .context(&mut tx, principal, workspace_id, id, true)
                .await?;
            context.check(principal, Action::RemoveTeamspaceMember)?;
            let current = repo::member_role(&mut *tx, id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            let owners = repo::count_owners(&mut *tx, id).await?;
            check_teamspace_owner_remains(
                owners,
                current == TeamspaceRole::Owner,
                false,
                context.manages_workspace(),
            )?;
            repo::remove_member(&mut *tx, id, user_id).await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                user_id = %user_id,
                "teamspace member removed"
            );
            Ok(())
        })
        .await
    }

    /// Reads a teamspace of a workspace and the principal's roles. With `lock`, the
    /// workspace's row is locked, then the teamspace's, in the order every change takes
    /// them: who is in the workspace, and so who may be in its teamspaces, cannot change
    /// meanwhile, nor can the teamspace's owners.
    async fn context(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        lock: bool,
    ) -> Result<Context, WorkspaceError> {
        let membership = if lock {
            locked_membership(&mut *conn, principal, workspace_id).await?
        } else {
            membership(&mut *conn, principal, workspace_id).await?
        };
        // Someone outside the workspace learns nothing of its teamspaces.
        membership.check(principal, Action::ViewWorkspace)?;
        let teamspace = if lock {
            repo::lock(&mut *conn, workspace_id, id).await?
        } else {
            repo::get(&mut *conn, workspace_id, id).await?
        }
        .ok_or(WorkspaceError::NotFound(NO_TEAMSPACE))?;
        let role = repo::member_role(&mut *conn, id, principal.user.id).await?;
        Ok(Context {
            membership,
            teamspace,
            role,
        })
    }

    async fn entry(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        teamspace: Teamspace,
    ) -> Result<TeamspaceEntry, WorkspaceError> {
        let role = repo::member_role(&mut *conn, teamspace.id, principal.user.id).await?;
        let members = repo::count_members(&mut *conn, teamspace.id).await?;
        Ok(TeamspaceEntry {
            teamspace,
            role,
            members,
        })
    }
}
