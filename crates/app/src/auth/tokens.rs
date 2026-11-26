//! API tokens: a person's, working across the instance or in one workspace.

use neon_domain::auth::token::{
    API_TOKEN_PREFIX, InvalidTokenName, SHOWN_PREFIX_LENGTH, check_token_name, token_scopes,
};
use neon_domain::auth::{ApiToken, Principal, Scope, TokenHolder};
use neon_domain::settings::Secret;
use neon_store::{api_tokens, workspaces};
use sqlx::PgExecutor;
use time::OffsetDateTime;
use tracing::info;
use uuid::Uuid;

use super::{AuthError, AuthService, require_session};
use crate::crypto::{hash_secret, new_secret};

/// What a new token is to be: whom it acts as, where, and with what.
pub(crate) struct TokenSpec<'a> {
    pub(crate) user_id: Uuid,
    pub(crate) workspace_id: Option<Uuid>,
    pub(crate) name: &'a str,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) expires_at: Option<OffsetDateTime>,
}

/// Why a token cannot be made as asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum InvalidToken {
    #[error(transparent)]
    Name(#[from] InvalidTokenName),
    #[error("a token expires after now")]
    Expired,
}

/// Saves a new token, person's or agent's, once its name and expiry are checked, and
/// returns its secret, shown once, and what is kept of it: its hash.
pub(crate) async fn mint(
    executor: impl PgExecutor<'_>,
    spec: TokenSpec<'_>,
    now: OffsetDateTime,
) -> Result<Result<(Secret, ApiToken), InvalidToken>, sqlx::Error> {
    let name = match check_token_name(spec.name) {
        Ok(name) => name,
        Err(invalid) => return Ok(Err(invalid.into())),
    };
    if spec.expires_at.is_some_and(|expires| expires <= now) {
        return Ok(Err(InvalidToken::Expired));
    }
    let secret = new_secret(API_TOKEN_PREFIX);
    let token = ApiToken {
        id: Uuid::now_v7(),
        user_id: spec.user_id,
        workspace_id: spec.workspace_id,
        name,
        prefix: secret[..SHOWN_PREFIX_LENGTH].to_owned(),
        scopes: spec.scopes,
        created_at: now,
        expires_at: spec.expires_at,
        last_used_at: None,
    };
    api_tokens::insert(executor, &token, &hash_secret(&secret)).await?;
    Ok(Ok((Secret::new(secret), token)))
}

impl AuthService {
    /// Makes a token for the principal, able to do what its scopes allow and never more
    /// than its owner's role, across the instance or, bound, in `workspace_id` alone. The
    /// admin scope is for an instance admin's token across the instance, or for a token
    /// bound to a workspace its owner owns or administers, for that workspace's admin work.
    /// Its secret is returned once; only its hash is kept. Only from a browser session.
    pub async fn create_api_token<S: AsRef<str> + Sync>(
        &self,
        principal: &Principal,
        name: &str,
        expires_at: Option<OffsetDateTime>,
        scopes: &[S],
        workspace_id: Option<Uuid>,
    ) -> Result<(Secret, ApiToken), AuthError> {
        self.observe("create_api_token", async {
            require_session(principal)?;
            let holder = match workspace_id {
                None => TokenHolder::Account {
                    role: principal.user.role,
                },
                Some(workspace_id) => {
                    let role =
                        workspaces::member_role(self.pool(), workspace_id, principal.user.id)
                            .await?
                            .ok_or(AuthError::NotFound("no such workspace"))?;
                    TokenHolder::Workspace {
                        manages: role.manages(),
                    }
                }
            };
            let scopes = token_scopes(scopes, holder).map_err(|error| match error.refusal() {
                Some(refusal) => AuthError::Forbidden(refusal),
                None => AuthError::invalid(error),
            })?;
            let spec = TokenSpec {
                user_id: principal.user.id,
                workspace_id,
                name,
                scopes,
                expires_at,
            };
            let (secret, token) = mint(self.pool(), spec, self.now())
                .await?
                .map_err(AuthError::invalid)?;
            info!(
                user_id = %principal.user.id,
                token_id = %token.id,
                workspace_id = ?token.workspace_id,
                scopes = ?token.scopes,
                "API token created"
            );
            Ok((secret, token))
        })
        .await
    }

    /// The principal's tokens, oldest first.
    pub async fn list_api_tokens(&self, principal: &Principal) -> Result<Vec<ApiToken>, AuthError> {
        self.observe("list_api_tokens", async {
            Ok(api_tokens::list_for_user(self.pool(), principal.user.id).await?)
        })
        .await
    }

    /// Deletes one of the principal's tokens, which stops working at once. Only from a
    /// browser session.
    pub async fn revoke_api_token(&self, principal: &Principal, id: Uuid) -> Result<(), AuthError> {
        self.observe("revoke_api_token", async {
            require_session(principal)?;
            if !api_tokens::delete(self.pool(), principal.user.id, id).await? {
                return Err(AuthError::NotFound("no such token"));
            }
            info!(user_id = %principal.user.id, token_id = %id, "API token revoked");
            self.ended(principal.user.id);
            Ok(())
        })
        .await
    }
}
