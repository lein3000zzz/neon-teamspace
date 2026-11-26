//! The MCP server at `/mcp`: agents and the people they act for reach the workspace's pages
//! and databases over the Model Context Protocol, through the SDK's Streamable HTTP
//! transport.
//!
//! - **Transport.** Streamable HTTP with sessions, as protocol revisions 2025-03-26 to
//!   2025-11-25 have it: a client's `initialize` opens a session named by `Mcp-Session-Id`,
//!   each request is a `POST` answered with JSON or server-sent events, a `GET` opens the
//!   stream of notifications, and a `DELETE` ends the session. A request whose `Origin` is
//!   not the server's own is refused with 403, against DNS rebinding: see [`own_origin`].
//! - **Authentication.** Every request, not only the first, carries `Authorization: Bearer`
//!   and is authenticated as the API routes authenticate it, an API token or an OAuth access
//!   token alike; a session cookie is not taken. One without a working token gets 401 and a
//!   `WWW-Authenticate` header naming the protected resource's metadata, which the OAuth
//!   server publishes, so that a client finds where to get a token. The token must be bound
//!   to one workspace, as an agent's and every OAuth token always is, and the server acts in
//!   that workspace alone. A refused request on a session has the session looked at again,
//!   which ends it when its own token no longer works.
//! - **Sessions** are this process's, each bound to the identity that opened it: a request
//!   on a session from another identity, or for another workspace, finds no session. They
//!   are bounded overall, per identity and per person with their agents, end once idle, and
//!   end at once when what they act as stops working: a token revoked, an agent suspended,
//!   access to the workspace lost. See [`sessions`].
//! - **Rate limits** count each identity's requests, and each person's with their agents'.
//! - **Tools and resources** use the app's services, as the API's routes do, so they see
//!   what the caller may see and no more, and their changes reach the editors, feeds and
//!   webhooks that the browser's do. See [`tools`] and [`resources`].
//!
//! Each tool call runs in a span `mcp <tool>` and is measured as RED metrics of [`SUBSYSTEM`]
//! with the tool's name as its operation; resource requests likewise. Nothing logs a page's
//! content, a value or a token.

mod resources;
mod server;
mod sessions;
mod tools;
mod watch;

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, ORIGIN, WWW_AUTHENTICATE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt as _;
use http_body_util::BodyExt as _;
use http_body_util::combinators::BoxBody;
use neon_app::auth::AuthService;
use neon_app::databases::DatabaseService;
use neon_app::pages::{AccessChange, AccessChanges, PageService};
use neon_app::ratelimit::{Bucket, Limited, RateLimiter};
use neon_app::settings::{SettingsService, mcp as keys};
use neon_app::sync::{Broadcaster, SyncService};
use neon_app::workspaces::WorkspaceService;
use neon_domain::agents::IdentityMode;
use neon_domain::auth::{Principal, UserKind};
use neon_domain::oauth::RESOURCE_METADATA_PATH;
use neon_observability::red::Red;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::common::http_header::HEADER_SESSION_ID;
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::http::{ApiError, AppState, WebConfig};
use server::Server;
use sessions::{Activity, Sessions};

/// RED subsystem of the MCP server's tools and resource requests.
pub const SUBSYSTEM: &str = "mcp";

/// Where the server is mounted: the resource OAuth's tokens for MCP name.
pub const PATH: &str = "/mcp";

/// The span of the HTTP request a call came in, for the call's own span to be under.
#[derive(Clone)]
pub(crate) struct RequestSpan(pub(crate) tracing::Span);

/// The largest request body the transport reads.
const MAX_BODY: usize = 2 * 1024 * 1024;

/// What the tools and resources use: the app's services, as the API's routes do.
#[derive(Clone)]
pub struct Services {
    pub settings: Arc<SettingsService>,
    pub auth: AuthService,
    pub workspaces: WorkspaceService,
    pub pages: PageService,
    pub sync: SyncService,
    pub databases: DatabaseService,
    /// Where pages' documents and databases tell of their changes, which subscriptions
    /// follow.
    pub broadcaster: Arc<dyn Broadcaster>,
}

/// The MCP server. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct Mcp {
    inner: Arc<Inner>,
}

struct Inner {
    transport: StreamableHttpService<Server, Sessions>,
    sessions: Arc<Sessions>,
    settings: Arc<SettingsService>,
    limiter: Arc<dyn RateLimiter>,
    stop: CancellationToken,
}

/// Tells the MCP server of access changes. Give it to the services that change access.
pub struct AccessListener(mpsc::UnboundedSender<AccessChange>);

impl AccessChanges for AccessListener {
    fn access_changed(&self, change: AccessChange) {
        // Once the server stops, nobody needs to hear.
        let _ = self.0.send(change);
    }
}

/// What an [`AccessListener`] hears, for the MCP server's sessions to look again at what
/// they act as.
pub struct AccessFeed(mpsc::UnboundedReceiver<AccessChange>);

/// A listener for the services that change access, and the feed for the MCP server.
pub fn access_feed() -> (Arc<AccessListener>, AccessFeed) {
    let (changes, heard) = mpsc::unbounded_channel();
    (Arc::new(AccessListener(changes)), AccessFeed(heard))
}

impl Mcp {
    /// The MCP server on `services`, counting requests in `limiter`, hearing of access changes
    /// from `feed`. Starts the task that ends idle sessions and those whose access ended, so
    /// call it inside the runtime, after telemetry is installed.
    pub fn new(services: Services, limiter: Arc<dyn RateLimiter>, feed: AccessFeed) -> Self {
        let AccessFeed(heard) = feed;
        let stop = CancellationToken::new();
        let red = Red::new(SUBSYSTEM);
        let sessions = Sessions::start(services.clone(), heard, stop.clone());
        let server = Server::new(services.clone(), Arc::clone(&sessions), red);
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(true)
            // The gate in front checks the Origin itself, as the server's other routes do, and
            // every request's token besides; the Host is the proxy's to keep.
            .disable_allowed_hosts()
            .disable_allowed_origins()
            .with_max_request_body_bytes(MAX_BODY)
            .with_cancellation_token(stop.clone());
        let transport =
            StreamableHttpService::new(move || Ok(server.clone()), Arc::clone(&sessions), config);
        Self {
            inner: Arc::new(Inner {
                transport,
                sessions,
                settings: services.settings,
                limiter,
                stop,
            }),
        }
    }

    /// How many sessions are open.
    pub fn session_count(&self) -> usize {
        self.inner.sessions.count()
    }

    /// How many subscriptions the sessions hold together.
    pub fn subscription_count(&self) -> usize {
        self.inner.sessions.subscriptions()
    }

    /// Ends every session, and refuses new ones: their streams close, so the HTTP server's
    /// drain does not wait on them.
    pub async fn shutdown(&self) {
        self.inner.stop.cancel();
        self.inner.sessions.close_all().await;
    }
}

/// The route of every request to `/mcp`: the gate the module docs describe, then the SDK's
/// transport.
pub(crate) async fn serve(State(state): State<AppState>, request: Request) -> Response {
    let mcp = state.mcp.clone();
    let (mut parts, body) = request.into_parts();
    let headers = &parts.headers;
    if let Some(origin) = headers.get(ORIGIN) {
        let origin = origin.to_str().unwrap_or_default();
        if !own_origin(&state.web, origin, headers) {
            warn!(origin, "cross-origin MCP request refused");
            return ApiError::forbidden("MCP requests from another origin are refused")
                .into_response();
        }
    }
    let session = headers
        .get(HEADER_SESSION_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let principal = match authenticate(&state, headers).await {
        Ok(principal) => principal,
        Err(refusal) => {
            if let Some(id) = &session
                && !refusal.status().is_server_error()
            {
                mcp.inner.sessions.look_again_soon(id);
            }
            if refusal.status() == StatusCode::UNAUTHORIZED {
                return challenge(&state, &parts, refusal);
            }
            return refusal.into_response();
        }
    };
    if let Err(limited) = rate(&mcp, &principal).await {
        warn!(user_id = %principal.user.id, "MCP requests rate limited");
        return ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many MCP requests: slow down",
        )
        .with_retry_after(limited.retry_after)
        .into_response();
    }
    let activity = match &session {
        Some(id) => match mcp.inner.sessions.enter(id, &principal) {
            Some(activity) => Some(activity),
            None => {
                return ApiError::not_found(
                    "no such MCP session: it ended, or is another identity's; initialize anew",
                )
                .into_response();
            }
        },
        None => None,
    };
    let (body, opening) = match (&session, &parts.method) {
        (None, &Method::POST) => {
            // Outside a session, only `initialize` is taken: it opens one.
            let bytes = match axum::body::to_bytes(body, MAX_BODY).await {
                Ok(bytes) => bytes,
                Err(_) => {
                    return ApiError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "too_large",
                        "an MCP request holds at most 2 MiB",
                    )
                    .into_response();
                }
            };
            if !initializes(&bytes) {
                return session_required();
            }
            match mcp.inner.sessions.reserve(&principal, limits(&mcp)) {
                Ok(opening) => (Body::from(bytes), Some(opening)),
                Err(refusal) => {
                    return ApiError::new(
                        StatusCode::TOO_MANY_REQUESTS,
                        "too_many_sessions",
                        refusal.to_string(),
                    )
                    .into_response();
                }
            }
        }
        // A stream, or the end of a session, needs the session.
        (None, &Method::GET | &Method::DELETE) => return session_required(),
        _ => (body, None),
    };
    parts.extensions.insert(principal.clone());
    // The SDK answers on a task of the session's, outside the request's span: its calls
    // name this one as their parent, so a trace holds the request and what it did.
    parts
        .extensions
        .insert(RequestSpan(tracing::Span::current()));
    let response = mcp
        .inner
        .transport
        .handle(Request::from_parts(parts, body))
        .await;
    if let Some(opening) = opening
        && let Some(id) = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
    {
        opening.open(id, principal);
    }
    track(response, activity)
}

/// Whether a request's `Origin` is the server's own: `server.public_url`'s, or else the
/// `Host`'s while that names this machine or an address. A page that points a name of its
/// own at the server's address sends that name as both, so without `server.public_url` an
/// origin named by a domain is refused.
fn own_origin(web: &WebConfig, origin: &str, headers: &HeaderMap) -> bool {
    if !web.same_origin(origin, headers) {
        return false;
    }
    if web.knows_its_origin() {
        return true;
    }
    url::Url::parse(origin).is_ok_and(|url| match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => true,
        None => false,
    })
}

/// What a request outside a session gets, but an `initialize`.
fn session_required() -> Response {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "session_required",
        "MCP requests belong to a session: send initialize first, then its Mcp-Session-Id \
         with every request",
    )
    .into_response()
}

/// Whether a request's body is a JSON-RPC `initialize` request.
fn initializes(body: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Request<'a> {
        #[serde(borrow)]
        method: Option<std::borrow::Cow<'a, str>>,
    }
    serde_json::from_slice::<Request<'_>>(body)
        .is_ok_and(|request| request.method.as_deref() == Some("initialize"))
}

/// Takes a request from the identity's rate, then from its person's, theirs and their agents'
/// together.
async fn rate(mcp: &Mcp, principal: &Principal) -> Result<(), Limited> {
    let limiter = &mcp.inner.limiter;
    limiter
        .check(Bucket::Mcp, &principal.user.id.to_string())
        .await?;
    limiter
        .check(
            Bucket::McpPerson,
            &sessions::person_of(principal).to_string(),
        )
        .await
}

/// How many sessions there may be, as the settings say now.
fn limits(mcp: &Mcp) -> sessions::Limits {
    let settings = mcp.inner.settings.current();
    let count = |key| usize::try_from(*settings.get(key)).unwrap_or(1);
    sessions::Limits {
        overall: count(keys::MAX_SESSIONS),
        per_identity: count(keys::MAX_SESSIONS_PER_IDENTITY),
        per_person: count(keys::MAX_SESSIONS_PER_PERSON),
    }
}

/// The SDK's answer as axum's, the session counted as active until its body ends: an event
/// stream stays open while the client listens.
fn track(
    response: axum::http::Response<BoxBody<Bytes, Infallible>>,
    activity: Option<Activity>,
) -> Response {
    let Some(activity) = activity else {
        return response.map(Body::new);
    };
    response.map(|body| {
        let frames = body.into_data_stream().map(move |frame| {
            let _held = &activity;
            frame
        });
        Body::from_stream(frames)
    })
}

/// The request's principal, by its bearer token alone, when it may use the MCP server: bound
/// to one workspace it is still in, and an identity the server lets connect.
async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<Principal, ApiError> {
    let Some(header) = headers.get(AUTHORIZATION) else {
        return Err(ApiError::unauthenticated(
            "the MCP server wants Authorization: Bearer and a token bound to one workspace: \
             connect through OAuth, or with an agent's token or a person's made for it",
        ));
    };
    let principal = crate::http::principal::bearer(state, header).await?;
    let refused = |code, message: &'static str| ApiError::new(StatusCode::FORBIDDEN, code, message);
    let Some(workspace_id) = principal.workspace() else {
        return Err(refused(
            "token_not_bound",
            "the MCP server works in one workspace: connect with a token bound to it, an agent's \
             token, a person's token made for that workspace, or an OAuth grant",
        ));
    };
    let identities = state.settings.current();
    let identities = identities.get(keys::IDENTITIES);
    match principal.user.kind {
        UserKind::Human if !IdentityMode::User.among(identities) => {
            return Err(refused(
                "identity_not_allowed",
                "MCP clients act as agents alone on this server: connect with an agent's token",
            ));
        }
        UserKind::Integration => {
            return Err(refused(
                "identity_not_allowed",
                "integrations deliver incoming webhooks, and do not connect over MCP",
            ));
        }
        _ => {}
    }
    if let Err(error) = state.workspaces.admits(&principal, workspace_id).await {
        let error = ApiError::from(error);
        if error.status().is_server_error() {
            return Err(error);
        }
        return Err(refused(
            "token_unusable",
            "the token's workspace is not one its identity is in now",
        ));
    }
    Ok(principal)
}

/// A 401 that names where the OAuth server publishes the metadata of `/mcp` (RFC 9728),
/// below the public origin its metadata names, so a client finds how to get a token.
fn challenge(state: &AppState, parts: &Parts, error: ApiError) -> Response {
    let origin = state.web.request_origin(parts);
    let mut response = error.into_response();
    let value = format!("Bearer resource_metadata=\"{origin}{RESOURCE_METADATA_PATH}\"");
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().insert(WWW_AUTHENTICATE, value);
    }
    response
}
