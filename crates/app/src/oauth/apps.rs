//! Connected apps: the grants a person made, and those that act as a workspace's agents,
//! with revoking each.

use neon_domain::auth::Principal;
use neon_domain::workspaces::{Action, Resource};
use neon_store::agents as agent_store;
use neon_store::oauth as repo;
use tracing::info;
use uuid::Uuid;

use super::{OAuthError, OAuthService};
use crate::standing::Standing;

/// A grant as the lists of connected apps show it.
pub use neon_store::oauth::Listed as ConnectedApp;

const NO_APP: &str = "no such connected app";
const NO_WORKSPACE: &str = "no such workspace";

impl OAuthService {
    /// The clients the person connected, as themselves or as their agents, last used
    /// first.
    pub async fn connected_apps(
        &self,
        principal: &Principal,
    ) -> Result<Vec<ConnectedApp>, OAuthError> {
        self.observe("connected_apps", async {
            Ok(repo::grants_of_person(self.pool(), principal.user.id).await?)
        })
        .await
    }

    /// Revokes one of the person's grants, from a browser session: its tokens stop at
    /// once.
    pub async fn disconnect_app(&self, principal: &Principal, id: Uuid) -> Result<(), OAuthError> {
        self.observe("disconnect_app", async {
            if !principal.via_session() {
                return Err(OAuthError::Forbidden(
                    "disconnecting an app needs a browser session: tokens cannot",
                ));
            }
            let mut tx = self.pool().begin().await?;
            let mine = repo::grant(&mut *tx, id)
                .await?
                .is_some_and(|grant| grant.person_id == principal.user.id);
            if !mine {
                return Err(OAuthError::NotFound(NO_APP));
            }
            let revoked = repo::delete_grant(&mut *tx, id).await?;
            tx.commit().await?;
            if let Some(grant) = revoked {
                info!(user_id = %principal.user.id, grant_id = %id, "OAuth grant revoked");
                self.grant_changed(grant.user_id);
            }
            Ok(())
        })
        .await
    }

    /// The grants that act as the workspace's agents that the principal may see the
    /// credentials of: every agent's for its owners and admins, their own agents' for the
    /// other members, as for agents' API tokens.
    pub async fn workspace_apps(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<ConnectedApp>, OAuthError> {
        self.observe("workspace_apps", async {
            let mut conn = self.pool().acquire().await?;
            let standing = Standing::read(&mut conn, principal, workspace_id).await?;
            if standing.role().is_none() {
                return Err(OAuthError::NotFound(NO_WORKSPACE));
            }
            let listed = repo::agent_grants_of_workspace(&mut *conn, workspace_id).await?;
            Ok(listed
                .into_iter()
                .filter(|app| {
                    let own = app
                        .identity
                        .owner
                        .as_ref()
                        .is_some_and(|owner| owner.id == principal.user.id);
                    let resource = Resource::Agent {
                        kind: app.identity.kind,
                        own,
                    };
                    standing
                        .authorize(Action::ListAgentTokens, resource, theirs(&standing, app))
                        .is_ok()
                })
                .collect())
        })
        .await
    }

    /// Revokes a grant that acts as one of the workspace's agents, for its owner and the
    /// workspace's owners and admins, from a browser session, as for agents' API tokens.
    pub async fn disconnect_workspace_app(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), OAuthError> {
        self.observe("disconnect_workspace_app", async {
            let mut tx = self.pool().begin().await?;
            let standing = Standing::read(&mut tx, principal, workspace_id).await?;
            if standing.role().is_none() {
                return Err(OAuthError::NotFound(NO_WORKSPACE));
            }
            let grant = repo::grant(&mut *tx, id)
                .await?
                .filter(|grant| grant.workspace_id == workspace_id && grant.acts_as_agent())
                .ok_or(OAuthError::NotFound(NO_APP))?;
            let agent = agent_store::get(&mut *tx, workspace_id, grant.user_id)
                .await?
                .ok_or(OAuthError::NotFound(NO_APP))?;
            let owner = agent.owner.as_ref().map(|owner| owner.id);
            let owned_by = |id: Option<Uuid>| Resource::Agent {
                kind: agent.kind,
                own: id.is_some() && id == owner,
            };
            standing
                .authorize(
                    Action::RevokeAgentToken,
                    owned_by(Some(principal.user.id)),
                    owned_by(standing.owner_id()),
                )
                .map_err(|denied| OAuthError::denied(denied, NO_APP))?;
            repo::delete_grant(&mut *tx, id).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                workspace_id = %workspace_id,
                grant_id = %id,
                agent_id = %grant.user_id,
                "OAuth grant of an agent revoked"
            );
            self.grant_changed(grant.user_id);
            Ok(())
        })
        .await
    }
}

/// The agent of a listed grant as the principal's owner stands to it, for an agent that
/// asks: owned by that owner or not.
fn theirs(standing: &Standing, app: &ConnectedApp) -> Resource {
    let owner = app.identity.owner.as_ref().map(|owner| owner.id);
    Resource::Agent {
        kind: app.identity.kind,
        own: standing.owner_id().is_some() && standing.owner_id() == owner,
    }
}
