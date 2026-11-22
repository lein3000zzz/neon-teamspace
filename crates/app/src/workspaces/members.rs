//! Who is in a workspace: listing them, changing their roles, removing them, leaving, and
//! handing ownership over.

use neon_domain::auth::Principal;
use neon_domain::workspaces::permissions::{check_owner_remains, check_teamspace_owner_remains};
use neon_domain::workspaces::{Action, Member, Resource, TeamspaceRole, WorkspaceRole, authorize};
use neon_store::{teamspaces, workspace_invites, workspaces as repo};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{AccessChange, WorkspaceError, WorkspaceService, actor, locked_membership, membership};

const NO_MEMBER: &str = "no such member";

impl WorkspaceService {
    /// A workspace's members that the principal may know of, owners first: everyone, or
    /// for a guest only themselves.
    pub async fn list_members(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<Member>, WorkspaceError> {
        self.observe("list_members", async {
            let mut conn = self.pool().acquire().await?;
            let membership = membership(&mut conn, principal, workspace_id).await?;
            membership.check(principal, Action::ViewWorkspace)?;
            let actor = actor(principal, membership.role);
            Ok(repo::list_members(&mut *conn, workspace_id)
                .await?
                .into_iter()
                .filter(|member| {
                    let resource = Resource::Member {
                        role: member.role,
                        is_actor: member.person.id == principal.user.id,
                    };
                    authorize(actor, Action::ViewMember, resource).is_ok()
                })
                .collect())
        })
        .await
    }

    /// Changes a member's role. Owners change anyone's; admins anyone's but an owner's,
    /// and make no owners. The last owner keeps the role. A member who becomes a guest
    /// leaves every teamspace; a guest who becomes more joins the default one.
    pub async fn change_member_role(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        user_id: Uuid,
        role: WorkspaceRole,
    ) -> Result<Member, WorkspaceError> {
        self.observe("change_member_role", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(principal, Action::ViewWorkspace)?;
            let current = target_role(&mut tx, workspace_id, user_id).await?;
            let target = Resource::Member {
                role: current,
                is_actor: user_id == principal.user.id,
            };
            authorize(
                actor(principal, membership.role),
                Action::ChangeRole { to: role },
                target,
            )
            .map_err(|denied| WorkspaceError::denied(denied, NO_MEMBER))?;
            if current != role {
                let owners = repo::count_owners(&mut *tx, workspace_id).await?;
                check_owner_remains(
                    owners,
                    current == WorkspaceRole::Owner,
                    role == WorkspaceRole::Owner,
                )?;
                self.set_role(&mut tx, workspace_id, user_id, current, role)
                    .await?;
            }
            let member = repo::member(&mut *tx, workspace_id, user_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                user_id = %user_id,
                from = %current,
                to = %role,
                "member role changed"
            );
            self.member_changed(workspace_id, user_id);
            Ok(member)
        })
        .await
    }

    /// Takes a member out of the workspace and its teamspaces. Owners remove anyone;
    /// admins anyone but an owner. Nobody removes themselves: they leave.
    pub async fn remove_member(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("remove_member", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(principal, Action::ViewWorkspace)?;
            let current = target_role(&mut tx, workspace_id, user_id).await?;
            let target = Resource::Member {
                role: current,
                is_actor: user_id == principal.user.id,
            };
            authorize(
                actor(principal, membership.role),
                Action::RemoveMember,
                target,
            )
            .map_err(|denied| WorkspaceError::denied(denied, NO_MEMBER))?;
            let owners = repo::count_owners(&mut *tx, workspace_id).await?;
            check_owner_remains(owners, current == WorkspaceRole::Owner, false)?;
            repo::remove_member(&mut *tx, workspace_id, user_id).await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                user_id = %user_id,
                "member removed"
            );
            self.member_changed(workspace_id, user_id);
            Ok(())
        })
        .await
    }

    /// Leaves a workspace and its teamspaces. The last owner cannot: they make another
    /// owner first, or delete the workspace. Nor can the last owner of one of its
    /// teamspaces, unless they own or administer the workspace, as leaving it would leave
    /// the teamspace without an owner.
    pub async fn leave_workspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("leave_workspace", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(principal, Action::Leave)?;
            let role = membership.role()?;
            let owners = repo::count_owners(&mut *tx, workspace_id).await?;
            check_owner_remains(owners, role == WorkspaceRole::Owner, false)?;
            // The workspace's lock keeps every teamspace's owners as they are meanwhile.
            for owners in
                teamspaces::owners_where_owner(&mut *tx, workspace_id, principal.user.id).await?
            {
                check_teamspace_owner_remains(owners, true, false, role.manages())?;
            }
            repo::remove_member(&mut *tx, workspace_id, principal.user.id).await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, workspace_id = %workspace_id, "workspace left");
            self.member_changed(workspace_id, principal.user.id);
            Ok(())
        })
        .await
    }

    /// Makes another member an owner and the owner calling an admin, together. Only from a
    /// browser session.
    pub async fn transfer_ownership(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        to: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("transfer_ownership", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(principal, Action::ViewWorkspace)?;
            let current = target_role(&mut tx, workspace_id, to).await?;
            let target = Resource::Member {
                role: current,
                is_actor: to == principal.user.id,
            };
            authorize(
                actor(principal, membership.role),
                Action::TransferOwnership,
                target,
            )
            .map_err(|denied| WorkspaceError::denied(denied, NO_MEMBER))?;
            self.set_role(&mut tx, workspace_id, to, current, WorkspaceRole::Owner)
                .await?;
            repo::set_member_role(
                &mut *tx,
                workspace_id,
                principal.user.id,
                WorkspaceRole::Admin,
            )
            .await?;
            tx.commit().await?;
            info!(
                from = %principal.user.id,
                to = %to,
                workspace_id = %workspace_id,
                "ownership transferred"
            );
            self.member_changed(workspace_id, to);
            self.member_changed(workspace_id, principal.user.id);
            Ok(())
        })
        .await
    }

    /// Tells the port that a member's role changed, or that they left or were removed.
    fn member_changed(&self, workspace_id: Uuid, user_id: Uuid) {
        self.changed(AccessChange::Workspace {
            workspace_id,
            user_id: Some(user_id),
        });
    }

    /// Saves a new role, with what it means for teamspaces: guests are in none, everyone
    /// else in the default one.
    pub(super) async fn set_role(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
        user_id: Uuid,
        from: WorkspaceRole,
        to: WorkspaceRole,
    ) -> Result<(), WorkspaceError> {
        repo::set_member_role(&mut *conn, workspace_id, user_id, to).await?;
        if from.joins_teamspaces() && !to.joins_teamspaces() {
            teamspaces::leave_all(&mut *conn, workspace_id, user_id).await?;
        } else if !from.joins_teamspaces() && to.joins_teamspaces() {
            teamspaces::join_default(
                &mut *conn,
                workspace_id,
                user_id,
                TeamspaceRole::Member,
                self.now(),
            )
            .await?;
        }
        Ok(())
    }

    /// Adds an account to a workspace with `role`, and to its default teamspace unless it
    /// is a guest. Run it on the transaction that locked the workspace and checked its
    /// limit. Whatever invitation of the account was still pending goes: once it is in,
    /// however it came in, no invitation may admit it again after it leaves or is removed.
    pub(super) async fn add_member(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
        user_id: Uuid,
        role: WorkspaceRole,
    ) -> Result<(), WorkspaceError> {
        let now = self.now();
        repo::add_member(&mut *conn, workspace_id, user_id, role, now)
            .await
            .map_err(|error| {
                if neon_store::is_unique_violation(&error, repo::ALREADY_MEMBER) {
                    WorkspaceError::AlreadyMember
                } else {
                    WorkspaceError::Database(error)
                }
            })?;
        workspace_invites::delete_pending(&mut *conn, workspace_id, user_id).await?;
        if role.joins_teamspaces() {
            teamspaces::join_default(
                &mut *conn,
                workspace_id,
                user_id,
                TeamspaceRole::Member,
                now,
            )
            .await?;
        }
        Ok(())
    }
}

/// The role of the member an action is aimed at. Whether the principal may know of them is
/// the rules' to say, as they would of anyone else.
async fn target_role(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<WorkspaceRole, WorkspaceError> {
    repo::member_role(&mut *conn, workspace_id, user_id)
        .await?
        .ok_or(WorkspaceError::NotFound(NO_MEMBER))
}
