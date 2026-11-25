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

use crate::crypto::hash_secret;
use crate::settings::mcp;

/// The principal of an access token that works now, noting the grant's use at most once
/// a minute; `None` for one that is unknown, expired or whose grant does not work.
pub(crate) async fn authenticate(
    pool: &PgPool,
    settings: &Snapshot,
    token: &str,
    now: OffsetDateTime,
) -> sqlx::Result<Option<Principal>> {
    let Some(found) = repo::access_by_hash(pool, &hash_secret(token)).await? else {
        return Ok(None);
    };
    let principal = principal(&found, settings, now);
    let grant = &found.found.grant;
    let due = grant
        .last_used_at
        .is_none_or(|last| now - last >= TOUCH_INTERVAL);
    if principal.is_some()
        && due
        && let Err(error) = repo::touch_grant(pool, grant.id, now).await
    {
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
        .and_then(|found| principal(&found, settings, now)))
}

/// The principal of a token while it works: unexpired, its grant working, and some scope
/// left to it.
fn principal(found: &FoundAccess, settings: &Snapshot, now: OffsetDateTime) -> Option<Principal> {
    let FoundAccess { token, found } = found;
    if now >= token.expires_at
        || !found
            .standing
            .works(&found.grant, settings.get(mcp::IDENTITIES))
    {
        return None;
    }
    let scopes = narrowed(&token.scopes, &found.grant.scopes);
    (!scopes.is_empty()).then(|| Principal {
        user: found.user.clone(),
        credential: Credential::Token {
            id: token.id,
            name: found.client_name.clone(),
            scopes,
            workspace_id: Some(found.grant.workspace_id),
        },
    })
}
