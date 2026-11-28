//! Teamspaces: listed as each member may see them, created, changed and deleted, joined
//! and left, and their members managed.

use std::collections::HashMap;

use neon_domain::agents::check_teamspace_role;
use neon_domain::auth::Principal;
use neon_domain::hue::check_hue;
use neon_domain::workspaces::permissions::check_teamspace_owner_remains;
use neon_domain::workspaces::{
    Action, Newcomer, Resource, Teamspace, TeamspaceAccess, TeamspaceMember, TeamspaceRole,
    WorkspaceRole, check_icon, check_name,
};
use neon_store::{is_unique_violation, teamspaces as repo, workspaces};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{
    AccessChange, Membership, WorkspaceError, WorkspaceService, locked_membership, membership,
};
use crate::Read;

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

/// A teamspace, its workspace and the caller's role in each, with an agent's owner's role
/// in the teamspace.
struct Context {
    membership: Membership,
    teamspace: Teamspace,
    role: Option<TeamspaceRole>,
    owner_role: Option<TeamspaceRole>,
}

impl Context {
    fn check(&self, action: Action) -> Result<(), WorkspaceError> {
        check_teamspace(
            &self.membership,
            &self.teamspace,
            self.role,
            self.owner_role,
            action,
        )
    }

    /// Whether the caller is an owner or admin of the workspace, who manage every
    /// teamspace in it.
    fn manages_workspace(&self) -> bool {
        self.membership
            .standing
            .role()
            .is_some_and(WorkspaceRole::manages)
    }
}

/// Asks the rules about an action on a teamspace, which the caller is in with `role` and an
/// agent's owner with `owner_role`.
fn check_teamspace(
    membership: &Membership,
    teamspace: &Teamspace,
    role: Option<TeamspaceRole>,
    owner_role: Option<TeamspaceRole>,
    action: Action,
) -> Result<(), WorkspaceError> {
    let seen_with = |membership| Resource::Teamspace {
        access: teamspace.access,
        membership,
    };
    membership
        .standing
        .authorize(action, seen_with(role), seen_with(owner_role))
        .map_err(|denied| WorkspaceError::denied(denied, NO_TEAMSPACE))
}

/// A teamspace of the workspace with the caller's role in it, and their owner's when the
/// caller is an agent: what the rules need to tell what the caller may do there.
pub(super) struct Placed {
    pub(super) teamspace: Teamspace,
    pub(super) role: Option<TeamspaceRole>,
    owner_role: Option<TeamspaceRole>,
    pub(super) members: i64,
}

impl Placed {
    /// Whether the rules let the caller, a member as `membership` says, do `action` there.
    pub(super) fn allows(&self, membership: &Membership, action: Action) -> bool {
        check_teamspace(
            membership,
            &self.teamspace,
            self.role,
            self.owner_role,
            action,
        )
        .is_ok()
    }
}

/// Every teamspace of the workspace, the default one first then by name, with the
/// principal's roles in each. Which of them they may see is the rules' business, through
/// [`Placed::allows`].
pub(super) async fn placed_teamspaces(
    conn: &mut PgConnection,
    principal: &Principal,
    membership: &Membership,
    workspace_id: Uuid,
) -> Result<Vec<Placed>, WorkspaceError> {
    let owner = membership.standing.owner_id();
    let owner_roles: HashMap<Uuid, TeamspaceRole> = match owner {
        Some(owner) => repo::list_with_membership(&mut *conn, workspace_id, owner)
            .await?
            .into_iter()
            .filter_map(|(teamspace, role, _)| Some((teamspace.id, role?)))
            .collect(),
        None => HashMap::new(),
    };
    Ok(
        repo::list_with_membership(&mut *conn, workspace_id, principal.user.id)
            .await?
            .into_iter()
            .map(|(teamspace, role, members)| Placed {
                owner_role: match owner {
                    Some(_) => owner_roles.get(&teamspace.id).copied(),
                    None => role,
                },
                teamspace,
                role,
                members,
            })
            .collect(),
    )
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
            membership.check(Action::ViewWorkspace)?;
            let mut list = TeamspaceList::default();
            for placed in placed_teamspaces(&mut conn, principal, &membership, workspace_id).await?
            {
                if placed.allows(&membership, Action::ViewTeamspace) {
                    list.teamspaces.push(TeamspaceEntry {
                        teamspace: placed.teamspace,
                        role: placed.role,
                        members: placed.members,
                    });
                } else if placed.allows(&membership, Action::SeeTeamspace) {
                    list.hidden.push(placed.teamspace);
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
                .context(&mut conn, principal, workspace_id, id, Read::Plain)
                .await?;
            context.check(Action::ViewTeamspace)?;
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
                .check(Action::CreateTeamspace)?;
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::UpdateTeamspace)?;
            let mut teamspace = context.teamspace.clone();
            let before = teamspace.access;
            if let Some(access) = update.access
                && access != teamspace.access
            {
                context.check(Action::ChangeTeamspaceAccess { to: access })?;
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
            if entry.teamspace.access != before {
                self.teamspace_changed(workspace_id, id, None);
            }
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::DeleteTeamspace)?;
            repo::delete(&mut *tx, workspace_id, id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                teamspace_id = %id,
                "teamspace deleted"
            );
            self.teamspace_changed(workspace_id, id, None);
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::JoinTeamspace)?;
            check_teamspace_role(principal.user.kind, TeamspaceRole::Member)
                .map_err(WorkspaceError::invalid)?;
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
            self.teamspace_changed(workspace_id, id, Some(principal.user.id));
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::LeaveTeamspace)?;
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
            self.teamspace_changed(workspace_id, id, Some(principal.user.id));
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
                .context(&mut conn, principal, workspace_id, id, Read::Plain)
                .await?;
            context.check(Action::ListTeamspaceMembers)?;
            Ok(repo::list_members(&mut *conn, id).await?)
        })
        .await
    }

    /// Adds a member of the workspace, not a guest, to a teamspace other than the default
    /// one, which everyone is in. The account's role in the workspace is read only once the
    /// caller may see the teamspace, so a caller who may not learns nothing of either. An
    /// agent joins as a member, never an owner; its own owner adds it only to teamspaces
    /// they are in. An integration joins none: its access is its incoming webhook's target.
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::ViewTeamspace)?;
            let newcomer = workspaces::member(&mut *tx, workspace_id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            let kind = newcomer.person.kind;
            let newcomer = if kind.is_human() {
                Newcomer::Person(newcomer.role)
            } else {
                Newcomer::Delegate {
                    own: newcomer
                        .person
                        .owner
                        .is_some_and(|owner| owner.id == principal.user.id),
                }
            };
            context.check(Action::AddTeamspaceMember { newcomer })?;
            check_teamspace_role(kind, role).map_err(WorkspaceError::invalid)?;
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
            self.teamspace_changed(workspace_id, id, Some(user_id));
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::ChangeTeamspaceRole)?;
            let current = repo::member(&mut *tx, id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            check_teamspace_role(current.person.kind, role).map_err(WorkspaceError::invalid)?;
            let current = current.role;
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
            self.teamspace_changed(workspace_id, id, Some(user_id));
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
                .context(&mut tx, principal, workspace_id, id, Read::Locked)
                .await?;
            context.check(Action::RemoveTeamspaceMember)?;
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
            self.teamspace_changed(workspace_id, id, Some(user_id));
            Ok(())
        })
        .await
    }

    /// Tells the port that someone's place in a teamspace changed, or everyone's when
    /// `user_id` is `None`.
    fn teamspace_changed(&self, workspace_id: Uuid, teamspace_id: Uuid, user_id: Option<Uuid>) {
        self.changed(AccessChange::Teamspace {
            workspace_id,
            teamspace_id,
            user_id,
        });
    }

    /// Reads a teamspace of a workspace and the principal's roles. With [`Read::Locked`],
    /// the workspace's row is locked, then the teamspace's, in the order every change takes
    /// them: who is in the workspace, and so who may be in its teamspaces, cannot change
    /// meanwhile, nor can the teamspace's owners.
    async fn context(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
        read: Read,
    ) -> Result<Context, WorkspaceError> {
        let membership = match read {
            Read::Locked => locked_membership(&mut *conn, principal, workspace_id).await?,
            Read::Plain => membership(&mut *conn, principal, workspace_id).await?,
        };
        // Someone outside the workspace learns nothing of its teamspaces.
        membership.check(Action::ViewWorkspace)?;
        let teamspace = match read {
            Read::Locked => repo::lock(&mut *conn, workspace_id, id).await?,
            Read::Plain => repo::get(&mut *conn, workspace_id, id).await?,
        }
        .ok_or(WorkspaceError::NotFound(NO_TEAMSPACE))?;
        let role = repo::member_role(&mut *conn, id, principal.user.id).await?;
        let owner_role = match membership.standing.owner_id() {
            Some(owner) => repo::member_role(&mut *conn, id, owner).await?,
            None => role,
        };
        Ok(Context {
            membership,
            teamspace,
            role,
            owner_role,
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
