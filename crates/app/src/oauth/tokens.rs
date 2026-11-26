//! The token endpoint, exchanging a code or a refresh token, and revocation (RFC 7009).

use std::time::Duration;

use neon_domain::auth::Scope;
use neon_domain::oauth::{
    ACCESS_TOKEN_PREFIX, Audience, Client, Endpoints, ErrorCode, Grant, REFRESH_TOKEN_PREFIX,
    TokenKind, check_verifier, narrowed, refreshed_scopes,
};
use neon_domain::settings::Secret;
use neon_store::oauth::{self as repo, FoundGrant, Token};
use sqlx::PgConnection;
use tracing::{info, warn};
use uuid::Uuid;

use super::{OAuthError, OAuthService};
use crate::crypto::{hash_secret, new_secret, pkce_matches};
use crate::settings::mcp;

/// The longest code or token read. Longer text matches nothing.
const MAX_SECRET: usize = 128;

/// A request to the token endpoint, as its form has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    pub client_id: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

/// What the token endpoint gives: an access token, and a refresh token that works once.
/// Their secrets exist only here and in the answer.
#[derive(Debug, Clone)]
pub struct IssuedTokens {
    pub access_token: Secret,
    pub refresh_token: Secret,
    /// How long the access token works.
    pub expires_in: Duration,
    pub scopes: Vec<Scope>,
}

fn invalid_request(description: &str) -> OAuthError {
    OAuthError::protocol(ErrorCode::InvalidRequest, description)
}

fn invalid_grant(description: &str) -> OAuthError {
    OAuthError::protocol(ErrorCode::InvalidGrant, description)
}

fn invalid_target(description: &str) -> OAuthError {
    OAuthError::protocol(ErrorCode::InvalidTarget, description)
}

impl OAuthService {
    /// Exchanges a code, with its PKCE verifier, or a refresh token, for an access token
    /// and a new refresh token. A code or refresh token used a second time means one of the
    /// two uses was a copy: its grant is revoked with every token issued from it, and the
    /// person connects the client again.
    pub async fn token(
        &self,
        request: &TokenRequest,
        endpoints: &Endpoints,
    ) -> Result<IssuedTokens, OAuthError> {
        self.observe("token", async {
            let client_id = request
                .client_id
                .as_deref()
                .ok_or_else(|| invalid_request("the request names no client_id"))?;
            let client = self.client(client_id).await?;
            let audience = match request.resource.as_deref() {
                None => None,
                Some(indicator) => Some(endpoints.audience_of(indicator).ok_or_else(|| {
                    invalid_target(
                        "the resource is this server's MCP endpoint, /mcp, or its API, /api/v1",
                    )
                })?),
            };
            let mut tx = self.pool().begin().await?;
            let issued = match request.grant_type.as_deref() {
                Some("authorization_code") => {
                    self.exchange(&mut tx, &client, request, audience).await
                }
                Some("refresh_token") => self.refresh(&mut tx, &client, request, audience).await,
                Some(_) => Err(Refused(OAuthError::protocol(
                    ErrorCode::UnsupportedGrantType,
                    "the grant types are authorization_code and refresh_token",
                ))),
                None => Err(Refused(invalid_request("the request names no grant_type"))),
            };
            let (grant, scopes) = match issued {
                Ok(issued) => issued,
                Err(Reused(grant_id, what)) => {
                    let revoked = repo::delete_grant(&mut *tx, grant_id).await?;
                    tx.commit().await?;
                    warn!(%grant_id, client_id = %client.id, "OAuth grant revoked: {what} used twice");
                    if let Some(grant) = revoked {
                        self.ended(grant.user_id);
                    }
                    return Err(invalid_grant(&format!(
                        "the {what} was used already: everything issued from it is revoked"
                    )));
                }
                Err(Refused(error)) => return Err(error),
            };
            let tokens = self.issue(&mut tx, &grant, scopes).await?;
            tx.commit().await?;
            Ok(tokens)
        })
        .await
    }

    /// The code's grant and scopes, the code marked used. Refused, the code stays as it
    /// was, so a client that sent a wrong parameter may try again within its minute. A
    /// resource named must be the one the code was given for.
    ///
    /// It locks the grant, then the code, the order in which revoking a grant locks them,
    /// so that an exchange and a revocation or a second exchange take turns.
    async fn exchange(
        &self,
        tx: &mut PgConnection,
        client: &Client,
        request: &TokenRequest,
        audience: Option<Audience>,
    ) -> Result<(Grant, Vec<Scope>), Outcome> {
        let (Some(code), Some(redirect_uri), Some(verifier)) = (
            request.code.as_deref(),
            request.redirect_uri.as_deref(),
            request.code_verifier.as_deref(),
        ) else {
            return Err(Refused(invalid_request(
                "a code needs its redirect_uri and code_verifier",
            )));
        };
        if code.len() > MAX_SECRET {
            return Err(Refused(invalid_grant("no such code")));
        }
        let hash = hash_secret(code);
        let locked = repo::lock_grant_of_code(&mut *tx, &hash)
            .await
            .map_err(failed)?;
        let found = match locked {
            Some(_) => repo::lock_code(&mut *tx, &hash).await.map_err(failed)?,
            None => None,
        };
        let Some(found) = found else {
            return Err(Refused(invalid_grant("no such code")));
        };
        if found.used_at.is_some() {
            return Err(Reused(found.grant_id, "code"));
        }
        let now = self.now();
        let grant = self.working_grant(&mut *tx, found.grant_id, client).await?;
        if now >= found.expires_at {
            return Err(Refused(invalid_grant("the code expired")));
        }
        if redirect_uri != found.redirect_uri {
            return Err(Refused(invalid_grant(
                "the redirect_uri is not the one the code was given to",
            )));
        }
        if !check_verifier(verifier) || !pkce_matches(verifier, &found.challenge) {
            return Err(Refused(invalid_grant("the code_verifier does not match")));
        }
        if audience.is_some_and(|audience| audience != found.audience) {
            return Err(Refused(invalid_target(
                "the resource is not the one the code was given for",
            )));
        }
        repo::use_code(&mut *tx, &hash, now).await.map_err(failed)?;
        let scopes = narrowed(&found.scopes, &grant.scopes);
        if scopes.is_empty() {
            return Err(Refused(invalid_grant(
                "none of the scopes is granted any more",
            )));
        }
        Ok((grant, scopes))
    }

    /// The refresh token's grant and the scopes of its successor, the token marked used.
    /// Refused, it stays as it was, so a client whose agent is suspended for now may
    /// refresh once it resumes. A resource named must be the one the grant was last
    /// allowed for, as its scopes are those last allowed.
    ///
    /// It locks the grant, then the refresh token, as [`exchange`](Self::exchange) does.
    async fn refresh(
        &self,
        tx: &mut PgConnection,
        client: &Client,
        request: &TokenRequest,
        audience: Option<Audience>,
    ) -> Result<(Grant, Vec<Scope>), Outcome> {
        let Some(secret) = request.refresh_token.as_deref() else {
            return Err(Refused(invalid_request(
                "the request names no refresh_token",
            )));
        };
        if !secret.starts_with(REFRESH_TOKEN_PREFIX) || secret.len() > MAX_SECRET {
            return Err(Refused(invalid_grant("no such refresh token")));
        }
        let hash = hash_secret(secret);
        let locked = repo::lock_grant_of_refresh(&mut *tx, &hash)
            .await
            .map_err(failed)?;
        let found = match locked {
            Some(_) => repo::lock_refresh(&mut *tx, &hash).await.map_err(failed)?,
            None => None,
        };
        let Some(old) = found else {
            return Err(Refused(invalid_grant("no such refresh token")));
        };
        if old.used_at.is_some() {
            return Err(Reused(old.grant_id, "refresh token"));
        }
        let now = self.now();
        let grant = self.working_grant(&mut *tx, old.grant_id, client).await?;
        if now >= old.expires_at {
            return Err(Refused(invalid_grant("the refresh token expired")));
        }
        if audience.is_some_and(|audience| audience != grant.audience) {
            return Err(Refused(invalid_target(
                "the resource is not the one the grant was allowed for",
            )));
        }
        let scopes = refreshed_scopes(request.scope.as_deref(), &old.scopes, &grant.scopes)
            .map_err(|problem| Refused(OAuthError::Protocol(problem)))?;
        repo::use_token(&mut *tx, old.id, now)
            .await
            .map_err(failed)?;
        Ok((grant, scopes))
    }

    /// The grant of a code or refresh token, while it is the client's and works.
    async fn working_grant(
        &self,
        conn: &mut PgConnection,
        id: Uuid,
        client: &Client,
    ) -> Result<Grant, Outcome> {
        let found = repo::found_grant(&mut *conn, id).await.map_err(failed)?;
        let Some(FoundGrant {
            grant, standing, ..
        }) = found
        else {
            return Err(Refused(invalid_grant("the grant was revoked")));
        };
        if grant.client_id != client.id {
            return Err(Refused(invalid_grant("the grant is another client's")));
        }
        if !standing.works(&grant, self.settings().get(mcp::IDENTITIES)) {
            return Err(Refused(invalid_grant(
                "the grant does not work now: its person cannot sign in or is out of the \
                 workspace, or the agent it acts as is suspended or switched off",
            )));
        }
        Ok(grant)
    }

    /// A new access token and refresh token of `grant`.
    async fn issue(
        &self,
        conn: &mut PgConnection,
        grant: &Grant,
        scopes: Vec<Scope>,
    ) -> Result<IssuedTokens, OAuthError> {
        let now = self.now();
        let access = new_secret(ACCESS_TOKEN_PREFIX);
        let refresh = new_secret(REFRESH_TOKEN_PREFIX);
        let access_ttl = self.access_ttl();
        for (secret, kind, ttl) in [
            (&access, TokenKind::Access, access_ttl),
            (&refresh, TokenKind::Refresh, self.refresh_ttl()),
        ] {
            let token = Token {
                id: Uuid::now_v7(),
                grant_id: grant.id,
                kind,
                scopes: scopes.clone(),
                created_at: now,
                expires_at: now + ttl,
                used_at: None,
            };
            repo::insert_token(&mut *conn, &hash_secret(secret), &token).await?;
        }
        repo::touch_grant(&mut *conn, grant.id, now).await?;
        repo::touch_client(&mut *conn, grant.client_id, now).await?;
        info!(grant_id = %grant.id, client_id = %grant.client_id, scopes = ?scopes, "OAuth tokens issued");
        Ok(IssuedTokens {
            access_token: Secret::new(access),
            refresh_token: Secret::new(refresh),
            expires_in: access_ttl,
            scopes,
        })
    }

    /// Revokes the grant of an access or refresh token, as its client asks when it signs
    /// out (RFC 7009): every token of it stops working. A token that is unknown, or
    /// another client's, changes nothing, and the answer is the same.
    pub async fn revoke(
        &self,
        token: Option<&str>,
        client_id: Option<&str>,
    ) -> Result<(), OAuthError> {
        self.observe("revoke", async {
            let token = token.ok_or_else(|| invalid_request("the request names no token"))?;
            let client_id =
                client_id.ok_or_else(|| invalid_request("the request names no client_id"))?;
            let client = self.client(client_id).await?;
            let ours = token.starts_with(ACCESS_TOKEN_PREFIX)
                || token.starts_with(REFRESH_TOKEN_PREFIX);
            if !ours || token.len() > MAX_SECRET {
                return Ok(());
            }
            let Some(found) = repo::token(self.pool(), &hash_secret(token)).await? else {
                return Ok(());
            };
            let mut tx = self.pool().begin().await?;
            let owned = repo::grant(&mut *tx, found.grant_id)
                .await?
                .is_some_and(|grant| grant.client_id == client.id);
            if !owned {
                return Ok(());
            }
            let revoked = repo::delete_grant(&mut *tx, found.grant_id).await?;
            tx.commit().await?;
            if let Some(grant) = revoked {
                info!(grant_id = %grant.id, client_id = %client.id, "OAuth grant revoked by its client");
                self.ended(grant.user_id);
            }
            Ok(())
        })
        .await
    }

    /// The client of a token request, which public clients name to identify themselves.
    async fn client(&self, client_id: &str) -> Result<Client, OAuthError> {
        let unknown = || OAuthError::protocol(ErrorCode::InvalidClient, "no such client");
        let id = Uuid::try_parse(client_id).map_err(|_| unknown())?;
        repo::client(self.pool(), id).await?.ok_or_else(unknown)
    }
}

/// Why an exchange gave no tokens.
enum Outcome {
    /// The code or refresh token of the grant was used before.
    Reused(Uuid, &'static str),
    Refused(OAuthError),
}

use Outcome::{Refused, Reused};

fn failed(error: sqlx::Error) -> Outcome {
    Refused(OAuthError::Database(error))
}
