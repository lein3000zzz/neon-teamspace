//! Who a request acts as.
//!
//! A request proves it with `Authorization: Bearer nw_…`, an API token, or with the session
//! cookie. A bearer header that does not authenticate is refused outright, without falling
//! back to the cookie. A token's scopes decide what it may do: reading needs `read`, every
//! other method `write`, and the admin routes `admin`. Unauthenticated requests get 401,
//! and those their role or scopes do not allow 403, both in the error shape.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, SET_COOKIE};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use neon_app::auth::AuthError;
use neon_domain::auth::{Principal, Scope};

use super::AppState;
use super::error::ApiError;

/// Any authenticated user, with the scope the request's method needs.
pub struct Authenticated(pub Principal);

/// An admin, through a session or a token with the admin scope.
pub struct Admin(pub Principal);

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let principal = authenticate(parts, state)
            .await
            .map_err(|refusal| refusal.into_response(state))?;
        let needed = if parts.method.is_safe() {
            Scope::Read
        } else {
            Scope::Write
        };
        if !principal.can(needed) {
            return Err(missing_scope(needed));
        }
        Ok(Self(principal))
    }
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        let principal = authenticate(parts, state)
            .await
            .map_err(|refusal| refusal.into_response(state))?;
        if principal.is_admin() {
            Ok(Self(principal))
        } else if principal.user.is_admin() {
            Err(missing_scope(Scope::Admin))
        } else {
            Err(ApiError::forbidden("only an admin may do this").into_response())
        }
    }
}

/// Why a request is not authenticated.
struct Refusal {
    error: ApiError,
    /// The session cookie it sent no longer works, so the answer clears it.
    clear_cookie: bool,
}

impl Refusal {
    fn new(error: ApiError) -> Self {
        Self {
            error,
            clear_cookie: false,
        }
    }

    fn into_response(self, state: &AppState) -> Response {
        let mut response = self.error.into_response();
        if self.clear_cookie {
            response
                .headers_mut()
                .insert(SET_COOKIE, state.web.clear_cookie());
        }
        response
    }
}

/// The principal of the bearer token, or else of the session cookie.
async fn authenticate(parts: &Parts, state: &AppState) -> Result<Principal, Refusal> {
    if let Some(header) = parts.headers.get(AUTHORIZATION) {
        let token = header
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or_else(|| {
                Refusal::new(ApiError::unauthenticated(
                    "the Authorization header wants Bearer and an API token",
                ))
            })?;
        return state.auth.authenticate_token(token).await.map_err(|error| {
            Refusal::new(match error {
                AuthError::Unauthenticated => ApiError::unauthenticated(
                    "the API token does not work: it is unknown, revoked or expired",
                ),
                other => other.into(),
            })
        });
    }
    let Some(token) = state.web.session_token(&parts.headers) else {
        return Err(Refusal::new(ApiError::unauthenticated("sign in first")));
    };
    state
        .auth
        .authenticate_session(token)
        .await
        .map_err(|error| match error {
            AuthError::Unauthenticated => Refusal {
                error: ApiError::unauthenticated("the session has ended: sign in again"),
                clear_cookie: true,
            },
            other => Refusal::new(other.into()),
        })
}

fn missing_scope(scope: Scope) -> Response {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "insufficient_scope",
        format!("this API token lacks the {scope} scope"),
    )
    .into_response()
}
