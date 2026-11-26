//! The OAuth 2.1 authorization server over HTTP.
//!
//! - **For clients**, from any origin, without cookies: the metadata at
//!   `/.well-known/oauth-authorization-server` (RFC 8414) and
//!   `/.well-known/oauth-protected-resource` (RFC 9728), dynamic client registration
//!   (RFC 7591), the token endpoint and revocation (RFC 7009). These answer in OAuth's own
//!   error shape, `{"error": code, "error_description": text}`, and allow CORS, so that a
//!   client in a browser can use them. The registration, token and revocation endpoints
//!   are spared the origin check: they never read the session cookie.
//! - **For the browser**: the authorization endpoint checks a request and sends the
//!   browser to the web app's consent page, or there with the problem; the page asks what
//!   it offers and sends the person's answer, from the session alone, in the API's error
//!   shape.
//! - **Connected apps**: a person's own, on the account's routes, and those acting as a
//!   workspace's agents, for the people who may see and revoke agents' tokens.
//!
//! The URLs the metadata and every authorization response name come from
//! `server.public_url`; without it, from the scheme and host each request was sent to.

use std::borrow::Cow;
use std::collections::HashSet;

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts, Request, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE, CACHE_CONTROL, CONTENT_TYPE, LOCATION, PRAGMA, RETRY_AFTER,
};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use neon_app::oauth::{
    Approval, ConnectedApp, Consent, ConsentWorkspace, IssuedTokens, OAuthError, TokenRequest,
};
use neon_domain::oauth::{
    API_RESOURCE_METADATA_PATH, Audience, AuthMethod, AuthorizationRequest, CONSENT_PATH,
    ClientMetadata, Endpoints, ErrorCode, GRANT_TYPES, Identity, MCP_RESOURCE_METADATA_PATH,
    REGISTER_PATH, RESOURCE_METADATA_PATH, RESPONSE_TYPES, REVOKE_PATH, SERVER_METADATA_PATH,
    TOKEN_PATH, scope_text,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::agents::Agent;
use super::dto::{Person, Scope, WorkspaceRole};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::principal::{Account, Authenticated};
use super::web::ClientIp;

/// The largest registration read: a name and ten addresses, with room to spare.
const MAX_REGISTRATION: usize = 64 * 1024;

/// The largest form the token and revocation endpoints read: a code, its verifier and the
/// longest redirect URI, each character percent-encoded, with room to spare.
const MAX_FORM: usize = 16 * 1024;

/// The most parameters a form may hold: more than any request here sends, few enough that
/// looking through them costs nothing.
const MAX_FORM_PAIRS: usize = 16;

/// The body limit of the registration endpoint, so that a larger body is never read.
pub(super) fn registration_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_REGISTRATION)
}

/// The body limit of the token and revocation endpoints, so that a larger form is never
/// read.
pub(super) fn form_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_FORM)
}

/// How long clients may keep the metadata.
const METADATA_MAX_AGE: &str = "public, max-age=3600";

/// The paths clients call from browsers of any origin: the metadata, registration, the
/// token endpoint and revocation.
const CORS_PATHS: [&str; 7] = [
    SERVER_METADATA_PATH,
    RESOURCE_METADATA_PATH,
    MCP_RESOURCE_METADATA_PATH,
    API_RESOURCE_METADATA_PATH,
    REGISTER_PATH,
    TOKEN_PATH,
    REVOKE_PATH,
];

/// CORS for [`CORS_PATHS`] alone: any origin, without credentials, as browsers send no
/// cookie with these requests and these answers hold nothing of anyone's. A preflight is
/// answered here, as the routes take no `OPTIONS` and would refuse it; the other answers
/// get `Access-Control-Allow-Origin: *`. Every other path stays closed to scripts of other
/// origins.
pub(super) async fn cors(request: Request, next: Next) -> Response {
    if !CORS_PATHS.contains(&request.uri().path()) {
        return next.run(request).await;
    }
    if request.method() == Method::OPTIONS {
        return (
            StatusCode::NO_CONTENT,
            [
                (ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                (ACCESS_CONTROL_ALLOW_METHODS, "GET, POST"),
                (
                    ACCESS_CONTROL_ALLOW_HEADERS,
                    "authorization, content-type, mcp-protocol-version",
                ),
                (ACCESS_CONTROL_MAX_AGE, "3600"),
            ],
        )
            .into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

/// Where the server's endpoints are, as clients reach them.
pub(super) struct Base(pub(super) Endpoints);

impl FromRequestParts<AppState> for Base {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip());
        Ok(Self(Endpoints::new(
            &state.web.public_origin(peer, &parts.headers),
        )))
    }
}

/// The authorization server's metadata (RFC 8414).
#[derive(Debug, Clone, Serialize)]
pub struct ServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// Absent while clients may not register themselves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: String,
    pub scopes_supported: Vec<&'static str>,
    pub response_types_supported: Vec<&'static str>,
    pub response_modes_supported: Vec<&'static str>,
    pub grant_types_supported: Vec<&'static str>,
    pub code_challenge_methods_supported: Vec<&'static str>,
    pub token_endpoint_auth_methods_supported: Vec<&'static str>,
    pub revocation_endpoint_auth_methods_supported: Vec<&'static str>,
    pub authorization_response_iss_parameter_supported: bool,
    pub service_documentation: String,
}

/// The metadata of a protected resource (RFC 9728).
#[derive(Debug, Clone, Serialize)]
pub struct ResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub scopes_supported: Vec<&'static str>,
    pub bearer_methods_supported: Vec<&'static str>,
    pub resource_name: String,
    pub resource_documentation: String,
}

fn scopes_supported() -> Vec<&'static str> {
    neon_domain::auth::Scope::ALL
        .iter()
        .map(|scope| scope.as_str())
        .collect()
}

fn metadata<T: Serialize>(document: T) -> Response {
    (
        [(CACHE_CONTROL, HeaderValue::from_static(METADATA_MAX_AGE))],
        axum::Json(document),
    )
        .into_response()
}

/// `GET /.well-known/oauth-authorization-server`
pub(super) async fn server_metadata(State(state): State<AppState>, Base(base): Base) -> Response {
    metadata(ServerMetadata {
        issuer: base.issuer().to_owned(),
        authorization_endpoint: base.authorization(),
        token_endpoint: base.token(),
        registration_endpoint: state.oauth.registration_open().then(|| base.registration()),
        revocation_endpoint: base.revocation(),
        scopes_supported: scopes_supported(),
        response_types_supported: RESPONSE_TYPES.to_vec(),
        response_modes_supported: vec!["query"],
        grant_types_supported: GRANT_TYPES.to_vec(),
        code_challenge_methods_supported: vec!["S256"],
        token_endpoint_auth_methods_supported: vec![AuthMethod::None.as_str()],
        revocation_endpoint_auth_methods_supported: vec![AuthMethod::None.as_str()],
        authorization_response_iss_parameter_supported: true,
        service_documentation: format!("{}/docs/guides/oauth/", base.issuer()),
    })
}

fn resource(state: &AppState, base: &Endpoints, audience: Audience) -> Response {
    let name = state
        .settings
        .current()
        .get(neon_app::settings::INSTANCE_NAME)
        .to_owned();
    metadata(ResourceMetadata {
        resource: base.resource(audience),
        authorization_servers: vec![base.issuer().to_owned()],
        scopes_supported: scopes_supported(),
        bearer_methods_supported: vec!["header"],
        resource_name: match audience {
            Audience::Mcp => format!("{name} MCP"),
            Audience::Api => format!("{name} API"),
        },
        resource_documentation: format!("{}/docs/guides/oauth/", base.issuer()),
    })
}

/// `GET /.well-known/oauth-protected-resource`, and the same below it for `/mcp`: the
/// MCP endpoint's, which its 401 names.
pub(super) async fn mcp_metadata(State(state): State<AppState>, Base(base): Base) -> Response {
    resource(&state, &base, Audience::Mcp)
}

/// `GET /.well-known/oauth-protected-resource/api/v1`: the REST API's.
pub(super) async fn api_metadata(State(state): State<AppState>, Base(base): Base) -> Response {
    resource(&state, &base, Audience::Api)
}

/// An error as OAuth's endpoints answer it (RFC 6749, section 5.2).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OAuthErrorBody {
    /// Such as `invalid_grant` or `invalid_redirect_uri`.
    #[schema(example = "invalid_grant")]
    pub error: String,
    /// What went wrong, for whoever reads the client's logs.
    #[schema(example = "the code expired")]
    pub error_description: String,
}

/// An error of the token, registration and revocation endpoints, in OAuth's shape.
#[derive(Debug)]
struct Protocol(OAuthError);

impl IntoResponse for Protocol {
    fn into_response(self) -> Response {
        let error = self.0;
        let (status, code) = match &error {
            OAuthError::Protocol(problem) => (
                if problem.code == ErrorCode::InvalidClient {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::BAD_REQUEST
                },
                problem.code.as_str(),
            ),
            OAuthError::RegistrationOff => (StatusCode::FORBIDDEN, error.code()),
            OAuthError::ClientLimit => (StatusCode::SERVICE_UNAVAILABLE, error.code()),
            OAuthError::RateLimited { .. } | OAuthError::SourceLimit => {
                (StatusCode::TOO_MANY_REQUESTS, error.code())
            }
            OAuthError::Database(_) | OAuthError::Internal(_) => {
                tracing::error!(error = %error, "an OAuth request failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(OAuthErrorBody {
                        error: "server_error".to_owned(),
                        error_description: "internal error".to_owned(),
                    }),
                )
                    .into_response();
            }
            OAuthError::Request(_)
            | OAuthError::Scopes(_)
            | OAuthError::IdentityUnavailable(_)
            | OAuthError::Forbidden(_)
            | OAuthError::NotFound(_) => (StatusCode::BAD_REQUEST, error.code()),
        };
        let description = match &error {
            OAuthError::Protocol(problem) => problem.description.clone(),
            other => other.to_string(),
        };
        let mut response = (
            status,
            [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
            axum::Json(OAuthErrorBody {
                error: code.to_owned(),
                error_description: description,
            }),
        )
            .into_response();
        if let OAuthError::RateLimited { retry_after } = error {
            let seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(seconds.max(1)));
        }
        response
    }
}

fn protocol(code: ErrorCode, description: &str) -> Protocol {
    Protocol(OAuthError::Protocol(neon_domain::oauth::Problem::new(
        code,
        description,
    )))
}

/// A body that could not be read, in OAuth's shape with `code` and the status the
/// rejection has: `413`, saying `too_large`, for one larger than the endpoint's limit.
fn unreadable(rejection: &BytesRejection, code: ErrorCode, too_large: &str) -> Response {
    let status = rejection.status();
    let description = if status == StatusCode::PAYLOAD_TOO_LARGE {
        too_large
    } else {
        "the body could not be read"
    };
    (
        status,
        [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
        axum::Json(OAuthErrorBody {
            error: code.as_str().to_owned(),
            error_description: description.to_owned(),
        }),
    )
        .into_response()
}

/// What a client says of itself to register (RFC 7591). Metadata this server does not use
/// is ignored.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct ClientRegistration {
    /// One to ten: https, http to this machine (127.0.0.1, [::1] or localhost) on any
    /// port, or a scheme of a native app's own.
    pub redirect_uris: Option<Vec<String>>,
    /// Shown on the consent page as the client's own word; at most 100 characters kept.
    #[schema(example = "Example CLI")]
    pub client_name: Option<String>,
    /// `authorization_code`, and `refresh_token`, when given.
    pub grant_types: Option<Vec<String>>,
    /// `code`, when given.
    pub response_types: Option<Vec<String>>,
    /// `none`: every client here is public. Another is replaced by it.
    pub token_endpoint_auth_method: Option<String>,
}

/// A registered client.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisteredClient {
    pub client_id: String,
    /// When it registered, in seconds since 1970.
    pub client_id_issued_at: i64,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    /// Always `none`.
    pub token_endpoint_auth_method: String,
}

#[utoipa::path(
    post,
    path = "/oauth/register",
    tag = "oauth",
    summary = "Register an OAuth client",
    description = "Dynamic client registration (RFC 7591), open to anyone while \
                   `oauth.registration` is on, a few at a time from one address or IPv6 /64, \
                   and a few waiting unconnected from each. Every client \
                   is public, with no secret: it proves itself with PKCE. Allows CORS from any \
                   origin. Errors are in OAuth's shape.",
    request_body = ClientRegistration,
    responses(
        (status = CREATED, description = "The client.", body = RegisteredClient),
        (status = BAD_REQUEST, description = "invalid_redirect_uri or invalid_client_metadata.", body = OAuthErrorBody),
        (status = FORBIDDEN, description = "registration_off.", body = OAuthErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "invalid_client_metadata: a body larger than 64 KiB.", body = OAuthErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited, with Retry-After; or too_many_clients: too many clients registered from the same address, or IPv6 /64, wait unconnected.", body = OAuthErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "temporarily_unavailable: too many clients wait unconnected.", body = OAuthErrorBody),
    )
)]
pub(super) async fn register_client(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            return unreadable(
                &rejection,
                ErrorCode::InvalidClientMetadata,
                "the metadata is larger than a client needs",
            );
        }
    };
    let Ok(registration) = serde_json::from_slice::<ClientRegistration>(&body) else {
        return protocol(
            ErrorCode::InvalidClientMetadata,
            "the body is not a client's metadata as JSON",
        )
        .into_response();
    };
    let metadata = ClientMetadata {
        client_name: registration.client_name,
        redirect_uris: registration.redirect_uris,
        grant_types: registration.grant_types,
        response_types: registration.response_types,
        token_endpoint_auth_method: registration.token_endpoint_auth_method,
    };
    match state.oauth.register(&metadata, &ip).await {
        Ok(client) => (
            StatusCode::CREATED,
            Json(RegisteredClient {
                client_id: client.id.to_string(),
                client_id_issued_at: client.created_at.unix_timestamp(),
                client_name: client.name,
                redirect_uris: client.redirect_uris,
                grant_types: GRANT_TYPES
                    .iter()
                    .map(|grant| (*grant).to_owned())
                    .collect(),
                response_types: RESPONSE_TYPES
                    .iter()
                    .map(|response| (*response).to_owned())
                    .collect(),
                token_endpoint_auth_method: AuthMethod::None.as_str().to_owned(),
            }),
        )
            .into_response(),
        Err(error) => Protocol(error).into_response(),
    }
}

/// The parameters of a form body, each named at most once, or why not. A form of more
/// than [`MAX_FORM_PAIRS`] is refused as soon as that shows.
fn form(headers: &HeaderMap, body: &[u8]) -> Result<Vec<(String, String)>, Protocol> {
    let urlencoded = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| {
            media
                .trim()
                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
        });
    if !urlencoded {
        return Err(protocol(
            ErrorCode::InvalidRequest,
            "the body is a form: application/x-www-form-urlencoded",
        ));
    }
    let mut pairs = Vec::new();
    let mut seen: HashSet<Cow<'_, str>> = HashSet::new();
    for (key, value) in url::form_urlencoded::parse(body) {
        if pairs.len() == MAX_FORM_PAIRS {
            return Err(protocol(
                ErrorCode::InvalidRequest,
                "the form has more parameters than any request here sends",
            ));
        }
        if !seen.insert(key.clone()) {
            return Err(protocol(
                ErrorCode::InvalidRequest,
                "a parameter is sent more than once",
            ));
        }
        pairs.push((key.into_owned(), value.into_owned()));
    }
    Ok(pairs)
}

/// A form the token or revocation endpoint could not read.
fn too_large_form(rejection: &BytesRejection) -> Response {
    unreadable(
        rejection,
        ErrorCode::InvalidRequest,
        "the form is larger than any request here sends",
    )
}

fn take(pairs: &mut Vec<(String, String)>, key: &str) -> Option<String> {
    let index = pairs.iter().position(|(name, _)| name == key)?;
    Some(pairs.swap_remove(index).1)
}

/// A request to the token endpoint, a form.
#[derive(Debug, Clone, Default, ToSchema)]
pub struct TokenForm {
    /// `authorization_code` or `refresh_token`.
    #[schema(required = true)]
    pub grant_type: Option<String>,
    /// The client, which public clients name to identify themselves.
    #[schema(required = true)]
    pub client_id: Option<String>,
    /// For `authorization_code`: the code.
    pub code: Option<String>,
    /// For `authorization_code`: the redirect URI the code was given to.
    pub redirect_uri: Option<String>,
    /// For `authorization_code`: the PKCE verifier.
    pub code_verifier: Option<String>,
    /// For `refresh_token`: the refresh token, which works once.
    pub refresh_token: Option<String>,
    /// For `refresh_token`: fewer scopes than the refresh token had.
    pub scope: Option<String>,
    /// The resource the tokens are for (RFC 8707): the one the authorization asked for.
    pub resource: Option<String>,
}

impl TokenForm {
    fn from_pairs(mut pairs: Vec<(String, String)>) -> Self {
        Self {
            grant_type: take(&mut pairs, "grant_type"),
            client_id: take(&mut pairs, "client_id"),
            code: take(&mut pairs, "code"),
            redirect_uri: take(&mut pairs, "redirect_uri"),
            code_verifier: take(&mut pairs, "code_verifier"),
            refresh_token: take(&mut pairs, "refresh_token"),
            scope: take(&mut pairs, "scope"),
            resource: take(&mut pairs, "resource"),
        }
    }
}

impl From<TokenForm> for TokenRequest {
    fn from(form: TokenForm) -> Self {
        Self {
            grant_type: form.grant_type,
            client_id: form.client_id,
            code: form.code,
            redirect_uri: form.redirect_uri,
            code_verifier: form.code_verifier,
            refresh_token: form.refresh_token,
            scope: form.scope,
            resource: form.resource,
        }
    }
}

/// What the token endpoint gives.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TokenResponse {
    /// `nwo_` and 43 characters: send it as `Authorization: Bearer`.
    pub access_token: String,
    /// `Bearer`.
    pub token_type: String,
    /// Seconds until the access token stops working.
    pub expires_in: u64,
    /// `nwr_` and 43 characters: works once, for a new access token and refresh token.
    pub refresh_token: String,
    /// The scopes granted, space-separated.
    #[schema(example = "read write")]
    pub scope: String,
}

impl From<IssuedTokens> for TokenResponse {
    fn from(tokens: IssuedTokens) -> Self {
        Self {
            access_token: tokens.access_token.expose().to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: tokens.expires_in.as_secs(),
            refresh_token: tokens.refresh_token.expose().to_owned(),
            scope: scope_text(&tokens.scopes),
        }
    }
}

#[utoipa::path(
    post,
    path = "/oauth/token",
    tag = "oauth",
    summary = "Exchange a code or a refresh token",
    description = "The token endpoint. `authorization_code` exchanges a code, within its \
                   minute and once, with its redirect URI and PKCE verifier; \
                   `refresh_token` exchanges a refresh token, once. Each gives an access token, \
                   an hour by default, and a new refresh token, 30 days by default. A code or \
                   refresh token used a second time revokes its grant and every token of it. \
                   Allows CORS from any origin. Errors are in OAuth's shape.",
    request_body(content = TokenForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = OK, description = "The tokens.", body = TokenResponse),
        (status = BAD_REQUEST, description = "invalid_request, invalid_grant, invalid_scope, invalid_target or unsupported_grant_type.", body = OAuthErrorBody),
        (status = UNAUTHORIZED, description = "invalid_client.", body = OAuthErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "invalid_request: a form larger than 16 KiB.", body = OAuthErrorBody),
    )
)]
pub(super) async fn issue_tokens(
    State(state): State<AppState>,
    Base(base): Base,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) => return too_large_form(&rejection),
    };
    let request = match form(&headers, &body) {
        Ok(pairs) => TokenRequest::from(TokenForm::from_pairs(pairs)),
        Err(refusal) => return refusal.into_response(),
    };
    match state.oauth.token(&request, &base).await {
        Ok(tokens) => (
            [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
            axum::Json(TokenResponse::from(tokens)),
        )
            .into_response(),
        Err(error) => Protocol(error).into_response(),
    }
}

/// A request to revoke a token, a form (RFC 7009). A `token_type_hint` may come along;
/// the token is looked up whatever it says.
#[derive(Debug, Clone, Default, ToSchema)]
pub struct RevokeForm {
    /// An access token or a refresh token.
    #[schema(required = true)]
    pub token: Option<String>,
    /// The client the token was issued to.
    #[schema(required = true)]
    pub client_id: Option<String>,
}

impl RevokeForm {
    fn from_pairs(mut pairs: Vec<(String, String)>) -> Self {
        Self {
            token: take(&mut pairs, "token"),
            client_id: take(&mut pairs, "client_id"),
        }
    }
}

#[utoipa::path(
    post,
    path = "/oauth/revoke",
    tag = "oauth",
    summary = "Revoke an OAuth token",
    description = "Token revocation (RFC 7009): revokes the grant of an access or refresh \
                   token, every token of it, as a client does when it signs out. A token that \
                   is unknown, or another client's, changes nothing, and the answer is the \
                   same. Allows CORS from any origin. Errors are in OAuth's shape.",
    request_body(content = RevokeForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = OK, description = "Revoked, or nothing to revoke."),
        (status = BAD_REQUEST, description = "invalid_request.", body = OAuthErrorBody),
        (status = UNAUTHORIZED, description = "invalid_client.", body = OAuthErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "invalid_request: a form larger than 16 KiB.", body = OAuthErrorBody),
    )
)]
pub(super) async fn revoke_oauth_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) => return too_large_form(&rejection),
    };
    let request = match form(&headers, &body) {
        Ok(pairs) => RevokeForm::from_pairs(pairs),
        Err(refusal) => return refusal.into_response(),
    };
    match state
        .oauth
        .revoke(request.token.as_deref(), request.client_id.as_deref())
        .await
    {
        Ok(()) => (StatusCode::OK, [(CACHE_CONTROL, "no-store")]).into_response(),
        Err(error) => Protocol(error).into_response(),
    }
}

/// The parameters of an authorization request, as the client sends the browser with them.
/// Each at most once; others are ignored.
#[derive(Debug, Clone, Default, Deserialize, Serialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct AuthorizationParams {
    /// `code`.
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    /// One the client registered, exactly, but for the port of a loopback address.
    pub redirect_uri: Option<String>,
    /// The PKCE challenge: the verifier's SHA-256, in base64url without padding.
    pub code_challenge: Option<String>,
    /// `S256`; `plain` is refused.
    pub code_challenge_method: Option<String>,
    /// Scopes asked for, space-separated: `read`, `write`, `admin`. The person chooses.
    pub scope: Option<String>,
    /// Sent back as it is; at most 1024 printable ASCII characters.
    pub state: Option<String>,
    /// The resource the tokens are for (RFC 8707): `<public URL>/mcp`, the default, or
    /// `<public URL>/api/v1`.
    pub resource: Option<String>,
}

impl From<AuthorizationParams> for AuthorizationRequest {
    fn from(params: AuthorizationParams) -> Self {
        Self {
            response_type: params.response_type,
            client_id: params.client_id,
            redirect_uri: params.redirect_uri,
            code_challenge: params.code_challenge,
            code_challenge_method: params.code_challenge_method,
            scope: params.scope,
            state: params.state,
            resource: params.resource,
        }
    }
}

/// A redirect to `location`.
fn found(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(value) => (
            StatusCode::FOUND,
            [
                (LOCATION, value),
                (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
        )
            .into_response(),
        Err(_) => ApiError::internal().into_response(),
    }
}

/// The consent page's address with `pairs` as its query.
fn consent_page<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    format!("{CONSENT_PATH}?{query}")
}

#[utoipa::path(
    get,
    path = "/oauth/authorize",
    tag = "oauth",
    summary = "Start an authorization",
    description = "The authorization endpoint, the code flow with PKCE (S256 alone). Checks \
                   the client, the redirect URI, the response type, the PKCE challenge, the \
                   scopes, the state and the resource, then sends the browser to the web app's \
                   consent page with them, where the person signs in if they have not, and \
                   chooses. A request that does not hold sends the browser to the consent page \
                   with `error`, its problem, which the page shows; it is never sent to the \
                   client's redirect URI on its own, as anyone may register one.",
    params(AuthorizationParams),
    responses(
        (status = FOUND, description = "To the consent page, with the request or `error`."),
    )
)]
pub(super) async fn authorize(
    State(state): State<AppState>,
    Base(base): Base,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let request = match AuthorizationRequest::from_pairs(pairs) {
        Ok(request) => request,
        Err(problem) => return found(&consent_page([("error", problem.as_str())])),
    };
    match state.oauth.check_authorization(&request, &base).await {
        Ok(_) => found(&consent_page(request.pairs())),
        Err(OAuthError::Request(problem)) => found(&consent_page([("error", problem.as_str())])),
        Err(error) => ApiError::from(error).into_response(),
    }
}

/// A registered client, as the consent page names it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConsentClient {
    pub id: Uuid,
    /// What it calls itself: its own word, not checked.
    #[schema(example = "Example CLI")]
    pub name: String,
}

/// What the resource a client asked for is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OAuthResource {
    /// The MCP endpoint, `/mcp`.
    Mcp,
    /// The REST API, `/api/v1`.
    Api,
}

impl From<Audience> for OAuthResource {
    fn from(audience: Audience) -> Self {
        match audience {
            Audience::Mcp => Self::Mcp,
            Audience::Api => Self::Api,
        }
    }
}

/// A workspace the consent page offers, with whom the client may act as there.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConsentOption {
    pub id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    #[schema(example = "design")]
    pub slug: String,
    pub icon: Option<String>,
    #[schema(minimum = 0, maximum = 359, example = 250)]
    pub hue: u16,
    /// The person's role there.
    pub role: WorkspaceRole,
    /// The scopes the client may have acting as the person; absent while `mcp.identities`
    /// leaves acting as the person out.
    pub person_scopes: Option<Vec<Scope>>,
    /// Whether it may act as an agent here: `mcp.identities` and the workspace's agent
    /// switch allow it, and the person is no guest.
    pub agents_on: bool,
    /// The person's agents here that work, which it may act as.
    pub agents: Vec<Agent>,
    /// The scopes it may have acting as one of them: read and write.
    pub agent_scopes: Vec<Scope>,
}

impl From<ConsentWorkspace> for ConsentOption {
    fn from(offered: ConsentWorkspace) -> Self {
        let scopes = |scopes: Vec<neon_domain::auth::Scope>| -> Vec<Scope> {
            scopes.into_iter().map(Scope::from).collect()
        };
        Self {
            hue: offered.workspace.hue(),
            id: offered.workspace.id,
            name: offered.workspace.name,
            slug: offered.workspace.slug,
            icon: offered.workspace.icon,
            role: offered.role.into(),
            person_scopes: offered.as_person.map(scopes),
            agents_on: offered.agents_on,
            agents: offered.agents.into_iter().map(Agent::from).collect(),
            agent_scopes: scopes(offered.agent_scopes),
        }
    }
}

/// What the consent page shows.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConsentInfo {
    pub client: ConsentClient,
    /// Where the browser goes back to.
    pub redirect_uri: String,
    /// Its scheme, host and port, a host in other alphabets written in ASCII.
    #[schema(example = "http://127.0.0.1:51004")]
    pub redirect_host: String,
    /// The scopes the client asked for; empty when it named none.
    pub requested_scopes: Vec<Scope>,
    pub resource: OAuthResource,
    /// The person's workspaces, by name.
    pub workspaces: Vec<ConsentOption>,
}

impl From<Consent> for ConsentInfo {
    fn from(consent: Consent) -> Self {
        Self {
            client: ConsentClient {
                id: consent.client.id,
                name: consent.client.name,
            },
            redirect_uri: consent.request.redirect_uri,
            redirect_host: consent.redirect_host,
            requested_scopes: consent
                .request
                .scopes
                .into_iter()
                .map(Scope::from)
                .collect(),
            resource: consent.request.audience.into(),
            workspaces: consent
                .workspaces
                .into_iter()
                .map(ConsentOption::from)
                .collect(),
        }
    }
}

#[utoipa::path(
    post,
    path = "/oauth/consent",
    tag = "oauth",
    summary = "What a client asks to be allowed",
    description = "Checks an authorization request again, from the consent page's address, \
                   and says what the page offers: the client, where the browser goes back to, \
                   and each of the person's workspaces with whom the client may act as there, \
                   the person or one of their agents that works, and with which scopes. Only \
                   reads. From a browser session alone.",
    request_body = AuthorizationParams,
    responses(
        (status = OK, description = "What the page offers.", body = ConsentInfo),
        (status = BAD_REQUEST, description = "The request's problem: duplicate_parameter, missing_client_id, unknown_client, missing_redirect_uri, redirect_uri_mismatch, unsupported_response_type, pkce_required, invalid_code_challenge, invalid_scope, invalid_state or invalid_target.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn read_consent(
    State(state): State<AppState>,
    Account(principal): Account,
    Base(base): Base,
    Json(params): Json<AuthorizationParams>,
) -> Result<Json<ConsentInfo>, ApiError> {
    let consent = state
        .oauth
        .consent(&principal, &params.into(), &base)
        .await?;
    Ok(Json(consent.into()))
}

/// The person's answer: the request, and what they chose.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveConsent {
    pub request: AuthorizationParams,
    /// The workspace the client acts in.
    pub workspace_id: Uuid,
    /// One of the person's agents there that the client acts as; absent, it acts as the
    /// person.
    #[serde(default)]
    pub agent_id: Option<Uuid>,
    /// Some of the scopes offered for that identity.
    pub scopes: Vec<String>,
}

/// The person's refusal: the request.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DenyConsent {
    pub request: AuthorizationParams,
}

/// Where to send the browser next.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConsentDecision {
    /// The client's redirect URI, with `code` or `error`, the state, and `iss`.
    pub redirect_to: String,
}

#[utoipa::path(
    post,
    path = "/oauth/consent/approve",
    tag = "oauth",
    summary = "Allow a client",
    description = "Grants the client what the person chose, from a browser session alone: the \
                   workspace, whom it acts as, the person or one of their agents that works \
                   there, and scopes among those offered. Consenting again to the same client, \
                   workspace and identity replaces the grant's scopes. Answers where to send the \
                   browser: the client's redirect URI with a code that works once, within a \
                   minute, with the request's PKCE verifier.",
    request_body = ApproveConsent,
    responses(
        (status = OK, description = "Where the browser goes.", body = ConsentDecision),
        (status = BAD_REQUEST, description = "The request's problem, as for /oauth/consent, or invalid_scope.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden, or identity_unavailable: acting as that identity is off, or the agent does not work.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace or agent.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn approve_consent(
    State(state): State<AppState>,
    Account(principal): Account,
    Base(base): Base,
    Json(body): Json<ApproveConsent>,
) -> Result<Json<ConsentDecision>, ApiError> {
    let approval = Approval {
        workspace_id: body.workspace_id,
        identity: body.agent_id.map_or(Identity::Person, Identity::Agent),
        scopes: body.scopes,
    };
    let redirect_to = state
        .oauth
        .approve(&principal, &body.request.into(), &approval, &base)
        .await?;
    Ok(Json(ConsentDecision { redirect_to }))
}

#[utoipa::path(
    post,
    path = "/oauth/consent/deny",
    tag = "oauth",
    summary = "Deny a client",
    description = "Answers where to send the browser when the person denies the client, from \
                   a browser session alone: its redirect URI with `error=access_denied`.",
    request_body = DenyConsent,
    responses(
        (status = OK, description = "Where the browser goes.", body = ConsentDecision),
        (status = BAD_REQUEST, description = "The request's problem, as for /oauth/consent.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn deny_consent(
    State(state): State<AppState>,
    Account(principal): Account,
    Base(base): Base,
    Json(body): Json<DenyConsent>,
) -> Result<Json<ConsentDecision>, ApiError> {
    let redirect_to = state
        .oauth
        .deny(&principal, &body.request.into(), &base)
        .await?;
    Ok(Json(ConsentDecision { redirect_to }))
}

/// A workspace a connected app acts in.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppWorkspace {
    pub id: Uuid,
    #[schema(example = "Design")]
    pub name: String,
    #[schema(example = "design")]
    pub slug: String,
}

/// Who connected an app.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConnectedBy {
    pub id: Uuid,
    #[schema(example = "Ada Lovelace")]
    pub display_name: String,
}

/// A client a person connected: one workspace, one identity, some scopes.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConnectedAppInfo {
    /// The grant's id, to revoke it by.
    pub id: Uuid,
    pub client: ConsentClient,
    pub workspace: AppWorkspace,
    /// Whom it acts as: the person, or one of their agents.
    pub identity: Person,
    pub connected_by: ConnectedBy,
    pub scopes: Vec<Scope>,
    pub resource: OAuthResource,
    /// When it was first connected.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When it was last consented to.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// When its tokens were last issued or used.
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

impl From<ConnectedApp> for ConnectedAppInfo {
    fn from(app: ConnectedApp) -> Self {
        Self {
            id: app.grant.id,
            client: ConsentClient {
                id: app.grant.client_id,
                name: app.client_name,
            },
            workspace: AppWorkspace {
                id: app.grant.workspace_id,
                name: app.workspace_name,
                slug: app.workspace_slug,
            },
            identity: app.identity.into(),
            connected_by: ConnectedBy {
                id: app.grant.person_id,
                display_name: app.person_name,
            },
            scopes: app.grant.scopes.into_iter().map(Scope::from).collect(),
            resource: app.grant.audience.into(),
            created_at: app.grant.created_at,
            updated_at: app.grant.updated_at,
            last_used_at: app.grant.last_used_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConnectedAppList {
    /// Last used first.
    pub apps: Vec<ConnectedAppInfo>,
}

fn list(apps: Vec<ConnectedApp>) -> Json<ConnectedAppList> {
    Json(ConnectedAppList {
        apps: apps.into_iter().map(ConnectedAppInfo::from).collect(),
    })
}

#[utoipa::path(
    get,
    path = "/account/connected-apps",
    tag = "oauth",
    summary = "List the connected apps",
    description = "The clients the person connected over OAuth, as themselves or as their \
                   agents, each in one workspace with its scopes. Needs a session or an API \
                   token across the instance with the read scope.",
    responses(
        (status = OK, description = "The connected apps.", body = ConnectedAppList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope, or token_bound.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_connected_apps(
    State(state): State<AppState>,
    Account(principal): Account,
) -> Result<Json<ConnectedAppList>, ApiError> {
    Ok(list(state.oauth.connected_apps(&principal).await?))
}

#[utoipa::path(
    delete,
    path = "/account/connected-apps/{id}",
    tag = "oauth",
    summary = "Disconnect an app",
    description = "Revokes one of the person's grants: every token of it stops working at once. \
                   From a browser session alone.",
    params(("id" = Uuid, Path, description = "The connected app's id.")),
    responses(
        (status = NO_CONTENT, description = "Disconnected."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not a browser session.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []))
)]
pub(super) async fn disconnect_app(
    State(state): State<AppState>,
    Account(principal): Account,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.oauth.disconnect_app(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/connected-apps",
    tag = "oauth",
    summary = "List the apps acting as agents",
    description = "The clients connected over OAuth that act as the workspace's agents: every \
                   agent's for the workspace's owners and admins, their own agents' for other \
                   members, none for guests.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    responses(
        (status = OK, description = "The connected apps.", body = ConnectedAppList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_workspace_apps(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<ConnectedAppList>, ApiError> {
    Ok(list(state.oauth.workspace_apps(&principal, id).await?))
}

#[utoipa::path(
    delete,
    path = "/workspaces/{id}/connected-apps/{app_id}",
    tag = "oauth",
    summary = "Disconnect an app acting as an agent",
    description = "Revokes a grant that acts as one of the workspace's agents: every token of \
                   it stops working at once. For the agent's owner and the workspace's owners \
                   and admins, from a browser session.",
    params(
        ("id" = Uuid, Path, description = "The workspace's id."),
        ("app_id" = Uuid, Path, description = "The connected app's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Disconnected."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn disconnect_workspace_app(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, app_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .oauth
        .disconnect_workspace_app(&principal, id, app_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_names_each_parameter_once() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded; charset=utf-8"),
        );

        let mut pairs =
            form(&headers, b"grant_type=refresh_token&scope=read+write").expect("a form");
        assert_eq!(take(&mut pairs, "scope").as_deref(), Some("read write"));
        assert_eq!(take(&mut pairs, "code"), None);
        assert!(form(&headers, b"code=a&code=b").is_err());
        assert!(form(&headers, b"code=a&c%6Fde=b").is_err());
        let most: Vec<String> = (0..MAX_FORM_PAIRS).map(|n| format!("k{n}=")).collect();
        assert_eq!(
            form(&headers, most.join("&").as_bytes())
                .expect("a form")
                .len(),
            MAX_FORM_PAIRS
        );
        let more = format!("{}&one=more", most.join("&"));
        assert!(form(&headers, more.as_bytes()).is_err());
        assert!(form(&HeaderMap::new(), b"code=a").is_err());
        let mut json = HeaderMap::new();
        json.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        assert!(form(&json, b"code=a").is_err());
    }

    #[test]
    fn the_consent_page_gets_the_request_in_its_query() {
        assert_eq!(
            consent_page([("client_id", "a b"), ("state", "x&y")]),
            "/oauth/consent?client_id=a+b&state=x%26y"
        );
    }
}
