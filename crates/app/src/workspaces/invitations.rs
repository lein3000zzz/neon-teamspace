//! Invitations into a workspace: sent to an account by its address, listed and revoked by
//! the workspace's owners and admins, accepted or declined by the account.

use neon_domain::auth::{Email, Principal};
use neon_domain::workspaces::{
    Action, INVITATION_TTL, Invitation, InvitationState, Person, Workspace, WorkspaceRole,
};
use neon_store::{is_unique_violation, users, workspace_invites as repo, workspaces};
use tracing::info;
use uuid::Uuid;

use super::lifecycle::Joined;
use super::{WorkspaceError, WorkspaceService, locked_membership, membership};
use crate::ratelimit::Bucket;

const NO_INVITATION: &str = "no such pending invitation";

/// An invitation a workspace sent, with who it is for.
#[derive(Debug, Clone)]
pub struct SentInvitation {
    pub invitation: Invitation,
    pub invitee: Person,
}

/// An invitation an account waits on, with its workspace and who sent it, while their
/// account exists.
#[derive(Debug, Clone)]
pub struct ReceivedInvitation {
    pub invitation: Invitation,
    pub workspace: Workspace,
    pub invited_by: Option<Person>,
}

impl WorkspaceService {
    /// Invites the account of an address to join with `role`, admin, member or guest, for
    /// [`INVITATION_TTL`]. Nothing is joined until the account accepts. An address with no
    /// account that can sign in is refused with [`WorkspaceError::NoAccount`]; as that
    /// answer tells whether an address has an account here, each inviter's lookups are
    /// rate-limited.
    pub async fn invite(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        email: &str,
        role: WorkspaceRole,
    ) -> Result<SentInvitation, WorkspaceError> {
        self.observe("invite", async {
            {
                let mut conn = self.pool().acquire().await?;
                membership(&mut conn, principal, workspace_id)
                    .await?
                    .check(principal, Action::Invite { role })?;
            }
            self.limit(Bucket::WorkspaceInvite, &principal.user.id.to_string())
                .await?;
            let email = Email::parse(email).map_err(WorkspaceError::invalid)?;
            let invitee = match users::by_email(self.pool(), email.normalized()).await? {
                Some((user, _)) if user.is_verified() && !user.is_disabled() => user,
                _ => return Err(WorkspaceError::NoAccount),
            };

            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let membership = locked_membership(&mut tx, principal, workspace_id).await?;
            membership.check(principal, Action::Invite { role })?;
            if workspaces::member_role(&mut *tx, workspace_id, invitee.id)
                .await?
                .is_some()
            {
                return Err(WorkspaceError::AlreadyMember);
            }
            self.check_room(&mut tx, workspace_id).await?;
            repo::set_aside_expired(&mut *tx, workspace_id, invitee.id, now).await?;
            let invitation = Invitation {
                id: Uuid::now_v7(),
                workspace_id,
                user_id: invitee.id,
                role,
                invited_by: Some(principal.user.id),
                created_at: now,
                expires_at: now + INVITATION_TTL,
                state: InvitationState::Pending,
                decided_at: None,
            };
            repo::insert(&mut *tx, &invitation).await.map_err(|error| {
                if is_unique_violation(&error, repo::ALREADY_INVITED) {
                    WorkspaceError::AlreadyInvited
                } else {
                    WorkspaceError::Database(error)
                }
            })?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                invitation_id = %invitation.id,
                user_id = %invitee.id,
                role = %role,
                "invitation sent"
            );
            Ok(SentInvitation {
                invitation,
                invitee: Person {
                    id: invitee.id,
                    hue_override: invitee.hue_override,
                    display_name: invitee.display_name,
                    email: invitee.email.as_str().to_owned(),
                },
            })
        })
        .await
    }

    /// The invitations a workspace sent that wait for an answer, newest first.
    pub async fn sent_invitations(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<SentInvitation>, WorkspaceError> {
        self.observe("sent_invitations", async {
            let mut conn = self.pool().acquire().await?;
            membership(&mut conn, principal, workspace_id)
                .await?
                .check(principal, Action::ManageInvitations)?;
            Ok(
                repo::pending_for_workspace(&mut *conn, workspace_id, self.now())
                    .await?
                    .into_iter()
                    .map(|(invitation, invitee)| SentInvitation {
                        invitation,
                        invitee,
                    })
                    .collect(),
            )
        })
        .await
    }

    /// Withdraws a pending invitation, which the account can no longer accept.
    pub async fn revoke_invitation(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("revoke_invitation", async {
            let mut tx = self.pool().begin().await?;
            locked_membership(&mut tx, principal, workspace_id)
                .await?
                .check(principal, Action::ManageInvitations)?;
            let revoked = repo::decide(
                &mut *tx,
                workspace_id,
                id,
                InvitationState::Revoked,
                self.now(),
            )
            .await?;
            if !revoked {
                return Err(WorkspaceError::NotFound(NO_INVITATION));
            }
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                invitation_id = %id,
                "invitation revoked"
            );
            Ok(())
        })
        .await
    }

    /// The invitations the principal waits on, newest first.
    pub async fn my_invitations(
        &self,
        principal: &Principal,
    ) -> Result<Vec<ReceivedInvitation>, WorkspaceError> {
        self.observe("my_invitations", async {
            Ok(
                repo::pending_for_user(self.pool(), principal.user.id, self.now())
                    .await?
                    .into_iter()
                    .map(|(invitation, workspace, invited_by)| ReceivedInvitation {
                        invitation,
                        workspace,
                        invited_by,
                    })
                    .collect(),
            )
        })
        .await
    }

    /// Accepts one of the principal's invitations: they join with its role, and the
    /// default teamspace unless as a guest. Refused while the workspace is at its member
    /// limit; the invitation then stays, to accept once there is room.
    pub async fn accept_invitation(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Joined, WorkspaceError> {
        self.observe("accept_invitation", async {
            let user = principal.user.id;
            let now = self.now();
            // The workspace's lock comes before the invitation's, in the order every
            // change to the members takes them.
            let workspace_id = self.open_invitation(id, user).await?.workspace_id;
            let mut tx = self.pool().begin().await?;
            let workspace = workspaces::lock(&mut *tx, workspace_id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_INVITATION))?;
            let invitation = repo::lock_open(&mut *tx, id, user, now)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_INVITATION))?;
            if workspaces::member_role(&mut *tx, workspace_id, user)
                .await?
                .is_some()
            {
                return Err(WorkspaceError::AlreadyMember);
            }
            self.check_room(&mut tx, workspace_id).await?;
            self.add_member(&mut tx, workspace_id, user, invitation.role)
                .await?;
            repo::decide(&mut *tx, workspace_id, id, InvitationState::Accepted, now).await?;
            tx.commit().await?;
            info!(
                user_id = %user,
                workspace_id = %workspace_id,
                invitation_id = %id,
                role = %invitation.role,
                "invitation accepted"
            );
            Ok(Joined {
                workspace,
                role: invitation.role,
            })
        })
        .await
    }

    /// Declines one of the principal's invitations.
    pub async fn decline_invitation(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("decline_invitation", async {
            let invitation = self.open_invitation(id, principal.user.id).await?;
            let declined = repo::decide(
                self.pool(),
                invitation.workspace_id,
                id,
                InvitationState::Declined,
                self.now(),
            )
            .await?;
            if !declined {
                return Err(WorkspaceError::NotFound(NO_INVITATION));
            }
            info!(
                user_id = %principal.user.id,
                workspace_id = %invitation.workspace_id,
                invitation_id = %id,
                "invitation declined"
            );
            Ok(())
        })
        .await
    }

    /// One of the account's invitations, while it waits for an answer.
    async fn open_invitation(&self, id: Uuid, user: Uuid) -> Result<Invitation, WorkspaceError> {
        repo::open(self.pool(), id, user, self.now())
            .await?
            .ok_or(WorkspaceError::NotFound(NO_INVITATION))
    }
}
