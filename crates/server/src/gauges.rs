//! Gauges of what the server holds: the database pool's connections and the job queue.
//!
//! They are sampled by a background task rather than at each change, since the pool keeps
//! its own counts and the queue lives in the database.

use std::time::Duration;

use metrics::Unit;
use neon_store::jobs;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

/// How often the gauges are sampled.
pub(crate) const INTERVAL: Duration = Duration::from_secs(15);

const POOL_CONNECTIONS: &str = "neon_db_pool_connections";
const POOL_MAX_CONNECTIONS: &str = "neon_db_pool_max_connections";
const QUEUE_DEPTH: &str = "neon_jobs_queue_depth";

/// The running sampler.
pub(crate) struct Gauges {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Gauges {
    /// Describes the gauges, then samples them now and every `interval` until stopped.
    pub(crate) fn spawn(pool: PgPool, interval: Duration) -> Self {
        describe();
        let stop = CancellationToken::new();
        let task = tokio::spawn(run(pool, interval, stop.clone()));
        Self { stop, task }
    }

    /// Stops sampling, waiting for a sample in progress, so the pool can close after it.
    pub(crate) async fn stop(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            error!(%error, "the gauge sampler failed");
        }
    }
}

fn describe() {
    metrics::describe_gauge!(
        POOL_CONNECTIONS,
        Unit::Count,
        "Connections the database pool holds, by state: idle, or in_use by a query or transaction."
    );
    metrics::describe_gauge!(
        POOL_MAX_CONNECTIONS,
        Unit::Count,
        "The most connections the database pool opens."
    );
    metrics::describe_gauge!(
        QUEUE_DEPTH,
        Unit::Count,
        "Jobs in the queue table, by status: queued, running, done or dead."
    );
}

async fn run(pool: PgPool, interval: Duration, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = ticks.tick() => sample(&pool).await,
        }
    }
}

/// Reads the pool before querying, so the query's own connection does not count as in use.
async fn sample(pool: &PgPool) {
    let size = pool.size();
    let idle = u32::try_from(pool.num_idle()).unwrap_or(u32::MAX).min(size);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "idle").set(idle);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "in_use").set(size - idle);
    metrics::gauge!(POOL_MAX_CONNECTIONS).set(pool.options().get_max_connections());

    match jobs::count_by_status(pool).await {
        Ok(counts) => {
            // Statuses no job has are absent from the counts and drop to zero.
            for status in jobs::STATUSES {
                let count = counts
                    .iter()
                    .find_map(|(counted, count)| (counted == status).then_some(*count))
                    .unwrap_or(0);
                // Exact up to 2^53 jobs.
                metrics::gauge!(QUEUE_DEPTH, "status" => status).set(count as f64);
            }
        }
        Err(error) => warn!(%error, "cannot count the jobs for the queue depth gauge"),
    }
}

#[cfg(test)]
mod tests {
    use metrics_util::MetricKind;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
    use neon_store::jobs::NewJob;
    use serde_json::json;

    use super::*;

    /// The gauges recorded, as name, label value and value, sorted.
    fn gauges(snapshotter: &Snapshotter) -> Vec<(String, Option<String>, f64)> {
        let mut gauges: Vec<_> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| {
                let (kind, key) = key.into_parts();
                let DebugValue::Gauge(value) = value else {
                    return None;
                };
                (kind == MetricKind::Gauge).then(|| {
                    let label = key.labels().next().map(|label| label.value().to_owned());
                    (key.name().to_owned(), label, value.into_inner())
                })
            })
            .collect();
        gauges.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        gauges
    }

    #[sqlx::test(migrator = "neon_store::MIGRATOR")]
    async fn samples_the_pool_and_the_queue(pool: PgPool) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        // Tests run on a single-threaded runtime, so the recorder sees every sample.
        let _recording = metrics::set_default_local_recorder(&recorder);
        for _ in 0..2 {
            jobs::enqueue(&pool, &NewJob::new("mail.send", json!({})))
                .await
                .unwrap();
        }
        let held = pool.acquire().await.unwrap();

        sample(&pool).await;

        let sampled = gauges(&snapshotter);
        let names: Vec<_> = sampled
            .iter()
            .map(|(name, label, _)| (name.as_str(), label.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                (POOL_CONNECTIONS, Some("idle")),
                (POOL_CONNECTIONS, Some("in_use")),
                (POOL_MAX_CONNECTIONS, None),
                (QUEUE_DEPTH, Some("dead")),
                (QUEUE_DEPTH, Some("done")),
                (QUEUE_DEPTH, Some("queued")),
                (QUEUE_DEPTH, Some("running")),
            ]
        );
        let value = |at: usize| sampled[at].2;
        // The pool returns connections in the background, so only bounds are certain.
        assert!(
            value(1) >= 1.0,
            "the held connection is in use: {sampled:?}"
        );
        assert!(value(0) + value(1) <= value(2), "{sampled:?}");
        assert_eq!(value(2), f64::from(pool.options().get_max_connections()));
        assert_eq!(
            [value(3), value(4), value(5), value(6)],
            [0.0, 0.0, 2.0, 0.0]
        );
        drop(held);
    }

    #[sqlx::test(migrator = "neon_store::MIGRATOR")]
    async fn stop_ends_the_sampler(pool: PgPool) {
        let gauges = Gauges::spawn(pool, Duration::from_millis(10));
        tokio::time::timeout(Duration::from_secs(5), gauges.stop())
            .await
            .expect("the sampler stops");
    }
}
