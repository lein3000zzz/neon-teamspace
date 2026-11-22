//! API tokens.

use neon_domain::auth::token::{
    API_TOKEN_PREFIX, InvalidScopes, SHOWN_PREFIX_LENGTH, check_token_name, token_scopes,
};
use neon_domain::auth::{ApiToken, Principal};
use neon_domain::settings::Secret;
use neon_store::api_tokens;
use time::OffsetDateTime;
use tracing::info;
use uuid::Uuid;

use super::{AuthError, AuthService, require_session};
use crate::crypto::{hash_secret, new_secret};

impl AuthService {
    /// Makes a token for the principal, able to do what its scopes allow and never more
    /// than its owner's role. Its secret is returned once; only its hash is kept. Only
    /// from a browser session.
    pub async fn create_api_token<S: AsRef<str> + Sync>(
        &self,
        principal: &Principal,
        name: &str,
        expires_at: Option<OffsetDateTime>,
        scopes: &[S],
    ) -> Result<(Secret, ApiToken), AuthError> {
        self.observe("create_api_token", async {
            require_session(principal)?;
            let scopes =
                token_scopes(scopes, principal.user.role).map_err(|error| match error {
                    InvalidScopes::AdminOnly => {
                        AuthError::Forbidden("only an admin's token may have the admin scope")
                    }
                    other => AuthError::invalid(other),
                })?;
            let name = check_token_name(name).map_err(AuthError::invalid)?;
            let now = self.now();
            if expires_at.is_some_and(|expires| expires <= now) {
                return Err(AuthError::InvalidInput(
                    "a token expires after now".to_owned(),
                ));
            }

            let secret = new_secret(API_TOKEN_PREFIX);
            let token = ApiToken {
                id: Uuid::now_v7(),
                user_id: principal.user.id,
                name,
                prefix: secret[..SHOWN_PREFIX_LENGTH].to_owned(),
                scopes,
                created_at: now,
                expires_at,
                last_used_at: None,
            };
            api_tokens::insert(self.pool(), &token, &hash_secret(&secret)).await?;
            info!(
                user_id = %principal.user.id,
                token_id = %token.id,
                scopes = ?token.scopes,
                "API token created"
            );
            Ok((Secret::new(secret), token))
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
            Ok(())
        })
        .await
    }
}
