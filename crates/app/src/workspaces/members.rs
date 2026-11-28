//! Who is in a workspace: listing them, changing their roles, removing them, leaving, and
//! handing ownership over.

use neon_domain::agents::AgentOwner;
use neon_domain::auth::{Principal, UserKind};
use neon_domain::workspaces::permissions::{check_owner_remains, check_teamspace_owner_remains};
use neon_domain::workspaces::{
    Action, Member, Person, Resource, TeamspaceRole, Workspace, WorkspaceRole,
};
use neon_store::{teamspaces, workspace_invites, workspaces as repo};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{
    AccessChange, Membership, WorkspaceError, WorkspaceService, locked_membership, membership,
};
use crate::webhooks;

const NO_MEMBER: &str = "no such member";

/// Whom a request acts as: its identity as the people it works with see it, and the one
/// workspace its token is bound to, while the identity is in it.
#[derive(Debug, Clone)]
pub struct Identity {
    pub person: Person,
    /// The workspace, and the role the rules see the identity in there: for a bound token
    /// without the admin scope, member at most.
    pub workspace: Option<(Workspace, WorkspaceRole)>,
}

impl WorkspaceService {
    /// Whom the principal acts as: a person, or an agent or integration with its owner.
    pub async fn identity(&self, principal: &Principal) -> Result<Identity, WorkspaceError> {
        self.observe("identity", async {
            let mut conn = self.pool().acquire().await?;
            let user = &principal.user;
            let owner = match user.owner_id.filter(|_| !user.kind.is_human()) {
                Some(owner) => neon_store::users::by_id(&mut *conn, owner)
                    .await?
                    .map(|owner| AgentOwner {
                        id: owner.id,
                        display_name: owner.display_name,
                        hue_override: owner.hue_override,
                    }),
                None => None,
            };
            let person = Person {
                id: user.id,
                kind: user.kind,
                display_name: user.display_name.clone(),
                email: user.email.as_ref().map(|email| email.as_str().to_owned()),
                hue_override: user.hue_override,
                owner,
            };
            let workspace = match principal.workspace() {
                Some(id) => {
                    let membership = membership(&mut conn, principal, id).await?;
                    membership
                        .standing
                        .role()
                        .map(|role| (membership.workspace, role))
                }
                None => None,
            };
            Ok(Identity { person, workspace })
        })
        .await
    }

    /// A workspace's members that the principal may know of, owners first: everyone, or
    /// for a guest only themselves. Agents and integrations are members too.
    pub async fn list_members(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<Member>, WorkspaceError> {
        self.observe("list_members", async {
            let mut conn = self.pool().acquire().await?;
            let membership = membership(&mut conn, principal, workspace_id).await?;
            membership.check(Action::ViewWorkspace)?;
            Ok(repo::list_members(&mut *conn, workspace_id)
                .await?
                .into_iter()
                .filter(|member| {
                    let target = Target {
                        id: member.person.id,
                        role: member.role,
                        kind: member.person.kind,
                    };
                    check_member(principal, &membership, &target, Action::ViewMember).is_ok()
                })
                .collect())
        })
        .await
    }

    /// Changes a member's role. Owners change anyone's; admins anyone's but an owner's,
    /// and make no owners. The last owner keeps the role. A member who becomes a guest
    /// leaves every teamspace; a guest who becomes more joins the default one. The webhooks
    /// whose URL they set that their new role may not register turn off. An owner or admin
    /// who becomes a member or guest, and so invites nobody, has the invitations they sent
    /// that wait for an answer revoked, and the invite links they made ended.
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
            membership.check(Action::ViewWorkspace)?;
            let target = target(&mut tx, workspace_id, user_id).await?;
            let current = target.role;
            check_member(
                principal,
                &membership,
                &target,
                Action::ChangeRole { to: role },
            )?;
            let mut withdrawn = Vec::new();
            if current != role {
                let owners = repo::count_owners(&mut *tx, workspace_id).await?;
                check_owner_remains(owners, current, Some(role))?;
                self.set_role(&mut tx, workspace_id, user_id, current, role)
                    .await?;
                if current.manages() && !role.manages() {
                    withdrawn = self
                        .withdraw_invites(&mut tx, workspace_id, user_id, principal.user.id)
                        .await?;
                }
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
            self.withdrawn(withdrawn);
            Ok(member)
        })
        .await
    }

    /// Takes a member out of the workspace and its teamspaces. Owners remove anyone;
    /// admins anyone but an owner. Nobody removes themselves: they leave. Their webhooks
    /// go, and those whose URL they set for others turn off. The invitations they sent that
    /// wait for an answer are revoked, and the invite links they made end.
    pub async fn remove_member(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("remove_member", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(Action::ViewWorkspace)?;
            let target = target(&mut tx, workspace_id, user_id).await?;
            check_member(principal, &membership, &target, Action::RemoveMember)?;
            let owners = repo::count_owners(&mut *tx, workspace_id).await?;
            check_owner_remains(owners, target.role, None)?;
            repo::remove_member(&mut *tx, workspace_id, user_id).await?;
            webhooks::lapse(&mut tx, workspace_id, user_id, self.now()).await?;
            let withdrawn = self
                .withdraw_invites(&mut tx, workspace_id, user_id, principal.user.id)
                .await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                user_id = %user_id,
                "member removed"
            );
            self.member_changed(workspace_id, user_id);
            self.withdrawn(withdrawn);
            Ok(())
        })
        .await
    }

    /// Leaves a workspace and its teamspaces, as removing a member does, webhooks, invitations
    /// and invite links included.
    /// The last owner cannot: they make another owner first, or delete the workspace. Nor
    /// can the last owner of one of its teamspaces, unless they own or administer the
    /// workspace, as leaving it would leave the teamspace without an owner.
    pub async fn leave_workspace(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("leave_workspace", async {
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(Action::Leave)?;
            let role = membership.role()?;
            let owners = repo::count_owners(&mut *tx, workspace_id).await?;
            check_owner_remains(owners, role, None)?;
            // The workspace's lock keeps every teamspace's owners as they are meanwhile.
            for owners in
                teamspaces::owners_where_owner(&mut *tx, workspace_id, principal.user.id).await?
            {
                check_teamspace_owner_remains(owners, TeamspaceRole::Owner, None, role.manages())?;
            }
            repo::remove_member(&mut *tx, workspace_id, principal.user.id).await?;
            webhooks::lapse(&mut tx, workspace_id, principal.user.id, self.now()).await?;
            let user = principal.user.id;
            let withdrawn = self
                .withdraw_invites(&mut tx, workspace_id, user, user)
                .await?;
            tx.commit().await?;
            info!(user_id = %principal.user.id, workspace_id = %workspace_id, "workspace left");
            self.member_changed(workspace_id, principal.user.id);
            self.withdrawn(withdrawn);
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
            membership.check(Action::ViewWorkspace)?;
            let target = target(&mut tx, workspace_id, to).await?;
            check_member(principal, &membership, &target, Action::TransferOwnership)?;
            self.set_role(&mut tx, workspace_id, to, target.role, WorkspaceRole::Owner)
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

    /// Tells the port that a member's role changed, or that they left or were removed. Their
    /// agents' connections are looked at again with theirs, as theirs bound them.
    fn member_changed(&self, workspace_id: Uuid, user_id: Uuid) {
        self.changed(AccessChange::Workspace {
            workspace_id,
            user_id: Some(user_id),
        });
    }

    /// Revokes the invitations into a workspace that `user_id` sent and that wait for an
    /// answer, and ends the invite links they made, as `by` does: once they may invite
    /// nobody there, nobody joins on their word. Gives whom the invitations invited, to tell
    /// once the change commits. A link token someone else made, which they may have copied,
    /// stays: whoever manages the workspace revokes it.
    async fn withdraw_invites(
        &self,
        conn: &mut PgConnection,
        workspace_id: Uuid,
        user_id: Uuid,
        by: Uuid,
    ) -> Result<Vec<Uuid>, WorkspaceError> {
        let now = self.now();
        let invitees =
            workspace_invites::revoke_sent_by(&mut *conn, workspace_id, user_id, now).await?;
        let links =
            workspace_invites::revoke_links_made_by(&mut *conn, workspace_id, user_id, by, now)
                .await?;
        if !invitees.is_empty() || links > 0 {
            info!(
                workspace_id = %workspace_id,
                user_id = %user_id,
                invitations = invitees.len(),
                links,
                "invitations and invite links withdrawn"
            );
        }
        Ok(invitees)
    }

    /// Tells the inboxes of those whose invitations were withdrawn that they no longer
    /// wait.
    fn withdrawn(&self, invitees: Vec<Uuid>) {
        if invitees.is_empty() {
            return;
        }
        let mut notifier = self.notifier();
        for invitee in invitees {
            notifier.touched(invitee);
        }
        notifier.tell();
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
        webhooks::lapse(&mut *conn, workspace_id, user_id, self.now()).await?;
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

/// The member an action is aimed at: who, with what role, and what they are.
pub(super) struct Target {
    pub(super) id: Uuid,
    pub(super) role: WorkspaceRole,
    pub(super) kind: UserKind,
}

/// The member an action is aimed at. Whether the principal may know of them is the rules'
/// to say, as they would of anyone else.
pub(super) async fn target(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Target, WorkspaceError> {
    let member = repo::member(&mut *conn, workspace_id, user_id)
        .await?
        .ok_or(WorkspaceError::NotFound(NO_MEMBER))?;
    Ok(Target {
        id: user_id,
        role: member.role,
        kind: member.person.kind,
    })
}

/// Asks the rules about an action on a member, as the principal and, for an agent, its
/// owner stand to them.
pub(super) fn check_member(
    principal: &Principal,
    membership: &Membership,
    target: &Target,
    action: Action,
) -> Result<(), WorkspaceError> {
    let seen_by = |id: Option<Uuid>| Resource::Member {
        role: target.role,
        kind: target.kind,
        is_actor: id == Some(target.id),
    };
    membership
        .standing
        .authorize(
            action,
            seen_by(Some(principal.user.id)),
            seen_by(membership.standing.owner_id()),
        )
        .map_err(|denied| WorkspaceError::denied(denied, NO_MEMBER))
}
