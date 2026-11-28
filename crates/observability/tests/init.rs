//! `init` installs process-wide state, so it runs in this separate test binary.

use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use neon_observability::admin::{self, Readiness};
use neon_observability::config::{LogFormat, OtlpProtocol};
use neon_observability::red::{Buckets, Red};
use neon_observability::{Config, InitError, ServiceInfo, Telemetry, init, propagation};
use tower::ServiceExt as _;

fn service() -> ServiceInfo {
    ServiceInfo::new("neon-test", "0.0.0")
}

fn quiet() -> Config {
    let mut config = Config::default();
    config.logging.level = "warn".to_owned();
    config.logging.format = LogFormat::Json;
    config
}

async fn metrics(telemetry: &Telemetry) -> String {
    let response = admin::router(telemetry, Readiness::always())
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("the router is infallible");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("an in-memory body");
    String::from_utf8(body.to_vec()).expect("Prometheus text is UTF-8")
}

#[tokio::test]
async fn installs_traces_propagation_and_metrics_once() {
    let telemetry = init(&quiet(), &service(), &[("jobs", Buckets::XLong)]).unwrap();

    let traceparent = tracing::info_span!("enqueue").in_scope(propagation::current_traceparent);
    let traceparent = traceparent.expect("spans get a trace context without export");
    assert!(traceparent.starts_with("00-"), "{traceparent}");

    Red::new("jobs").start("send_email").ok();
    let output = metrics(&telemetry).await;
    assert!(
        output.contains(
            r#"neon_jobs_operation_duration_seconds_bucket{operation="send_email",status="ok",le="300"} 1"#
        ),
        "{output}"
    );

    let again = init(&quiet(), &service(), &[]);
    assert!(matches!(again, Err(InitError::AlreadyInitialized)));

    telemetry.shutdown().run().await.unwrap();
    telemetry.shutdown().run().await.unwrap();
}

#[tokio::test]
async fn rejects_invalid_config_before_installing_anything() {
    let mut config = quiet();
    config.tracing.protocol = OtlpProtocol::Grpc;
    config.tracing.sample_ratio = 2.0;

    match init(&config, &service(), &[]) {
        Err(InitError::Config(invalid)) => assert_eq!(invalid.errors.len(), 2),
        other => panic!("expected a config error, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_malformed_subsystems() {
    let result = init(&quiet(), &service(), &[("web-socket", Buckets::Short)]);
    assert!(matches!(result, Err(InitError::Subsystem(name)) if name == "web-socket"));
}

#[test]
fn needs_a_tokio_runtime() {
    assert!(matches!(
        init(&quiet(), &service(), &[]),
        Err(InitError::NoRuntime)
    ));
}
