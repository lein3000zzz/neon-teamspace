//! Invite links: made, listed, copied again and revoked by a workspace's owners and
//! admins, and used by signed-in accounts to join.

use neon_domain::auth::Principal;
use neon_domain::settings::Secret;
use neon_domain::workspaces::{
    Action, InviteLink, LINK_TOKEN_PREFIX, MAX_LINK_TOKEN_INPUT, Workspace, WorkspaceRole,
    check_link_expiry, check_link_uses,
};
use neon_store::{workspace_invites as repo, workspaces};
use time::OffsetDateTime;
use tracing::info;
use uuid::Uuid;

use super::lifecycle::Joined;
use super::{WorkspaceError, WorkspaceService, locked_membership, membership};
use crate::crypto::{OpenError, hash_secret, new_secret};
use crate::ratelimit::Bucket;

const NO_LINK: &str = "no such invite link";

/// What a new invite link is: the role it admits with, member or guest, how many people,
/// 10 unless told, and until when, a week from now unless told.
#[derive(Debug, Clone)]
pub struct LinkSpec {
    pub role: WorkspaceRole,
    pub max_uses: Option<i32>,
    pub expires_at: Option<OffsetDateTime>,
}

/// A new invite link with its token, shown to whoever made it.
#[derive(Debug, Clone)]
pub struct CreatedLink {
    pub link: InviteLink,
    pub token: Secret,
}

/// What an invite link admits to, shown to the signed-in account about to use it.
#[derive(Debug, Clone)]
pub struct LinkPreview {
    pub workspace: Workspace,
    /// The role it admits with.
    pub role: WorkspaceRole,
    pub expires_at: OffsetDateTime,
    /// The caller's role in the workspace, when they are in it already.
    pub member: Option<WorkspaceRole>,
}

impl WorkspaceService {
    /// Makes an invite link. Its token is kept as a hash, and sealed when a secret key is
    /// configured, so it can be copied again. Refused while the workspace is at its member
    /// limit.
    pub async fn create_invite_link(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        spec: LinkSpec,
    ) -> Result<CreatedLink, WorkspaceError> {
        self.observe("create_invite_link", async {
            let role = spec.role;
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            locked_membership(&mut tx, principal, workspace_id)
                .await?
                .check(principal, Action::CreateInviteLink { role })?;
            let max_uses = check_link_uses(spec.max_uses).map_err(WorkspaceError::invalid)?;
            let expires_at =
                check_link_expiry(spec.expires_at, now).map_err(WorkspaceError::invalid)?;
            self.check_room(&mut tx, workspace_id).await?;

            let id = Uuid::now_v7();
            let token = Secret::new(new_secret(LINK_TOKEN_PREFIX));
            let envelope = &self.inner.envelope;
            let sealed = if envelope.has_key() {
                let sealed = envelope
                    .seal(token.expose().as_bytes(), &link_aad(id))
                    .map_err(|error| {
                        WorkspaceError::Internal(format!("sealing an invite link: {error}"))
                    })?;
                Some(sealed)
            } else {
                None
            };
            let link = InviteLink {
                id,
                workspace_id,
                role,
                max_uses,
                uses: 0,
                created_by: Some(principal.user.id),
                created_at: now,
                expires_at,
                revoked_at: None,
                revoked_by: None,
                token_kept: sealed.is_some(),
            };
            repo::insert_link(
                &mut *tx,
                &link,
                &hash_secret(token.expose()),
                sealed.as_deref(),
            )
            .await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                link_id = %id,
                role = %role,
                max_uses,
                token_kept = link.token_kept,
                "invite link created"
            );
            Ok(CreatedLink { link, token })
        })
        .await
    }

    /// A workspace's invite links, newest first, without their tokens.
    pub async fn invite_links(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<InviteLink>, WorkspaceError> {
        self.observe("invite_links", async {
            let mut conn = self.pool().acquire().await?;
            membership(&mut conn, principal, workspace_id)
                .await?
                .check(principal, Action::ManageInviteLinks)?;
            Ok(repo::links(&mut *conn, workspace_id).await?)
        })
        .await
    }

    /// An invite link's token, opened from its sealed copy, to copy again.
    pub async fn invite_link_token(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Secret, WorkspaceError> {
        self.observe("invite_link_token", async {
            let mut conn = self.pool().acquire().await?;
            membership(&mut conn, principal, workspace_id)
                .await?
                .check(principal, Action::ManageInviteLinks)?;
            let sealed = repo::sealed_token(&mut *conn, workspace_id, id)
                .await?
                .ok_or(WorkspaceError::NotFound(NO_LINK))?
                .ok_or_else(|| {
                    WorkspaceError::TokenUnavailable(
                        "the token of this link was not kept: no secret key was configured \
                         when it was made"
                            .to_owned(),
                    )
                })?;
            let token = self
                .inner
                .envelope
                .open(&sealed, &link_aad(id))
                .map_err(|error| match error {
                    OpenError::NoKey | OpenError::UnknownKey(_) => {
                        WorkspaceError::TokenUnavailable(format!(
                            "the token cannot be opened: {error}"
                        ))
                    }
                    OpenError::Malformed | OpenError::Decrypt => WorkspaceError::Internal(format!(
                        "the sealed token of invite link {id} does not open: {error}"
                    )),
                })?;
            let token = String::from_utf8(token.to_vec()).map_err(|_| {
                WorkspaceError::Internal(format!("the token of invite link {id} is not text"))
            })?;
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                link_id = %id,
                "invite link copied"
            );
            Ok(Secret::new(token))
        })
        .await
    }

    /// Ends an invite link: it admits nobody more, and stays listed.
    pub async fn revoke_invite_link(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.observe("revoke_invite_link", async {
            let mut conn = self.pool().acquire().await?;
            membership(&mut conn, principal, workspace_id)
                .await?
                .check(principal, Action::ManageInviteLinks)?;
            if !repo::revoke_link(&mut *conn, workspace_id, id, principal.user.id, self.now())
                .await?
            {
                return Err(WorkspaceError::NotFound(NO_LINK));
            }
            info!(
                actor_id = %principal.user.id,
                workspace_id = %workspace_id,
                link_id = %id,
                "invite link revoked"
            );
            Ok(())
        })
        .await
    }

    /// What an invite link that still admits people admits to, without using it: the
    /// workspace and the role, and whether the caller is in the workspace already. It
    /// takes from the allowance that joining takes from, so tokens are tried no faster
    /// this way.
    pub async fn preview_link(
        &self,
        principal: &Principal,
        token: &Secret,
    ) -> Result<LinkPreview, WorkspaceError> {
        self.observe("preview_link", async {
            let user = principal.user.id;
            self.limit(Bucket::WorkspaceLinkJoin, &user.to_string())
                .await?;
            let link = self.open_link(token).await?;
            let mut conn = self.pool().acquire().await?;
            let workspace = workspaces::get(&mut *conn, link.workspace_id)
                .await?
                .ok_or(WorkspaceError::InvalidInviteLink)?;
            let member = workspaces::member_role(&mut *conn, workspace.id, user).await?;
            Ok(LinkPreview {
                workspace,
                role: link.role,
                expires_at: link.expires_at,
                member,
            })
        })
        .await
    }

    /// Joins the workspace of an invite link with its role: using the link is accepting
    /// it. The use is taken in the transaction that adds the member, so people racing for
    /// its last use cannot both have it. Someone in the workspace already takes no use.
    pub async fn join_with_link(
        &self,
        principal: &Principal,
        token: &Secret,
    ) -> Result<Joined, WorkspaceError> {
        self.observe("join_with_link", async {
            let user = principal.user.id;
            self.limit(Bucket::WorkspaceLinkJoin, &user.to_string())
                .await?;
            let link = self.open_link(token).await?;
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let workspace = workspaces::lock(&mut *tx, link.workspace_id)
                .await?
                .ok_or(WorkspaceError::InvalidInviteLink)?;
            if workspaces::member_role(&mut *tx, workspace.id, user)
                .await?
                .is_some()
            {
                return Err(WorkspaceError::AlreadyMember);
            }
            self.check_room(&mut tx, workspace.id).await?;
            if !repo::consume_link(&mut *tx, link.id, now).await? {
                return Err(WorkspaceError::InvalidInviteLink);
            }
            self.add_member(&mut tx, workspace.id, user, link.role)
                .await?;
            tx.commit().await?;
            info!(
                user_id = %user,
                workspace_id = %workspace.id,
                link_id = %link.id,
                role = %link.role,
                "joined by invite link"
            );
            Ok(Joined {
                workspace,
                role: link.role,
            })
        })
        .await
    }
}

impl WorkspaceService {
    /// The link of a token, while it still admits people.
    async fn open_link(&self, token: &Secret) -> Result<InviteLink, WorkspaceError> {
        let token = token.expose().trim();
        if token.is_empty() || token.len() > MAX_LINK_TOKEN_INPUT {
            return Err(WorkspaceError::InvalidInviteLink);
        }
        repo::open_link(self.pool(), &hash_secret(token), self.now())
            .await?
            .ok_or(WorkspaceError::InvalidInviteLink)
    }
}

/// Binds a sealed token to its link, so it does not open as another's.
fn link_aad(id: Uuid) -> Vec<u8> {
    format!("workspace-invite-link:{id}").into_bytes()
}
