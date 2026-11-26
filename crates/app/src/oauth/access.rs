//! OAuth access tokens as bearer credentials: the principal each stands for, which the
//! auth service's bearer authentication gives as it gives an API token's.
//!
//! The principal is the grant's identity, the person or their agent, with a token
//! credential bound to the grant's workspace whose scopes are those the access token was
//! issued with that the grant still has. So the rules treat it as they treat an API token
//! bound to that workspace: an agent's access meets its owner's, a token without the admin
//! scope manages nothing, and the account's and instance admins' routes refuse it.

use neon_domain::auth::session::TOUCH_INTERVAL;
use neon_domain::auth::{Credential, Principal};
use neon_domain::oauth::narrowed;
use neon_domain::settings::Snapshot;
use neon_store::oauth::{self as repo, FoundAccess};
use sqlx::PgPool;
use time::OffsetDateTime;
use tracing::warn;
use uuid::Uuid;

use crate::auth::AuthError;
use crate::crypto::hash_secret;
use crate::settings::mcp;

/// The principal of an access token that works now, noting the grant's use at most once
/// a minute. One that is unknown, expired or whose grant does not work is unauthenticated,
/// but for an unexpired one whose agent alone stopped working, which names why.
pub(crate) async fn authenticate(
    pool: &PgPool,
    settings: &Snapshot,
    token: &str,
    now: OffsetDateTime,
) -> Result<Principal, AuthError> {
    let Some(found) = repo::access_by_hash(pool, &hash_secret(token)).await? else {
        return Err(AuthError::Unauthenticated);
    };
    let principal = principal(&found, settings, now)?;
    let grant = &found.found.grant;
    let due = grant
        .last_used_at
        .is_none_or(|last| now - last >= TOUCH_INTERVAL);
    if due && let Err(error) = repo::touch_grant(pool, grant.id, now).await {
        warn!(grant_id = %grant.id, %error, "recording an OAuth grant's use failed");
    }
    Ok(principal)
}

/// The principal of the access token of `id` as it stands now, read again, or `None` once
/// it stopped working. It does not count as a use.
pub(crate) async fn recheck(
    pool: &PgPool,
    settings: &Snapshot,
    id: Uuid,
    now: OffsetDateTime,
) -> sqlx::Result<Option<Principal>> {
    Ok(repo::access_by_id(pool, id)
        .await?
        .and_then(|found| principal(&found, settings, now).ok()))
}

/// The principal of a token while it works: unexpired, its grant working, and some scope
/// left to it. An unexpired token whose grant stopped for its agent's status alone names
/// that status.
fn principal(
    found: &FoundAccess,
    settings: &Snapshot,
    now: OffsetDateTime,
) -> Result<Principal, AuthError> {
    let FoundAccess { token, found } = found;
    let identities = settings.get(mcp::IDENTITIES);
    if now >= token.expires_at {
        return Err(AuthError::Unauthenticated);
    }
    if !found.standing.works(&found.grant, identities) {
        return Err(found
            .standing
            .stopped_agent(&found.grant, identities)
            .and_then(AuthError::agent_stopped)
            .unwrap_or(AuthError::Unauthenticated));
    }
    let scopes = narrowed(&token.scopes, &found.grant.scopes);
    if scopes.is_empty() {
        return Err(AuthError::Unauthenticated);
    }
    Ok(Principal {
        user: found.user.clone(),
        credential: Credential::Token {
            id: token.id,
            name: found.client_name.clone(),
            scopes,
            workspace_id: Some(found.grant.workspace_id),
        },
    })
}
