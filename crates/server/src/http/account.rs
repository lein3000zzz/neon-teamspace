//! `/api/v1/account`: the signed-in user's own account, and the invitations into
//! workspaces waiting for them.

use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::SET_COOKIE;
use axum::response::{IntoResponse, Response};
use neon_app::auth::ProfileUpdate;
use neon_domain::auth::preferences::{
    Mode as DomainMode, Palette as DomainPalette, UiFont as DomainUiFont,
};
use neon_domain::auth::{Credential, Preferences as DomainPreferences};
use neon_domain::settings::Secret;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::dto::{Person, Scope, User, WorkspaceRole, nullable};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path};
use super::principal::Authenticated;
use super::workspaces::WorkspaceSummary;

/// How a request proved who it is.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Session,
    Token,
}

/// The signed-in user, and what this request may do.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Me {
    #[serde(flatten)]
    pub user: User,
    pub credential: CredentialKind,
    /// A session has every scope its role allows; a token, its own.
    pub scopes: Vec<Scope>,
}

#[utoipa::path(
    get,
    path = "/account/me",
    tag = "account",
    summary = "The signed-in user",
    description = "Needs a session or an API token with the read scope.",
    responses(
        (status = OK, description = "The user.", body = Me),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_me(Authenticated(principal): Authenticated) -> Json<Me> {
    let credential = match principal.credential {
        Credential::Session { .. } => CredentialKind::Session,
        Credential::Token { .. } => CredentialKind::Token,
    };
    Json(Me {
        scopes: principal.scopes().into_iter().map(Scope::from).collect(),
        credential,
        user: principal.user.into(),
    })
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMe {
    /// At most 100 characters; blank takes the address's local part.
    #[serde(default)]
    #[schema(example = "Ada Lovelace")]
    pub display_name: Option<String>,
    /// The hue to be drawn in, 0 to 359; null goes back to the one the id hashes to.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>, minimum = 0, maximum = 359)]
    pub hue_override: Option<Option<i64>>,
}

#[utoipa::path(
    patch,
    path = "/account/me",
    tag = "account",
    summary = "Rename the signed-in user, or pin their hue",
    description = "Changes what the body names, all of it or, when any part is invalid, none. \
                   Needs a session or an API token with the write scope.",
    request_body = UpdateMe,
    responses(
        (status = OK, description = "The user.", body = User),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_me(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<UpdateMe>,
) -> Result<Json<User>, ApiError> {
    let update = ProfileUpdate {
        display_name: body.display_name,
        hue_override: body.hue_override,
    };
    let user = state.auth.update_profile(&principal, update).await?;
    Ok(Json(user.into()))
}

/// An invitation the signed-in user waits on.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReceivedInvitation {
    pub id: Uuid,
    pub workspace: InvitingWorkspace,
    /// What they would join as: admin, member or guest.
    pub role: WorkspaceRole,
    /// Who invited them; absent once that account is gone.
    pub invited_by: Option<Person>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// The workspace an invitation is to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitingWorkspace {
    pub id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    #[schema(example = "design")]
    pub slug: String,
    pub icon: Option<String>,
    #[schema(minimum = 0, maximum = 359)]
    pub hue: u16,
}

impl InvitingWorkspace {
    pub(super) fn new(workspace: &neon_domain::workspaces::Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name.clone(),
            slug: workspace.slug.clone(),
            icon: workspace.icon.clone(),
            hue: workspace.hue(),
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReceivedInvitationList {
    /// Newest first.
    pub invitations: Vec<ReceivedInvitation>,
}

#[utoipa::path(
    get,
    path = "/account/invitations",
    tag = "account",
    summary = "List the invitations waiting for the user",
    description = "Invitations into workspaces that the user has not answered and that have not \
                   expired. Needs a session or an API token with the read scope.",
    responses(
        (status = OK, description = "The invitations.", body = ReceivedInvitationList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_my_invitations(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<ReceivedInvitationList>, ApiError> {
    let received = state.workspaces.my_invitations(&principal).await?;
    Ok(Json(ReceivedInvitationList {
        invitations: received
            .into_iter()
            .map(|entry| ReceivedInvitation {
                id: entry.invitation.id,
                workspace: InvitingWorkspace::new(&entry.workspace),
                role: entry.invitation.role.into(),
                invited_by: entry.invited_by.map(Person::from),
                created_at: entry.invitation.created_at,
                expires_at: entry.invitation.expires_at,
            })
            .collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/account/invitations/{id}/accept",
    tag = "account",
    summary = "Accept an invitation",
    description = "Joins the workspace with the invitation's role, and its default teamspace \
                   unless as a guest. Needs a session or an API token with the write scope.",
    params(("id" = Uuid, Path, description = "The invitation's id.")),
    responses(
        (status = OK, description = "The workspace joined.", body = WorkspaceSummary),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such invitation waiting for the user.", body = ErrorBody),
        (status = CONFLICT, description = "already_member, or member_limit: the workspace is full; the invitation stays.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn accept_invitation(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkspaceSummary>, ApiError> {
    let joined = state.workspaces.accept_invitation(&principal, id).await?;
    Ok(Json(joined.into()))
}

#[utoipa::path(
    post,
    path = "/account/invitations/{id}/decline",
    tag = "account",
    summary = "Decline an invitation",
    description = "Needs a session or an API token with the write scope.",
    params(("id" = Uuid, Path, description = "The invitation's id.")),
    responses(
        (status = NO_CONTENT, description = "Declined."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such invitation waiting for the user.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn decline_invitation(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.workspaces.decline_invitation(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePassword {
    #[schema(format = Password)]
    pub current_password: String,
    #[schema(format = Password)]
    pub new_password: String,
}

#[utoipa::path(
    post,
    path = "/account/password",
    tag = "account",
    summary = "Change the password",
    description = "Needs the current password and a browser session; API tokens cannot. Signs \
                   out every other session, and mails the account that its password changed.",
    request_body = ChangePassword,
    responses(
        (status = NO_CONTENT, description = "Changed."),
        (status = BAD_REQUEST, description = "weak_password.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated, or invalid_credentials: the current password is wrong.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn change_password(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<ChangePassword>,
) -> Result<StatusCode, ApiError> {
    state
        .auth
        .change_password(
            &principal,
            &Secret::new(body.current_password),
            &Secret::new(body.new_password),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A browser session of the user.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionInfo {
    pub id: Uuid,
    /// The session this request came with.
    pub current: bool,
    pub user_agent: String,
    pub ip: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub absolute_expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionList {
    /// Newest first.
    pub sessions: Vec<SessionInfo>,
}

#[utoipa::path(
    get,
    path = "/account/sessions",
    tag = "account",
    summary = "List the browser sessions",
    description = "Needs a session or an API token with the read scope.",
    responses(
        (status = OK, description = "The sessions that have not expired.", body = SessionList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_sessions(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<SessionList>, ApiError> {
    let current = principal.session_id();
    let sessions = state.auth.list_sessions(&principal).await?;
    Ok(Json(SessionList {
        sessions: sessions
            .into_iter()
            .map(|session| SessionInfo {
                current: Some(session.id) == current,
                id: session.id,
                user_agent: session.user_agent,
                ip: session.ip,
                created_at: session.created_at,
                last_seen_at: session.last_seen_at,
                expires_at: session.expires_at,
                absolute_expires_at: session.absolute_expires_at,
            })
            .collect(),
    }))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Revoked {
    /// How many sessions ended.
    pub revoked: u64,
}

#[utoipa::path(
    delete,
    path = "/account/sessions",
    tag = "account",
    summary = "Sign out every other session",
    description = "Ends every session of the user but the current one. Only from a browser \
                   session.",
    responses(
        (status = OK, description = "How many ended.", body = Revoked),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn revoke_other_sessions(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<Revoked>, ApiError> {
    let revoked = state.auth.revoke_other_sessions(&principal).await?;
    Ok(Json(Revoked { revoked }))
}

#[utoipa::path(
    delete,
    path = "/account/sessions/{id}",
    tag = "account",
    summary = "End a session",
    description = "Ends one of the user's sessions; ending the current one also clears its \
                   cookie. Only from a browser session.",
    params(("id" = Uuid, Path, description = "The session's id.")),
    responses(
        (status = NO_CONTENT, description = "Ended."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn revoke_session(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    state.auth.revoke_session(&principal, id).await?;
    if principal.session_id() == Some(id) {
        return Ok((
            StatusCode::NO_CONTENT,
            [(SET_COOKIE, state.web.clear_cookie())],
        )
            .into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// An API token, without its secret.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiToken {
    pub id: Uuid,
    #[schema(example = "CI deploys")]
    pub name: String,
    /// The token's first characters, to recognise it.
    #[schema(example = "nw_AbCdEfG")]
    pub prefix: String,
    pub scopes: Vec<Scope>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Absent for a token that never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

impl From<neon_domain::auth::ApiToken> for ApiToken {
    fn from(token: neon_domain::auth::ApiToken) -> Self {
        Self {
            id: token.id,
            name: token.name,
            prefix: token.prefix,
            scopes: token.scopes.into_iter().map(Scope::from).collect(),
            created_at: token.created_at,
            expires_at: token.expires_at,
            last_used_at: token.last_used_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiTokenList {
    /// Oldest first.
    pub tokens: Vec<ApiToken>,
}

#[utoipa::path(
    get,
    path = "/account/tokens",
    tag = "account",
    summary = "List the API tokens",
    description = "Needs a session or an API token with the read scope.",
    responses(
        (status = OK, description = "The tokens.", body = ApiTokenList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_tokens(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<ApiTokenList>, ApiError> {
    let tokens = state.auth.list_api_tokens(&principal).await?;
    Ok(Json(ApiTokenList {
        tokens: tokens.into_iter().map(ApiToken::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateApiToken {
    /// 1 to 64 characters.
    #[schema(example = "CI deploys")]
    pub name: String,
    /// At least one; admin only for an admin.
    pub scopes: Vec<Scope>,
    /// After now; absent for a token that never expires.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

/// A new token with its secret, shown this once.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreatedApiToken {
    /// Send it as `Authorization: Bearer <token>`.
    #[schema(example = "nw_AbCdEfGh...")]
    pub token: String,
    pub api_token: ApiToken,
}

#[utoipa::path(
    post,
    path = "/account/tokens",
    tag = "account",
    summary = "Create an API token",
    description = "Its secret is in this answer only; the server keeps its hash. A token never \
                   does more than its owner's role allows. Only from a browser session.",
    request_body = CreateApiToken,
    responses(
        (status = CREATED, description = "The token.", body = CreatedApiToken),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session, or the admin scope for a user who is no admin.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn create_token(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<CreateApiToken>,
) -> Result<(StatusCode, Json<CreatedApiToken>), ApiError> {
    let scopes: Vec<&str> = body.scopes.iter().map(|scope| scope.as_str()).collect();
    let (secret, token) = state
        .auth
        .create_api_token(&principal, &body.name, body.expires_at, &scopes)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedApiToken {
            token: secret.expose().to_owned(),
            api_token: token.into(),
        }),
    ))
}

#[utoipa::path(
    delete,
    path = "/account/tokens/{id}",
    tag = "account",
    summary = "Revoke an API token",
    description = "It stops working at once. Only from a browser session.",
    params(("id" = Uuid, Path, description = "The token's id.")),
    responses(
        (status = NO_CONTENT, description = "Revoked."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn revoke_token(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.auth.revoke_api_token(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The colour set.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    Nothing,
    Soft,
    Warm,
}

/// Light or dark, or whichever the system prefers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    System,
    Light,
    Dark,
}

/// The interface's typeface.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiFont {
    Default,
    Mono,
}

/// The look of the web app, kept with the account.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub palette: Palette,
    pub mode: Mode,
    /// The user's own accent, a whole number of degrees, 0 to 359. Absent or null, the
    /// accent is the hue of the workspace in view, or a blue outside any.
    #[schema(minimum = 0, maximum = 359, example = 250)]
    pub accent_hue: Option<u16>,
    pub ui_font: UiFont,
}

impl From<DomainPreferences> for Preferences {
    fn from(preferences: DomainPreferences) -> Self {
        Self {
            palette: match preferences.palette {
                DomainPalette::Nothing => Palette::Nothing,
                DomainPalette::Soft => Palette::Soft,
                DomainPalette::Warm => Palette::Warm,
            },
            mode: match preferences.mode {
                DomainMode::System => Mode::System,
                DomainMode::Light => Mode::Light,
                DomainMode::Dark => Mode::Dark,
            },
            accent_hue: preferences.accent_hue(),
            ui_font: match preferences.ui_font {
                DomainUiFont::Default => UiFont::Default,
                DomainUiFont::Mono => UiFont::Mono,
            },
        }
    }
}

impl TryFrom<Preferences> for DomainPreferences {
    type Error = ApiError;

    fn try_from(preferences: Preferences) -> Result<Self, ApiError> {
        let palette = match preferences.palette {
            Palette::Nothing => DomainPalette::Nothing,
            Palette::Soft => DomainPalette::Soft,
            Palette::Warm => DomainPalette::Warm,
        };
        let mode = match preferences.mode {
            Mode::System => DomainMode::System,
            Mode::Light => DomainMode::Light,
            Mode::Dark => DomainMode::Dark,
        };
        let ui_font = match preferences.ui_font {
            UiFont::Default => DomainUiFont::Default,
            UiFont::Mono => DomainUiFont::Mono,
        };
        DomainPreferences::new(palette, mode, preferences.accent_hue, ui_font).map_err(|error| {
            ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", error.to_string())
        })
    }
}

#[utoipa::path(
    get,
    path = "/account/preferences",
    tag = "account",
    summary = "The web app's look for this user",
    description = "Defaults for what the user never set. Needs a session or an API token with \
                   the read scope.",
    responses(
        (status = OK, description = "The preferences.", body = Preferences),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_preferences(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Json<Preferences>, ApiError> {
    Ok(Json(state.auth.preferences(&principal).await?.into()))
}

#[utoipa::path(
    put,
    path = "/account/preferences",
    tag = "account",
    summary = "Save the web app's look for this user",
    description = "Replaces all four. Needs a session or an API token with the write scope.",
    request_body = Preferences,
    responses(
        (status = OK, description = "The preferences.", body = Preferences),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn put_preferences(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<Preferences>,
) -> Result<Json<Preferences>, ApiError> {
    let saved = state
        .auth
        .set_preferences(&principal, body.try_into()?)
        .await?;
    Ok(Json(saved.into()))
}
