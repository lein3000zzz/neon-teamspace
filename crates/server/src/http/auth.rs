//! `/api/v1/auth`: what anyone may call to sign up, sign in and out, and use mailed links.

use axum::extract::State;
use axum::http::header::{SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use neon_app::auth::{LoginInput, RegisterInput};
use neon_domain::auth::RegistrationMode;
use neon_domain::settings::Secret;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::AppState;
use super::dto::{CodeRequest, EmailRequest, TokenRequest, User};
use super::error::{ApiError, ErrorBody};
use super::extract::Json;
use super::principal::Authenticated;
use super::web::ClientIp;

/// Who may create an account.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    Open,
    Invite,
    Closed,
}

impl From<RegistrationMode> for Registration {
    fn from(mode: RegistrationMode) -> Self {
        match mode {
            RegistrationMode::Open => Self::Open,
            RegistrationMode::Invite => Self::Invite,
            RegistrationMode::Closed => Self::Closed,
        }
    }
}

/// Which sign-in and sign-up forms are on, and what a password needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuthConfig {
    pub registration: Registration,
    #[schema(example = 12)]
    pub min_password_length: usize,
    /// Self sign-ups verify their address by a mailed link before they sign in.
    pub verify_email: bool,
    /// Forgotten passwords can be reset by mail.
    pub password_reset: bool,
    /// People without an invite can ask for one.
    pub requests: bool,
}

#[utoipa::path(
    get,
    path = "/auth/config",
    tag = "auth",
    summary = "Sign-up and sign-in settings",
    description = "Which forms to show: the registration mode, and whether verification, \
                   password reset and requests to join are on, which also needs mail set up.",
    responses(
        (status = OK, description = "The settings.", body = AuthConfig),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn get_config(State(state): State<AppState>) -> Json<AuthConfig> {
    let config = state.auth.config();
    Json(AuthConfig {
        registration: config.registration.into(),
        min_password_length: config.min_password_length,
        verify_email: config.verify_email,
        password_reset: config.password_reset,
        requests: config.requests,
    })
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    #[schema(example = "ada@example.com")]
    pub email: String,
    #[schema(format = Password)]
    pub password: String,
    /// The address's local part when absent.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Needed while registration is by invite.
    #[serde(default)]
    pub invite_code: Option<String>,
}

/// A new account.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Registered {
    pub user: User,
    /// The account signs in only once it opened the link mailed to it.
    pub must_verify: bool,
    /// The link was mailed. When it was not, it can be asked for again.
    pub sent: bool,
}

#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    summary = "Create an account",
    description = "Signs up as the registration mode allows: anyone while it is open, with an \
                   invite while it is by invite, nobody while it is closed. Signs nobody in.",
    request_body = RegisterRequest,
    responses(
        (status = CREATED, description = "The account.", body = Registered),
        (status = BAD_REQUEST, description = "invalid_input, invalid_email, weak_password or invalid_invite.", body = ErrorBody),
        (status = FORBIDDEN, description = "registration_closed.", body = ErrorBody),
        (status = CONFLICT, description = "email_taken.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<Registered>), ApiError> {
    let registration = state
        .auth
        .register(RegisterInput {
            email: body.email,
            password: Secret::new(body.password),
            display_name: body.display_name,
            invite_code: body.invite_code.map(Secret::new),
            ip,
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(Registered {
            user: registration.user.into(),
            must_verify: registration.must_verify,
            sent: registration.sent,
        }),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    #[schema(example = "ada@example.com")]
    pub email: String,
    #[schema(format = Password)]
    pub password: String,
}

/// A new session. Its token is only in the cookie.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SignedIn {
    pub user: User,
    /// When the session ends unless it is used before.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// When it ends however much it is used.
    #[serde(with = "time::serde::rfc3339")]
    pub absolute_expires_at: OffsetDateTime,
}

#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    summary = "Sign in",
    description = "Opens a browser session and sets its cookie: `neon_session`, or \
                   `__Host-neon_session` when the server's public URL is https; HttpOnly, \
                   SameSite=Lax, Path=/, Secure over https. An unknown address and a wrong \
                   password answer the same, and take as long; a disabled or unverified \
                   account is only told so after the right password.",
    request_body = LoginRequest,
    responses(
        (status = OK, description = "Signed in.", body = SignedIn,
            headers(("Set-Cookie" = String, description = "The session cookie."))),
        (status = UNAUTHORIZED, description = "invalid_credentials.", body = ErrorBody),
        (status = FORBIDDEN, description = "user_disabled or email_not_verified.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let outcome = state
        .auth
        .login(LoginInput {
            email: body.email,
            password: Secret::new(body.password),
            ip,
            user_agent,
        })
        .await?;
    let session = &outcome.session;
    // The cookie lasts as long as the session can, from when it started just now.
    let lasts = (session.absolute_expires_at - session.created_at)
        .try_into()
        .unwrap_or_default();
    let cookie = state.web.session_cookie(outcome.token.expose(), lasts);
    let body = SignedIn {
        expires_at: session.expires_at,
        absolute_expires_at: session.absolute_expires_at,
        user: outcome.user.into(),
    };
    Ok(([(SET_COOKIE, cookie)], Json(body)).into_response())
}

#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    summary = "Sign out",
    description = "Ends the current browser session and clears its cookie. With an API token, \
                   it ends nothing: revoke the token instead.",
    responses(
        (status = NO_CONTENT, description = "Signed out.",
            headers(("Set-Cookie" = String, description = "Clears the session cookie."))),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn logout(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
) -> Result<Response, ApiError> {
    if let Some(id) = principal.session_id() {
        state.auth.logout(id).await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [(SET_COOKIE, state.web.clear_cookie())],
    )
        .into_response())
}

/// An address a link verified, for the sign-in form.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Verified {
    #[schema(example = "ada@example.com")]
    pub email: String,
}

#[utoipa::path(
    post,
    path = "/auth/verify-email",
    tag = "auth",
    summary = "Verify an address",
    description = "Takes the token of the link mailed after signing up. The link works once. \
                   It signs nobody in: the password is still needed.",
    request_body = TokenRequest,
    responses(
        (status = OK, description = "Verified.", body = Verified),
        (status = BAD_REQUEST, description = "invalid_link: used already, or too old.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn verify_email(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<TokenRequest>,
) -> Result<Json<Verified>, ApiError> {
    let email = state
        .auth
        .verify_email(&Secret::new(body.token), &ip)
        .await?;
    Ok(Json(Verified { email }))
}

#[utoipa::path(
    post,
    path = "/auth/verification",
    tag = "auth",
    summary = "Mail the verification link again",
    description = "Mails a new link to the account of the address if it must still verify. \
                   Answers the same whether it has one or not.",
    request_body = EmailRequest,
    responses(
        (status = NO_CONTENT, description = "Taken."),
        (status = CONFLICT, description = "mail_not_set_up.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn resend_verification(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<EmailRequest>,
) -> Result<StatusCode, ApiError> {
    state.auth.resend_verification(&body.email, &ip).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/auth/password-reset",
    tag = "auth",
    summary = "Ask for a password reset link",
    description = "Mails a link setting a new password to the account of the address, if it has \
                   one. Answers the same whether it has one or not.",
    request_body = EmailRequest,
    responses(
        (status = NO_CONTENT, description = "Taken."),
        (status = FORBIDDEN, description = "password_reset_off.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn request_password_reset(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<EmailRequest>,
) -> Result<StatusCode, ApiError> {
    state.auth.request_password_reset(&body.email, &ip).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResetPasswordRequest {
    /// The token in the link.
    pub token: String,
    #[schema(format = Password)]
    pub password: String,
}

#[utoipa::path(
    post,
    path = "/auth/password-reset/confirm",
    tag = "auth",
    summary = "Set a new password with a mailed link",
    description = "The link works once. It verifies the address and signs the account out of \
                   every session, but signs nobody in.",
    request_body = ResetPasswordRequest,
    responses(
        (status = OK, description = "The password is set.", body = Verified),
        (status = BAD_REQUEST, description = "invalid_link or weak_password.", body = ErrorBody),
        (status = FORBIDDEN, description = "password_reset_off.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn reset_password(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<Json<Verified>, ApiError> {
    let email = state
        .auth
        .reset_password(&Secret::new(body.token), &Secret::new(body.password), &ip)
        .await?;
    Ok(Json(Verified { email }))
}

/// What the sign-up form needs to know of an invite.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteLookup {
    /// The address it is bound to, which the form fills in; absent for any address.
    pub email: Option<String>,
}

#[utoipa::path(
    post,
    path = "/auth/invite",
    tag = "auth",
    summary = "Check an invite code",
    description = "Says whether an invite admits sign-ups now, and the address it is bound to.",
    request_body = CodeRequest,
    responses(
        (status = OK, description = "It admits sign-ups.", body = InviteLookup),
        (status = BAD_REQUEST, description = "invalid_invite: used up, expired, revoked or none.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn check_invite(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<CodeRequest>,
) -> Result<Json<InviteLookup>, ApiError> {
    let invite = state.auth.invite_for(&Secret::new(body.code), &ip).await?;
    Ok(Json(InviteLookup {
        email: invite.email,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinRequestRequest {
    #[schema(example = "ada@example.com")]
    pub email: String,
    /// For the admins, plain text, at most 500 characters.
    #[serde(default)]
    pub note: String,
}

#[utoipa::path(
    post,
    path = "/auth/join-requests",
    tag = "auth",
    summary = "Ask to join",
    description = "Mails the address a link to confirm the request; admins see it once it is \
                   confirmed. Answers the same whatever comes of it, so it tells nobody which \
                   addresses have accounts or requests.",
    request_body = JoinRequestRequest,
    responses(
        (status = NO_CONTENT, description = "Taken."),
        (status = BAD_REQUEST, description = "invalid_email or invalid_input.", body = ErrorBody),
        (status = FORBIDDEN, description = "requests_off.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn request_to_join(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<JoinRequestRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .auth
        .request_to_join(&body.email, &body.note, &ip)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/auth/join-requests/confirm",
    tag = "auth",
    summary = "Confirm a request to join",
    description = "Takes the token of the mailed link. The admins see the request from now.",
    request_body = TokenRequest,
    responses(
        (status = NO_CONTENT, description = "Confirmed."),
        (status = BAD_REQUEST, description = "invalid_link: used already, or too old.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    )
)]
pub(super) async fn confirm_join(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(body): Json<TokenRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .auth
        .confirm_join(&Secret::new(body.token), &ip)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
