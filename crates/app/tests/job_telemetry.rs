//! Job runs continue the trace they were queued in and record RED metrics.
//!
//! Telemetry is installed once per process, so these tests sit in their own binary and share
//! one installation.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use neon_app::jobs::{
    Handler, Job, JobError, NewJob, RedHook, SUBSYSTEM, Worker, WorkerConfig, enqueue, run_span,
};
use neon_domain::jobs::Backoff;
use neon_observability::admin::{self, Readiness};
use neon_observability::propagation::current_traceparent;
use neon_observability::red::Buckets;
use neon_observability::{Config, ServiceInfo, Telemetry};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt as _;
use tracing::Instrument as _;
use uuid::Uuid;

/// The telemetry of this test binary, installed by the first test that asks for it.
fn telemetry() -> &'static Telemetry {
    static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();
    TELEMETRY.get_or_init(|| {
        let mut config = Config::default();
        config.logging.level = "off".to_owned();
        neon_observability::init(
            &config,
            &ServiceInfo::new("neon-test", "0.0.0"),
            &[(SUBSYSTEM, Buckets::Long)],
        )
        .expect("telemetry installs once")
    })
}

async fn metrics() -> String {
    let response = admin::router(telemetry(), Readiness::always())
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("the router is infallible");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("an in-memory body");
    String::from_utf8(body.to_vec()).expect("Prometheus text is UTF-8")
}

fn runs_total(kind: &str, status: &str) -> String {
    format!(r#"neon_jobs_operations_total{{operation="{kind}",status="{status}"}}"#)
}

fn config() -> WorkerConfig {
    WorkerConfig {
        poll_interval: Duration::from_millis(50),
        run_timeout: Duration::from_secs(5),
        lease_timeout: Duration::from_secs(10),
        maintenance_interval: Duration::from_millis(100),
        backoff: Backoff {
            base: Duration::from_millis(40),
            max: Duration::from_millis(200),
        },
        stop_timeout: Duration::from_secs(5),
        ..WorkerConfig::default()
    }
}

/// The trace id in a `traceparent`: `00-<trace id>-<span id>-<flags>`.
fn trace_id(traceparent: &str) -> &str {
    traceparent.split('-').nth(1).expect("a W3C traceparent")
}

/// Remembers the `traceparent` current inside each run.
#[derive(Clone, Default)]
struct Witness(Arc<Mutex<HashMap<Uuid, Option<String>>>>);

#[async_trait]
impl Handler for Witness {
    fn kind(&self) -> &'static str {
        "test.traced"
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        self.0
            .lock()
            .expect("witness lock")
            .insert(job.id, current_traceparent());
        Ok(())
    }
}

/// Fails, succeeds or panics as told, by attempt.
struct Scripted {
    kind: &'static str,
    attempt: fn(i32) -> Result<(), JobError>,
}

#[async_trait]
impl Handler for Scripted {
    fn kind(&self) -> &'static str {
        self.kind
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        (self.attempt)(job.attempt)
    }
}

async fn status(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("job exists")
}

/// Waits up to ten seconds for `check` to hold.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_status(pool: &PgPool, id: Uuid, want: &str) {
    eventually(&format!("job {id} to be {want}"), || async {
        status(pool, id).await == want
    })
    .await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_job_queued_inside_a_span_runs_in_the_same_trace(pool: PgPool) {
    telemetry();
    let request = tracing::info_span!("request");
    let request_traceparent = request
        .in_scope(current_traceparent)
        .expect("spans have a trace context");
    let queued = enqueue(&pool, &NewJob::new("test.traced", json!({})))
        .instrument(request.clone())
        .await
        .unwrap();
    let untraced = enqueue(&pool, &NewJob::new("test.traced", json!({})))
        .await
        .unwrap();
    let stored: Option<String> = sqlx::query_scalar("SELECT traceparent FROM jobs WHERE id = $1")
        .bind(untraced)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, None, "a job queued outside a span has no trace");

    let witness = Witness::default();
    let worker = Worker::new(pool.clone(), config())
        .handler(witness.clone())
        .span(run_span)
        .hook(RedHook::new())
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, queued, "done").await;
    wait_for_status(&pool, untraced, "done").await;
    worker.stop().await;

    let seen = witness.0.lock().unwrap().clone();
    let run = seen[&queued]
        .as_deref()
        .expect("the run has a trace context");
    assert_eq!(trace_id(run), trace_id(&request_traceparent));
    assert_ne!(run, request_traceparent, "the run is a span of its own");
    let fresh = seen[&untraced]
        .as_deref()
        .expect("a run without a stored trace starts one");
    assert_ne!(trace_id(fresh), trace_id(&request_traceparent));

    let metrics = metrics().await;
    assert!(
        metrics.contains(&format!("{} 2", runs_total("test.traced", "ok"))),
        "{metrics}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn retries_count_as_ok_and_failures_that_kill_the_job_as_errors(pool: PgPool) {
    telemetry();
    let flaky = enqueue(&pool, &NewJob::new("test.flaky", json!({})))
        .await
        .unwrap();
    let broken = enqueue(&pool, &NewJob::new("test.broken", json!({})))
        .await
        .unwrap();
    let exhausted = enqueue(
        &pool,
        &NewJob::new("test.exhausted", json!({})).max_attempts(1),
    )
    .await
    .unwrap();
    let panicky = enqueue(
        &pool,
        &NewJob::new("test.panicky", json!({})).max_attempts(2),
    )
    .await
    .unwrap();

    let worker = Worker::new(pool.clone(), config())
        .handler(Scripted {
            kind: "test.flaky",
            attempt: |attempt| match attempt {
                1 => Err(JobError::retry("mail server down")),
                _ => Ok(()),
            },
        })
        .handler(Scripted {
            kind: "test.broken",
            attempt: |_| Err(JobError::permanent("bad payload")),
        })
        .handler(Scripted {
            kind: "test.exhausted",
            attempt: |_| Err(JobError::retry("mail server down")),
        })
        .handler(Scripted {
            kind: "test.panicky",
            attempt: |_| panic!("handler bug"),
        })
        .span(run_span)
        .hook(RedHook::new())
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, flaky, "done").await;
    for dead in [broken, exhausted, panicky] {
        wait_for_status(&pool, dead, "dead").await;
    }
    worker.stop().await;

    let metrics = metrics().await;
    for (kind, status, count) in [
        ("test.flaky", "ok", 2),
        ("test.broken", "error", 1),
        ("test.exhausted", "error", 1),
        ("test.panicky", "error", 2),
    ] {
        let line = format!("{} {count}", runs_total(kind, status));
        assert!(metrics.contains(&line), "no {line} in {metrics}");
    }
    for kind in [
        "test.flaky",
        "test.broken",
        "test.exhausted",
        "test.panicky",
    ] {
        assert!(
            metrics.contains(&format!(
                r#"neon_jobs_operations_in_flight{{operation="{kind}"}} 0"#
            )),
            "{metrics}"
        );
    }
    assert!(
        !metrics.contains(&runs_total("test.flaky", "error")),
        "{metrics}"
    );
    assert!(
        !metrics.contains(&runs_total("test.broken", "ok")),
        "{metrics}"
    );
}
