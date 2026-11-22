use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use neon_app::jobs::{
    Handler, Job, JobError, NewJob, RunFuture, RunHook, StartError, Worker, WorkerConfig, enqueue,
};
use neon_domain::jobs::Backoff;
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const KIND: &str = "test.record";

/// What the test handler does on each run.
#[derive(Clone, Copy)]
enum Behavior {
    Succeed,
    Fail,
    FailPermanently,
    Panic,
    Sleep(Duration),
}

/// Runs jobs of [`KIND`] and remembers when each run started.
#[derive(Clone)]
struct Recorder {
    behavior: Behavior,
    runs: Arc<Mutex<Vec<(Uuid, Instant)>>>,
}

impl Recorder {
    fn new(behavior: Behavior) -> Self {
        Self {
            behavior,
            runs: Arc::default(),
        }
    }

    fn runs(&self) -> Vec<(Uuid, Instant)> {
        self.runs.lock().expect("runs lock").clone()
    }
}

#[async_trait]
impl Handler for Recorder {
    fn kind(&self) -> &'static str {
        KIND
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        self.runs
            .lock()
            .expect("runs lock")
            .push((job.id, Instant::now()));
        match self.behavior {
            Behavior::Succeed => Ok(()),
            Behavior::Fail => Err(JobError::retry("boom")),
            Behavior::FailPermanently => Err(JobError::permanent("bad payload")),
            Behavior::Panic => panic!("handler bug"),
            Behavior::Sleep(duration) => {
                tokio::time::sleep(duration).await;
                Ok(())
            }
        }
    }
}

/// Counts finished runs per kind and outcome, as a metrics hook would.
#[derive(Clone, Default)]
struct Outcomes(Arc<Mutex<HashMap<(String, bool), usize>>>);

impl RunHook for Outcomes {
    fn around(&self, job: &Job, run: RunFuture) -> RunFuture {
        let counts = Arc::clone(&self.0);
        let kind = job.kind.clone();
        Box::pin(async move {
            let result = run.await;
            *counts
                .lock()
                .expect("outcomes lock")
                .entry((kind, result.is_ok()))
                .or_default() += 1;
            result
        })
    }
}

fn config() -> WorkerConfig {
    WorkerConfig {
        concurrency: 4,
        poll_interval: Duration::from_millis(50),
        run_timeout: Duration::from_secs(5),
        lease_timeout: Duration::from_secs(10),
        maintenance_interval: Duration::from_millis(100),
        retention: Duration::from_secs(3600),
        backoff: Backoff {
            base: Duration::from_millis(40),
            max: Duration::from_millis(200),
        },
        stop_timeout: Duration::from_secs(5),
    }
}

fn job() -> NewJob {
    NewJob::new(KIND, json!({}))
}

async fn status(pool: &PgPool, id: Uuid) -> (String, i32, Option<String>) {
    sqlx::query_as("SELECT status, attempts, last_error FROM jobs WHERE id = $1")
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
        status(pool, id).await.0 == want
    })
    .await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn runs_queued_jobs_through_the_hooks_and_span(pool: PgPool) {
    let recorder = Recorder::new(Behavior::Succeed);
    let outcomes = Outcomes::default();
    let spans = Arc::new(AtomicUsize::new(0));
    let made = Arc::clone(&spans);
    let queued = enqueue(&pool, &job()).await.unwrap();

    let worker = Worker::new(pool.clone(), config())
        .handler(recorder.clone())
        .hook(outcomes.clone())
        .span(move |job| {
            made.fetch_add(1, Ordering::SeqCst);
            tracing::info_span!("job", kind = %job.kind, traceparent = ?job.traceparent)
        })
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, queued, "done").await;
    worker.stop().await;

    assert_eq!(status(&pool, queued).await, ("done".into(), 1, None));
    assert_eq!(recorder.runs().len(), 1);
    assert_eq!(spans.load(Ordering::SeqCst), 1);
    assert_eq!(
        *outcomes.0.lock().unwrap(),
        HashMap::from([((KIND.to_owned(), true), 1)])
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn two_workers_never_run_the_same_job(pool: PgPool) {
    let recorder = Recorder::new(Behavior::Sleep(Duration::from_millis(5)));
    let mut ids = Vec::new();
    for _ in 0..40 {
        ids.push(enqueue(&pool, &job()).await.unwrap());
    }

    let first = Worker::new(pool.clone(), config())
        .handler(recorder.clone())
        .start()
        .await
        .unwrap();
    let second = Worker::new(pool.clone(), config())
        .handler(recorder.clone())
        .start()
        .await
        .unwrap();
    assert_ne!(first.id(), second.id());
    eventually("every job to be done", || async {
        let done: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE status = 'done'")
            .fetch_one(&pool)
            .await
            .unwrap();
        done == 40
    })
    .await;
    first.stop().await;
    second.stop().await;

    let mut runs: HashMap<Uuid, usize> = HashMap::new();
    for (id, _) in recorder.runs() {
        *runs.entry(id).or_default() += 1;
    }
    assert_eq!(runs.len(), 40);
    assert!(runs.values().all(|&count| count == 1), "{runs:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn failures_retry_with_backoff_then_the_job_dies(pool: PgPool) {
    let recorder = Recorder::new(Behavior::Fail);
    let outcomes = Outcomes::default();
    let id = enqueue(&pool, &job().max_attempts(3)).await.unwrap();

    let worker = Worker::new(pool.clone(), config())
        .handler(recorder.clone())
        .hook(outcomes.clone())
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, id, "dead").await;
    worker.stop().await;

    assert_eq!(
        status(&pool, id).await,
        ("dead".into(), 3, Some("boom".into()))
    );
    let starts: Vec<Instant> = recorder.runs().into_iter().map(|(_, at)| at).collect();
    assert_eq!(starts.len(), 3);
    // At least half of each backoff step passes: 40ms, then 80ms.
    assert!(starts[1] - starts[0] >= Duration::from_millis(20));
    assert!(starts[2] - starts[1] >= Duration::from_millis(40));
    assert_eq!(
        *outcomes.0.lock().unwrap(),
        HashMap::from([((KIND.to_owned(), false), 3)])
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn permanent_failures_and_panics_skip_or_use_retries(pool: PgPool) {
    let permanent = enqueue(&pool, &job()).await.unwrap();
    let worker = Worker::new(pool.clone(), config())
        .handler(Recorder::new(Behavior::FailPermanently))
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, permanent, "dead").await;
    worker.stop().await;
    assert_eq!(
        status(&pool, permanent).await,
        ("dead".into(), 1, Some("bad payload".into()))
    );

    let panicking = enqueue(&pool, &job().max_attempts(2)).await.unwrap();
    let worker = Worker::new(pool.clone(), config())
        .handler(Recorder::new(Behavior::Panic))
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, panicking, "dead").await;
    worker.stop().await;
    assert_eq!(
        status(&pool, panicking).await,
        (
            "dead".into(),
            2,
            Some("the handler panicked: handler bug".into())
        )
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn runs_past_the_timeout_fail(pool: PgPool) {
    let id = enqueue(&pool, &job().max_attempts(1)).await.unwrap();
    let config = WorkerConfig {
        run_timeout: Duration::from_millis(100),
        ..config()
    };

    let worker = Worker::new(pool.clone(), config)
        .handler(Recorder::new(Behavior::Sleep(Duration::from_secs(60))))
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, id, "dead").await;
    worker.stop().await;

    assert_eq!(
        status(&pool, id).await,
        ("dead".into(), 1, Some("timed out after 100ms".into()))
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_job_held_by_a_dead_worker_is_reclaimed(pool: PgPool) {
    let id = enqueue(&pool, &job()).await.unwrap();
    let claimed = neon_store::jobs::claim(&pool, "crashed", &[KIND.to_owned()], 1)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    sqlx::query("UPDATE jobs SET locked_at = now() - interval '1 hour' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let recorder = Recorder::new(Behavior::Succeed);

    let worker = Worker::new(pool.clone(), config())
        .handler(recorder.clone())
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, id, "done").await;
    worker.stop().await;

    assert_eq!(status(&pool, id).await.1, 2);
    assert_eq!(recorder.runs().len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_notification_wakes_the_worker_before_the_next_poll(pool: PgPool) {
    let config = WorkerConfig {
        poll_interval: Duration::from_secs(3600),
        maintenance_interval: Duration::from_secs(3600),
        ..config()
    };
    let worker = Worker::new(pool.clone(), config)
        .handler(Recorder::new(Behavior::Succeed))
        .start()
        .await
        .unwrap();
    // Let the first claim, which runs at start, find nothing.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let id = enqueue(&pool, &job()).await.unwrap();
    wait_for_status(&pool, id, "done").await;
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn listening_takes_no_connection_from_the_pool(pool: PgPool) {
    let single = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(2))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let worker = Worker::new(single.clone(), config())
        .handler(Recorder::new(Behavior::Succeed))
        .start()
        .await
        .unwrap();

    let id = enqueue(&single, &job())
        .await
        .expect("the pool's only connection is free");
    wait_for_status(&pool, id, "done").await;
    worker.stop().await;
    single.close().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn stop_lets_running_jobs_finish_and_claims_no_more(pool: PgPool) {
    let recorder = Recorder::new(Behavior::Sleep(Duration::from_millis(300)));
    let running = enqueue(&pool, &job()).await.unwrap();
    let worker = Worker::new(pool.clone(), config())
        .handler(recorder.clone())
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, running, "running").await;

    worker.stop().await;

    assert_eq!(status(&pool, running).await.0, "done");
    let later = enqueue(&pool, &job()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(status(&pool, later).await, ("queued".into(), 0, None));
    assert_eq!(recorder.runs().len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn stop_queues_again_jobs_that_outlast_the_stop_timeout(pool: PgPool) {
    let config = WorkerConfig {
        stop_timeout: Duration::from_millis(100),
        ..config()
    };
    let id = enqueue(&pool, &job()).await.unwrap();
    let worker = Worker::new(pool.clone(), config)
        .handler(Recorder::new(Behavior::Sleep(Duration::from_secs(60))))
        .start()
        .await
        .unwrap();
    wait_for_status(&pool, id, "running").await;

    worker.stop().await;

    assert_eq!(status(&pool, id).await, ("queued".into(), 0, None));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn finished_jobs_past_retention_are_deleted(pool: PgPool) {
    let id = enqueue(&pool, &job()).await.unwrap();
    sqlx::query("UPDATE jobs SET status = 'done', updated_at = now() - interval '2 hours'")
        .execute(&pool)
        .await
        .unwrap();

    let worker = Worker::new(pool.clone(), config())
        .handler(Recorder::new(Behavior::Succeed))
        .start()
        .await
        .unwrap();
    eventually("the old job to be deleted", || async {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        left == 0
    })
    .await;
    worker.stop().await;
}

/// Like [`Recorder`], for a kind whose finished jobs are not kept.
struct Unkept(Recorder);

const UNKEPT: &str = "test.unkept";

#[async_trait]
impl Handler for Unkept {
    fn kind(&self) -> &'static str {
        UNKEPT
    }

    fn keeps_finished(&self) -> bool {
        false
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        self.0.run(job).await
    }
}

async fn exists(pool: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM jobs WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the query runs")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn jobs_of_handlers_that_keep_none_are_deleted_as_they_finish(pool: PgPool) {
    for behavior in [Behavior::Succeed, Behavior::FailPermanently] {
        let recorder = Recorder::new(behavior);
        let id = enqueue(&pool, &NewJob::new(UNKEPT, json!({})))
            .await
            .unwrap();

        let worker = Worker::new(pool.clone(), config())
            .handler(Unkept(recorder.clone()))
            .start()
            .await
            .unwrap();
        eventually("the finished job to be deleted", || async {
            !recorder.runs().is_empty() && !exists(&pool, id).await
        })
        .await;
        worker.stop().await;
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_job_that_will_be_retried_stays_until_it_finishes(pool: PgPool) {
    let recorder = Recorder::new(Behavior::Fail);
    let id = enqueue(&pool, &NewJob::new(UNKEPT, json!({})).max_attempts(2))
        .await
        .unwrap();

    let worker = Worker::new(pool.clone(), config())
        .handler(Unkept(recorder.clone()))
        .start()
        .await
        .unwrap();
    eventually("both attempts to run", || async {
        recorder.runs().len() == 2
    })
    .await;
    eventually("the dead job to be deleted", || async {
        !exists(&pool, id).await
    })
    .await;
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn dead_jobs_left_behind_are_deleted_by_maintenance(pool: PgPool) {
    let unkept = enqueue(&pool, &NewJob::new(UNKEPT, json!({})))
        .await
        .unwrap();
    let kept = enqueue(&pool, &job()).await.unwrap();
    sqlx::query("UPDATE jobs SET status = 'dead'")
        .execute(&pool)
        .await
        .unwrap();

    let worker = Worker::new(pool.clone(), config())
        .handler(Recorder::new(Behavior::Succeed))
        .handler(Unkept(Recorder::new(Behavior::Succeed)))
        .start()
        .await
        .unwrap();
    eventually("the unkept dead job to be deleted", || async {
        !exists(&pool, unkept).await
    })
    .await;
    worker.stop().await;

    assert_eq!(status(&pool, kept).await.0, "dead");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn start_refuses_an_invalid_config(pool: PgPool) {
    let config = WorkerConfig {
        concurrency: 0,
        ..config()
    };

    let result = Worker::new(pool, config).start().await;

    assert!(matches!(result, Err(StartError::Config(_))), "{result:?}");
}
