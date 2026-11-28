//! Heap profiles in a binary that runs on the crate's jemalloc wrapper, as a server would.
//!
//! The global allocator covers the whole binary, so this runs apart from the unit tests,
//! which stay on the system allocator.
#![cfg(target_os = "linux")]

use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use neon_observability::admin::{self, Readiness};
use neon_observability::config::{Profile, ProfilingMode};
use neon_observability::profiling::{Jemalloc, MALLOC_CONF_ENV};
use neon_observability::{Config, ServiceInfo, init};
use tower::ServiceExt as _;

#[global_allocator]
static ALLOCATOR: Jemalloc = Jemalloc;

#[tokio::test]
async fn heap_endpoint_answers_by_jemallocs_profiling_switch() {
    assert!(Jemalloc::in_use());

    let mut config = Config::default();
    config.logging.level = "warn".to_owned();
    config.profiling.mode = ProfilingMode::Pull;
    config.profiling.profiles = vec![Profile::Heap];
    let telemetry = init(&config, &ServiceInfo::new("neon-test", "0.0.0"), &[]).unwrap();

    let response = admin::router(&telemetry, Readiness::always())
        .oneshot(
            Request::get("/debug/pprof/heap")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    // jemalloc reads the variable once, before `main`, so the test follows whatever the run
    // was started with.
    let profiling = std::env::var(MALLOC_CONF_ENV).is_ok_and(|conf| conf.contains("prof:true"));
    if profiling {
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..2], &[0x1f, 0x8b], "not a gzipped profile");
    } else {
        let reason = String::from_utf8_lossy(&body);
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{reason}");
        assert!(reason.contains(MALLOC_CONF_ENV), "{reason}");
    }
    telemetry.shutdown().run().await.unwrap();
}
