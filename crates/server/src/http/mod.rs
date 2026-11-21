//! The HTTP API under `/api/v1`, its description, and the middleware every request
//! passes.

mod error;
mod extract;
mod meta;
mod openapi;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, SET_COOKIE};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use neon_app::settings::SettingsService;
use neon_domain::duration;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::sensitive_headers::{
    SetSensitiveRequestHeadersLayer, SetSensitiveResponseHeadersLayer,
};
use tower_http::trace::TraceLayer;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub use error::{ApiError, ErrorBody, ErrorDetail};
pub use extract::{Json, Path, Query};
pub use meta::Meta;
pub use openapi::{spec, spec_json};

/// What the handlers share.
#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<SettingsService>,
}

/// Every API route, each registered together with its entry in the description.
fn api() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(openapi::base()).nest(
        "/api/v1",
        OpenApiRouter::new().routes(routes!(meta::get_meta)),
    )
}

/// The server's HTTP service.
pub fn router(state: AppState) -> Router {
    app(api(), Limits::default()).with_state(state)
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
fn app(api: OpenApiRouter<AppState>, limits: Limits) -> Router<AppState> {
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
                .layer(TraceLayer::new_for_http())
                .layer(SetSensitiveResponseHeadersLayer::new([SET_COOKIE]))
                .layer(CatchPanicLayer::custom(error::panicked))
                .layer(middleware::from_fn(move |request: Request, next: Next| {
                    time_limit(limits.timeout, request, next)
                }))
                .layer(DefaultBodyLimit::max(limits.body)),
        )
}

async fn time_limit(timeout: Duration, request: Request, next: Next) -> Response {
    match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "timeout",
            format!(
                "the server did not answer within {}",
                duration::format(timeout)
            ),
        )
        .into_response(),
    }
}

#[cfg(test)]
mod tests;
