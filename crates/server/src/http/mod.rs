//! The HTTP API under `/api/v1`, its description, and the middleware every request
//! passes.

mod account;
mod admin;
mod admin_workspaces;
mod agents;
mod auth;
mod database_live;
mod database_rows;
mod databases;
mod dto;
mod error;
mod extract;
mod favorites;
mod meta;
mod openapi;
mod page_shares;
mod pages;
mod principal;
mod sync;
mod teamspaces;
mod transfer;
mod web;
mod webhooks;
mod workspaces;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use neon_app::auth::AuthService;
use neon_app::databases::{DatabaseService, LiveDatabases};
use neon_app::pages::PageService;
use neon_app::settings::SettingsService;
use neon_app::sync::SyncService;
use neon_app::transfer::TransferService;
use neon_app::webhooks::WebhookService;
use neon_app::workspaces::{AgentService, WorkspaceService};
use neon_domain::duration;
use neon_observability::http::HttpLayer;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::sensitive_headers::{
    SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeadersLayer,
};
use tracing::{debug, error};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use error::{ApiError, ErrorBody, ErrorDetail, ProblemDetail};
pub use extract::{Json, Path, Query};
pub use meta::Meta;
pub use openapi::{spec, spec_json};
pub use web::{SECURE_SESSION_COOKIE, SESSION_COOKIE, WebConfig};

/// What the handlers share.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<SettingsService>,
    pub auth: AuthService,
    pub workspaces: WorkspaceService,
    pub agents: AgentService,
    pub pages: PageService,
    pub sync: SyncService,
    pub databases: DatabaseService,
    pub live: LiveDatabases,
    pub transfer: TransferService,
    pub webhooks: WebhookService,
    pub web: Arc<WebConfig>,
}

/// Every API route, each registered together with its entry in the description.
fn api() -> OpenApiRouter<AppState> {
    let routes = OpenApiRouter::new()
        .routes(routes!(meta::get_meta))
        .routes(routes!(auth::get_config))
        .routes(routes!(auth::register))
        .routes(routes!(auth::login))
        .routes(routes!(auth::logout))
        .routes(routes!(auth::verify_email))
        .routes(routes!(auth::resend_verification))
        .routes(routes!(auth::request_password_reset))
        .routes(routes!(auth::reset_password))
        .routes(routes!(auth::check_invite))
        .routes(routes!(auth::request_to_join))
        .routes(routes!(auth::confirm_join))
        .routes(routes!(account::get_me, account::update_me))
        .routes(routes!(account::change_password))
        .routes(routes!(
            account::list_sessions,
            account::revoke_other_sessions
        ))
        .routes(routes!(account::revoke_session))
        .routes(routes!(account::list_tokens, account::create_token))
        .routes(routes!(account::revoke_token))
        .routes(routes!(account::get_preferences, account::put_preferences))
        .routes(routes!(account::list_my_invitations))
        .routes(routes!(account::accept_invitation))
        .routes(routes!(account::decline_invitation))
        .routes(routes!(admin::list_users, admin::create_user))
        .routes(routes!(admin::update_user))
        .routes(routes!(admin::send_verification))
        .routes(routes!(admin::list_invites, admin::create_invite))
        .routes(routes!(admin::check_invite_code))
        .routes(routes!(admin::invite_code))
        .routes(routes!(admin::revoke_invite))
        .routes(routes!(admin::list_join_requests))
        .routes(routes!(admin::approve_join))
        .routes(routes!(admin::deny_join))
        .routes(routes!(admin::list_settings, admin::update_settings))
        .routes(routes!(admin::send_test_mail))
        .routes(routes!(admin_workspaces::admin_list_workspaces))
        .routes(routes!(admin_workspaces::set_workspace_limits))
        .routes(routes!(admin_workspaces::admin_delete_workspace))
        .routes(routes!(
            workspaces::list_workspaces,
            workspaces::create_workspace
        ))
        .routes(routes!(
            workspaces::get_workspace,
            workspaces::update_workspace,
            workspaces::delete_workspace
        ))
        .routes(routes!(workspaces::list_members))
        .routes(routes!(
            workspaces::change_member_role,
            workspaces::remove_member
        ))
        .routes(routes!(workspaces::leave_workspace))
        .routes(routes!(workspaces::transfer_ownership))
        .routes(routes!(
            workspaces::list_invitations,
            workspaces::create_invitation
        ))
        .routes(routes!(workspaces::revoke_invitation))
        .routes(routes!(
            workspaces::list_invite_links,
            workspaces::create_invite_link
        ))
        .routes(routes!(workspaces::invite_link_token))
        .routes(routes!(workspaces::revoke_invite_link))
        .routes(routes!(workspaces::preview_invite_link))
        .routes(routes!(workspaces::join_with_link))
        .routes(routes!(agents::list_agents, agents::create_agent))
        .routes(routes!(
            agents::get_agent,
            agents::update_agent,
            agents::delete_agent
        ))
        .routes(routes!(agents::transfer_agent))
        .routes(routes!(
            agents::list_agent_tokens,
            agents::create_agent_token
        ))
        .routes(routes!(agents::revoke_agent_token))
        .routes(routes!(webhooks::list_webhooks, webhooks::create_webhook))
        .routes(routes!(
            webhooks::get_webhook,
            webhooks::update_webhook,
            webhooks::delete_webhook
        ))
        .routes(routes!(webhooks::rotate_webhook_secret))
        .routes(routes!(webhooks::webhook_deliveries))
        .routes(routes!(webhooks::test_webhook))
        .routes(routes!(
            teamspaces::list_teamspaces,
            teamspaces::create_teamspace
        ))
        .routes(routes!(
            teamspaces::get_teamspace,
            teamspaces::update_teamspace,
            teamspaces::delete_teamspace
        ))
        .routes(routes!(teamspaces::join_teamspace))
        .routes(routes!(teamspaces::leave_teamspace))
        .routes(routes!(
            teamspaces::list_teamspace_members,
            teamspaces::add_teamspace_member
        ))
        .routes(routes!(
            teamspaces::change_teamspace_role,
            teamspaces::remove_teamspace_member
        ))
        .routes(routes!(pages::create_page))
        .routes(routes!(
            pages::get_page,
            pages::update_page,
            pages::delete_page
        ))
        .routes(routes!(pages::move_page))
        .routes(routes!(pages::trash_page))
        .routes(routes!(pages::restore_page))
        .routes(routes!(pages::page_breadcrumbs))
        .routes(routes!(pages::teamspace_pages))
        .routes(routes!(pages::private_pages))
        .routes(routes!(pages::shared_pages))
        .routes(routes!(pages::workspace_trash))
        .routes(routes!(
            page_shares::list_page_shares,
            page_shares::share_page
        ))
        .routes(routes!(
            page_shares::change_page_share,
            page_shares::remove_page_share
        ))
        .routes(routes!(favorites::list_favorites, favorites::add_favorite))
        .routes(routes!(favorites::move_favorite))
        .routes(routes!(favorites::remove_favorite))
        .routes(routes!(databases::create_database))
        .routes(routes!(databases::get_database))
        .routes(routes!(databases::add_property))
        .routes(routes!(
            databases::change_property,
            databases::delete_property
        ))
        .routes(routes!(databases::move_property))
        .routes(routes!(databases::create_view))
        .routes(routes!(databases::change_view, databases::delete_view))
        .routes(routes!(databases::move_view))
        .routes(routes!(databases::duplicate_view))
        .routes(routes!(database_rows::query_rows))
        .routes(routes!(database_rows::create_row))
        .routes(routes!(database_rows::row_by_unique_id))
        .routes(routes!(database_rows::get_row, database_rows::change_row))
        .routes(routes!(database_rows::move_row))
        .routes(routes!(transfer::export_page))
        .routes(routes!(transfer::export_teamspace))
        // Uploads are larger than the bodies of other routes.
        .merge(
            OpenApiRouter::new()
                .routes(routes!(transfer::import_pages))
                .layer(transfer::import_body_limit()),
        );
    // Real-time sync speaks WebSocket, which the API's description has no words for; the
    // docs describe it.
    OpenApiRouter::with_openapi(openapi::base())
        .nest("/api/v1", routes)
        .route("/ws/pages/{page_id}", get(sync::open_page))
        .route(
            "/ws/databases/{database_id}",
            get(database_live::follow_database),
        )
}

/// The server's HTTP service. `trust_incoming` is `observability.tracing.trust_incoming`:
/// whether requests continue the trace context their callers send.
pub fn router(state: AppState, trust_incoming: bool) -> Router {
    let telemetry = HttpLayer::new().trust_incoming(trust_incoming);
    let web = Arc::clone(&state.web);
    app(api(), Limits::default(), telemetry, web).with_state(state)
}

/// Bounds on each request.
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// The largest body an extractor reads. Larger ones are refused with 413.
    body: usize,
    /// How long a handler may take to answer. Responses that stream, like server-sent
    /// events, are only bound until their headers are sent.
    timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            body: 2 * 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

/// The routes plus the description route, the fallbacks and the middleware.
///
/// Requests that change something from another origin are refused before any handler
/// runs; see [`web`].
///
/// `Router::layer` wraps each route, after routing, so `telemetry` sees the route template.
/// It sits outside the layers that answer for the handler, the panic catcher and the time
/// limit, so its spans and RED metrics record the status the client gets, a 500 for a
/// panic and a 503 for a timeout included. Everything inside it logs within the request's
/// span, so those lines carry its trace id.
fn app(
    api: OpenApiRouter<AppState>,
    limits: Limits,
    telemetry: HttpLayer,
    web: Arc<WebConfig>,
) -> Router<AppState> {
    let (router, spec) = api.split_for_parts();
    let spec = Bytes::from(openapi::render(&spec));

    router
        .route(
            "/api/v1/openapi.json",
            get(move || async move { ([(CONTENT_TYPE, "application/json")], spec) }),
        )
        .fallback(error::no_route)
        .method_not_allowed_fallback(error::wrong_method)
        .layer(
            ServiceBuilder::new()
                // Marked before anything logs a request, so their values never reach a log.
                .layer(SetSensitiveRequestHeadersLayer::new([
                    AUTHORIZATION,
                    COOKIE,
                ]))
                .layer(telemetry)
                .layer(middleware::from_fn(log_answer))
                .layer(SetSensitiveResponseHeadersLayer::new([SET_COOKIE]))
                .layer(CatchPanicLayer::custom(error::panicked))
                .layer(middleware::from_fn(move |request: Request, next: Next| {
                    time_limit(limits.timeout, request, next)
                }))
                .layer(middleware::from_fn_with_state(web, web::same_origin_only))
                .layer(DefaultBodyLimit::max(limits.body)),
        )
}

/// Logs every answer at `debug`. The span already names the route and the request id.
async fn log_answer(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let response = next.run(request).await;
    debug!(
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "request answered"
    );
    response
}

async fn time_limit(timeout: Duration, request: Request, next: Next) -> Response {
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            error!(
                timeout = %duration::format(timeout),
                "a request handler did not answer in time"
            );
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "timeout",
                format!(
                    "the server did not answer within {}",
                    duration::format(timeout)
                ),
            )
            .into_response()
        }
    }
}

#[cfg(test)]
mod agents_tests;
#[cfg(test)]
mod auth_tests;
#[cfg(test)]
mod databases_tests;
#[cfg(test)]
mod pages_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transfer_tests;
#[cfg(test)]
mod webhooks_tests;
#[cfg(test)]
mod workspaces_tests;
