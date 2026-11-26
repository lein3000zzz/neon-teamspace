//! `/api/v1/whoami`: whom a request acts as, whatever its credential: a browser session, a
//! person's token across the instance or bound to a workspace, or an agent's.

use axum::extract::State;
use neon_domain::auth::Credential;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::account::CredentialKind;
use super::dto::{Person, Scope, WorkspaceRole};
use super::error::{ApiError, ErrorBody};
use super::extract::Json;
use super::principal::Authenticated;

/// Whom a request acts as, and what it may do.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WhoAmI {
    /// A person, or an agent or integration with the person it acts for.
    pub identity: Person,
    pub credential: CredentialKind,
    /// A session has every scope its role allows; a token, its own.
    pub scopes: Vec<Scope>,
    /// The workspace a token is bound to, while its identity is in it; absent for a session
    /// and for a token across the instance.
    pub workspace: Option<BoundWorkspace>,
}

/// The workspace a token is bound to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BoundWorkspace {
    pub id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    #[schema(example = "design")]
    pub slug: String,
    /// The role the rules see the identity in: a token without the admin scope is a member
    /// at most.
    pub role: WorkspaceRole,
}

impl WhoAmI {
    pub(crate) fn new(
        principal: &neon_domain::auth::Principal,
        identity: neon_app::workspaces::Identity,
    ) -> Self {
        Self {
            identity: identity.person.into(),
            credential: match principal.credential {
                Credential::Session { .. } => CredentialKind::Session,
                Credential::Token { .. } => CredentialKind::Token,
            },
            scopes: principal.scopes().into_iter().map(Scope::from).collect(),
            workspace: identity.workspace.map(|(workspace, role)| BoundWorkspace {
                id: workspace.id,
                name: workspace.name,
                slug: workspace.slug,
                role: role.into(),
            }),
        }
    }
}

#[utoipa::path(
    get,
    path = "/whoami",
    tag = "account",
    summary = "Whom the request acts as",
    description = "Its identity, a person or an agent or integration with its owner, its scopes, \
                   and the workspace its token is bound to. Unlike `/account`, it answers every \
                   credential, an agent's token too.",
    responses(
        (status = OK, description = "Whom the request acts as.", body = WhoAmI),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn whoami(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<WhoAmI>, ApiError> {
    let identity = state.workspaces.identity(&principal).await?;
    Ok(Json(WhoAmI::new(&principal, identity)))
}
