//! The internal admin listener: metrics, probes and profiles.
//!
//! It runs as a separate server so none of this is reachable through the public port. Keep it
//! on loopback or an internal network: anyone who reaches it can read memory contents through
//! heap profiles and hold the CPU profiler for a minute.
//!
//! | Route | Answer |
//! |---|---|
//! | `GET /livez` | always `200`: the process is up and serving |
//! | `GET /readyz` | `200`, or `503` with the reason when the readiness check fails, times out or shutdown has begun |
//! | `GET /metrics` | Prometheus text format, when metrics are enabled |
//! | `GET /debug/pprof/*` | profiles, in pull mode; see [`crate::profiling`] |

use std::fmt::Display;
use std::future::{Future, IntoFuture as _};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::StatusCode;
use http::header::{CONTENT_TYPE, HeaderValue};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::Telemetry;
use crate::config::AdminConfig;
use crate::profiling;

/// How long `/readyz` waits for the readiness check. Probes have short timeouts of their own,
/// and a check that hangs must read as not ready rather than as a dead listener.
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(5);

const PROMETHEUS_TEXT: &str = "text/plain; version=0.0.4; charset=utf-8";

type CheckFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// The caller's answer to "can this instance take traffic?", such as a database ping.
#[derive(Clone)]
pub struct Readiness(Arc<dyn Fn() -> CheckFuture + Send + Sync>);

impl Readiness {
    /// Runs `check` on every `/readyz` request. An error is reported as the reason.
    pub fn new<F, Fut, E>(check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: Display,
    {
        Self(Arc::new(move || {
            let check = check();
            Box::pin(async move { check.await.map_err(|error| error.to_string()) })
        }))
    }

    /// Ready whenever the process is not shutting down.
    #[must_use]
    pub fn always() -> Self {
        Self::new(|| std::future::ready(Ok::<(), std::convert::Infallible>(())))
    }

    async fn check(&self) -> Result<(), String> {
        match tokio::time::timeout(READINESS_TIMEOUT, (self.0)()).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "readiness check timed out after {READINESS_TIMEOUT:?}"
            )),
        }
    }
}

impl std::fmt::Debug for Readiness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Readiness")
    }
}

/// The admin routes, without a listener. Useful for tests and custom servers.
pub fn router(telemetry: &Telemetry, readiness: Readiness) -> Router {
    let shutdown = telemetry.shutdown().clone();
    let mut router = Router::new().route("/livez", get(|| async { "ok" })).route(
        "/readyz",
        get(move || {
            let shutdown = shutdown.clone();
            let readiness = readiness.clone();
            async move { ready(shutdown.is_started(), &readiness).await }
        }),
    );
    if let Some(handle) = telemetry.metrics.clone() {
        router = router.route(
            "/metrics",
            get(move || {
                let handle = handle.clone();
                async move {
                    (
                        [(CONTENT_TYPE, HeaderValue::from_static(PROMETHEUS_TEXT))],
                        handle.render(),
                    )
                }
            }),
        );
    }
    if let Some(pull) = &telemetry.pull {
        router = router.merge(profiling::router(Arc::clone(pull)));
    }
    router
}

/// A load balancer must stop sending traffic as soon as shutdown begins, before the check
/// itself starts failing because connections are closing.
async fn ready(shutting_down: bool, readiness: &Readiness) -> Response {
    if shutting_down {
        return (StatusCode::SERVICE_UNAVAILABLE, "shutting down").into_response();
    }
    match readiness.check().await {
        Ok(()) => "ok".into_response(),
        Err(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason).into_response(),
    }
}

/// Binds `config.addr`, serves [`router`] in the background and registers its graceful stop
/// with the telemetry's shutdown registry.
///
/// Returns the bound address, or `None` when the listener is disabled. Call it early, before
/// the public server starts: the registry stops it after everything registered later, so
/// `/readyz` reports the shutdown and `/metrics` stays up while the app drains.
///
/// Must be called from within a Tokio runtime.
///
/// # Errors
///
/// Returns the error from binding the address.
pub async fn spawn(
    config: &AdminConfig,
    telemetry: &Telemetry,
    readiness: Readiness,
) -> io::Result<Option<SocketAddr>> {
    if !config.enabled {
        return Ok(None);
    }
    let listener = TcpListener::bind(config.addr).await?;
    let addr = listener.local_addr()?;
    if addr.ip().is_loopback() {
        tracing::info!(%addr, "admin listener started");
    } else {
        tracing::warn!(
            %addr,
            "admin listener is bound to a non-loopback address; keep it off public networks, \
             it serves profiles and memory contents"
        );
    }

    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(
        axum::serve(listener, router(telemetry, readiness))
            .with_graceful_shutdown(async move {
                let _ = stopped.await;
            })
            .into_future(),
    );
    telemetry
        .shutdown()
        .on_shutdown("admin listener", move || async move {
            let _ = stop.send(());
            server.await.map_err(io::Error::other)?
        });
    Ok(Some(addr))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use http::Request;
    use tower::ServiceExt as _;

    use super::*;
    use crate::red::{Buckets, Red};

    async fn get(router: &Router, uri: &str) -> (StatusCode, String) {
        let response = router
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn livez_is_always_ok() {
        let router = router(
            &Telemetry::noop(),
            Readiness::new(|| async { Err("db down") }),
        );
        assert_eq!(
            get(&router, "/livez").await,
            (StatusCode::OK, "ok".to_owned())
        );
    }

    #[tokio::test]
    async fn readyz_reports_the_check() {
        let telemetry = Telemetry::noop();
        let ok = router(&telemetry, Readiness::always());
        assert_eq!(get(&ok, "/readyz").await, (StatusCode::OK, "ok".to_owned()));

        let failing = router(
            &telemetry,
            Readiness::new(|| async { Err("database unreachable") }),
        );
        assert_eq!(
            get(&failing, "/readyz").await,
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "database unreachable".to_owned()
            )
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readyz_fails_when_the_check_hangs() {
        let router = router(
            &Telemetry::noop(),
            Readiness::new(|| async {
                std::future::pending::<()>().await;
                Ok::<(), String>(())
            }),
        );
        let (status, body) = get(&router, "/readyz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("timed out"), "{body}");
    }

    #[tokio::test]
    async fn readyz_fails_once_shutdown_starts() {
        let telemetry = Telemetry::noop();
        let router = router(&telemetry, Readiness::always());
        telemetry.shutdown().run().await.unwrap();
        assert_eq!(
            get(&router, "/readyz").await,
            (StatusCode::SERVICE_UNAVAILABLE, "shutting down".to_owned())
        );
    }

    #[tokio::test]
    async fn metrics_renders_prometheus_text() {
        let recorder = crate::prometheus::recorder(&[("http", Buckets::Medium)]).unwrap();
        let telemetry = Telemetry {
            metrics: Some(recorder.handle()),
            ..Telemetry::noop()
        };
        metrics::with_local_recorder(&recorder, || Red::new("http").start("GET /").ok());

        let response = router(&telemetry, Readiness::always())
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], PROMETHEUS_TEXT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            body.contains(r#"neon_http_operations_total{operation="GET /",status="ok"} 1"#),
            "{body}"
        );
    }

    #[tokio::test]
    async fn routes_follow_the_configuration() {
        let router = router(&Telemetry::noop(), Readiness::always());
        assert_eq!(get(&router, "/metrics").await.0, StatusCode::NOT_FOUND);
        assert_eq!(
            get(&router, "/debug/pprof/heap").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn spawn_serves_until_shutdown() {
        let telemetry = Telemetry::noop();
        let config = AdminConfig {
            enabled: true,
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        };
        let addr = spawn(&config, &telemetry, Readiness::always())
            .await
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(addr).await.is_ok());

        telemetry.shutdown().run().await.unwrap();
        assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    }

    #[tokio::test]
    async fn disabled_listener_binds_nothing() {
        let config = AdminConfig {
            enabled: false,
            ..AdminConfig::default()
        };
        let spawned = spawn(&config, &Telemetry::noop(), Readiness::always()).await;
        assert_eq!(spawned.unwrap(), None);
    }
}
