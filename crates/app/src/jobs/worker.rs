use std::any::Any;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use neon_domain::duration;
use neon_domain::jobs::Backoff;
use neon_store::jobs::{self, CHANNEL, Job};
use sqlx::PgPool;
use sqlx::postgres::{PgListener, PgPoolOptions};
use tokio::sync::Notify;
use tokio::task::{Id, JoinError, JoinHandle, JoinSet};
use tokio::time::{MissedTickBehavior, interval, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument as _, Span, debug, error, info, info_span, warn};
use uuid::Uuid;

use super::handler::{Handler, JobError, RunFuture, RunHook};

/// How long to wait before listening again after the notification connection failed.
const LISTEN_RETRY: Duration = Duration::from_secs(5);

/// Connections a started worker opens beside the pool it is given: one that waits for
/// notifications. It stays outside the pool so it never takes a connection queries need.
pub const LISTEN_CONNECTIONS: u32 = 1;

/// How a worker runs jobs.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerConfig {
    /// Jobs run at once.
    pub concurrency: usize,
    /// How often to look for due jobs without being notified: retries that came due, and
    /// jobs whose notification was lost.
    pub poll_interval: Duration,
    /// How long a run may take before it is cancelled and counted as failed.
    pub run_timeout: Duration,
    /// How long a claim holds before any worker takes the job back, presuming its worker
    /// died. Longer than `run_timeout`, so a live worker always gives up first.
    pub lease_timeout: Duration,
    /// How often to take back expired claims and delete old finished jobs.
    pub maintenance_interval: Duration,
    /// How long done and dead jobs are kept for inspection, except those of handlers that
    /// keep none, which are deleted as they finish.
    pub retention: Duration,
    /// The delays between attempts.
    pub backoff: Backoff,
    /// How long stopping waits for running jobs. Jobs still running then are cancelled
    /// and queued again without counting the attempt.
    pub stop_timeout: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            poll_interval: Duration::from_secs(5),
            run_timeout: Duration::from_secs(10 * 60),
            lease_timeout: Duration::from_secs(15 * 60),
            maintenance_interval: Duration::from_secs(60),
            retention: Duration::from_secs(7 * 24 * 3600),
            backoff: Backoff {
                base: Duration::from_secs(10),
                max: Duration::from_secs(3600),
            },
            stop_timeout: Duration::from_secs(30),
        }
    }
}

impl WorkerConfig {
    fn validate(&self) -> Result<(), String> {
        if self.concurrency == 0 {
            return Err("concurrency must be at least 1".to_owned());
        }
        if self.poll_interval.is_zero() || self.maintenance_interval.is_zero() {
            return Err("poll_interval and maintenance_interval must be above zero".to_owned());
        }
        if self.run_timeout >= self.lease_timeout {
            return Err(
                "run_timeout must be shorter than lease_timeout, or a slow job \
                        would be taken back while it runs and run twice"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("invalid job worker config: {0}")]
    Config(String),
    #[error("listening for queued jobs: {0}")]
    Listen(sqlx::Error),
}

from_cause!(StartError { Listen(sqlx::Error) });

type MakeSpan = Arc<dyn Fn(&Job) -> Span + Send + Sync>;

/// A job worker to start: its handlers by kind, and the hooks around every run.
pub struct Worker {
    pool: PgPool,
    config: WorkerConfig,
    handlers: HashMap<&'static str, Arc<dyn Handler>>,
    hooks: Vec<Arc<dyn RunHook>>,
    span: MakeSpan,
}

impl Worker {
    pub fn new(pool: PgPool, config: WorkerConfig) -> Self {
        Self {
            pool,
            config,
            handlers: HashMap::new(),
            hooks: Vec::new(),
            span: Arc::new(default_span),
        }
    }

    /// Runs the jobs of `handler.kind()`. Jobs of kinds without a handler stay queued.
    ///
    /// # Panics
    ///
    /// When a handler for the same kind is already registered.
    pub fn handler(mut self, handler: impl Handler) -> Self {
        let kind = handler.kind();
        let previous = self.handlers.insert(kind, Arc::new(handler));
        assert!(previous.is_none(), "two handlers for job kind {kind}");
        self
    }

    /// Wraps every run. The first hook added is the outermost.
    pub fn hook(mut self, hook: impl RunHook) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Makes the span each run is recorded in, replacing a root span named `job`. This is
    /// where a tracing integration makes the job's `traceparent` the span's parent, as
    /// [`run_span`](super::run_span) does.
    pub fn span(mut self, make: impl Fn(&Job) -> Span + Send + Sync + 'static) -> Self {
        self.span = Arc::new(make);
        self
    }

    /// Listens for queued jobs and starts claiming them.
    pub async fn start(self) -> Result<WorkerHandle, StartError> {
        self.config.validate().map_err(StartError::Config)?;
        if let Some(kind) = self
            .handlers
            .iter()
            .find_map(|(kind, handler)| (handler.max_running() == Some(0)).then_some(kind))
        {
            return Err(StartError::Config(format!(
                "the jobs of {kind} would never run: a share of the slots is at least one"
            )));
        }

        let id = format!("{}-{}", std::process::id(), Uuid::now_v7().simple());
        let mut kinds: Vec<String> = self.handlers.keys().map(|&k| k.to_owned()).collect();
        kinds.sort();
        let unkept: Vec<String> = kinds
            .iter()
            .filter(|kind| {
                self.handlers
                    .get(kind.as_str())
                    .is_some_and(|handler| !handler.keeps_finished())
            })
            .cloned()
            .collect();

        // A pool of one with the same options, as `PgListener::connect` builds, so the
        // listener reconnects on its own. Its connection is held for the worker's life and
        // never expires.
        let listen_pool = PgPoolOptions::new()
            .max_connections(LISTEN_CONNECTIONS)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect_lazy_with((*self.pool.connect_options()).clone());
        let mut listener = PgListener::connect_with(&listen_pool).await?;
        listener.listen(CHANNEL).await?;
        let wake = Arc::new(Notify::new());
        let listening = tokio::spawn(listen(listener, kinds.clone(), Arc::clone(&wake)));

        info!(worker = %id, ?kinds, "job worker started");
        let span = info_span!("job_worker", worker = %id);
        let inner = Arc::new(Inner {
            pool: self.pool,
            config: self.config,
            id: id.clone(),
            kinds,
            unkept,
            handlers: self.handlers,
            hooks: self.hooks,
            span: self.span,
        });
        let stop_timeout = inner.config.stop_timeout;
        let stop = CancellationToken::new();
        let running = tokio::spawn(inner.run(wake, stop.clone()).instrument(span));

        Ok(WorkerHandle {
            id,
            stop_timeout,
            stop,
            running,
            listening,
            listen_pool,
        })
    }
}

fn default_span(job: &Job) -> Span {
    info_span!(parent: None, "job", kind = %job.kind, id = %job.id, attempt = job.attempt)
}

/// A running worker. It runs until [`stop`](Self::stop) is called, even if the handle is
/// dropped.
#[must_use = "dropping the handle leaves the worker running with no way to stop it"]
#[derive(Debug)]
pub struct WorkerHandle {
    id: String,
    stop_timeout: Duration,
    stop: CancellationToken,
    running: JoinHandle<()>,
    listening: JoinHandle<()>,
    listen_pool: PgPool,
}

impl WorkerHandle {
    /// The name the worker claims jobs under, in the jobs table's `locked_by`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// How long [`stop`](Self::stop) waits for running jobs before it cancels them.
    pub fn stop_timeout(&self) -> Duration {
        self.stop_timeout
    }

    /// Stops claiming jobs, waits up to the stop timeout for running ones, then cancels
    /// the rest and queues them again.
    pub async fn stop(self) {
        self.stop.cancel();
        if let Err(error) = self.running.await {
            error!(worker = %self.id, %error, "job worker failed");
        }
        self.listening.abort();
        // The listener was cancelled; there is nothing to report.
        let _ = self.listening.await;
        self.listen_pool.close().await;
        info!(worker = %self.id, "job worker stopped");
    }
}

/// Turns notifications for the worker's kinds into wake-ups. A notification is only a
/// hint: the claim query decides what runs, so a lost one only delays a job until the
/// next poll.
async fn listen(mut listener: PgListener, kinds: Vec<String>, wake: Arc<Notify>) {
    loop {
        match listener.recv().await {
            Ok(notification) => {
                if kinds.iter().any(|kind| kind == notification.payload()) {
                    wake.notify_one();
                }
            }
            Err(sqlx::Error::PoolClosed) => return,
            Err(error) => {
                warn!(%error, "job notifications lost; polling until they resume");
                sleep(LISTEN_RETRY).await;
            }
        }
    }
}

struct Inner {
    pool: PgPool,
    config: WorkerConfig,
    id: String,
    kinds: Vec<String>,
    /// The kinds whose finished jobs are deleted rather than kept.
    unkept: Vec<String>,
    handlers: HashMap<&'static str, Arc<dyn Handler>>,
    hooks: Vec<Arc<dyn RunHook>>,
    span: MakeSpan,
}

impl Inner {
    async fn run(self: Arc<Self>, wake: Arc<Notify>, stop: CancellationToken) {
        let mut tasks = JoinSet::new();
        let mut running = HashMap::new();
        let mut poll = interval(self.config.poll_interval);
        let mut maintenance = interval(self.config.maintenance_interval);
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        maintenance.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // An interval's first tick is immediate; the loop claims once before waiting.
        poll.tick().await;

        // Whether due jobs may be waiting. It stays set while every slot is busy, so a
        // wake-up that arrives then is acted on as soon as a run finishes.
        let mut maybe_due = true;

        loop {
            if maybe_due && tasks.len() < self.config.concurrency {
                let free = self.config.concurrency - tasks.len();
                let claimed = self.claim(free, &running).await;
                // A claim that filled every free slot may have left more behind.
                maybe_due = claimed.len() == free;
                for job in claimed {
                    let job_id = job.id;
                    let kind = job.kind.clone();
                    let span = (self.span)(&job);
                    let task = tasks.spawn(Arc::clone(&self).execute(job).instrument(span));
                    running.insert(task.id(), (job_id, kind));
                }
            }

            tokio::select! {
                () = stop.cancelled() => break,
                Some(joined) = tasks.join_next_with_id() => {
                    // A kind held to its share may have jobs waiting for the slot it left.
                    let kind = finished(joined, &mut running);
                    if kind.is_some_and(|kind| self.share(&kind).is_some()) {
                        maybe_due = true;
                    }
                }
                () = wake.notified() => maybe_due = true,
                _ = poll.tick() => maybe_due = true,
                _ = maintenance.tick() => {
                    self.maintain().await;
                    maybe_due = true;
                }
            }
        }

        self.drain(tasks, running).await;
    }

    /// Claims up to `free` due jobs: of the kinds that may take every slot first, then of
    /// each held to a share, as many as its share has room for beside those `running`.
    async fn claim(&self, free: usize, running: &Running) -> Vec<Job> {
        let shared: Vec<String> = self
            .kinds
            .iter()
            .filter(|kind| self.share(kind).is_none())
            .cloned()
            .collect();
        let mut claimed = self.claim_of(&shared, free).await;
        for kind in &self.kinds {
            let Some(share) = self.share(kind) else {
                continue;
            };
            let busy = running.values().filter(|(_, of)| of == kind).count();
            let room = share
                .saturating_sub(busy)
                .min(free.saturating_sub(claimed.len()));
            claimed.extend(self.claim_of(std::slice::from_ref(kind), room).await);
        }
        claimed
    }

    /// The share of the slots a kind's jobs may take, when it is held to one.
    fn share(&self, kind: &str) -> Option<usize> {
        self.handlers
            .get(kind)
            .and_then(|handler| handler.max_running())
    }

    async fn claim_of(&self, kinds: &[String], limit: usize) -> Vec<Job> {
        if kinds.is_empty() || limit == 0 {
            return Vec::new();
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        jobs::claim(&self.pool, &self.id, kinds, limit)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, "could not claim jobs");
                Vec::new()
            })
    }

    async fn execute(self: Arc<Self>, job: Job) {
        let job = Arc::new(job);
        let outcome = AssertUnwindSafe(self.wrapped(&job))
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| {
                Err(JobError::retry(format!(
                    "the handler panicked: {}",
                    panic_message(panic.as_ref())
                )))
            });
        self.record(&job, outcome).await;
    }

    /// The handler's run with its timeout, inside the hooks.
    fn wrapped(&self, job: &Arc<Job>) -> RunFuture {
        let handler = self.handlers.get(job.kind.as_str()).cloned();
        let timeout = self.config.run_timeout;
        let run_job = Arc::clone(job);
        let run: RunFuture = Box::pin(async move {
            let Some(handler) = handler else {
                return Err(JobError::permanent(format!(
                    "no handler for job kind {}",
                    run_job.kind
                )));
            };
            tokio::time::timeout(timeout, handler.run(&run_job))
                .await
                .unwrap_or_else(|_| {
                    Err(JobError::retry(format!(
                        "timed out after {}",
                        duration::format(timeout)
                    )))
                })
        });

        self.hooks
            .iter()
            .rev()
            .fold(run, |run, hook| hook.around(job, run))
    }

    async fn record(&self, job: &Job, outcome: Result<(), JobError>) {
        let lease = job.lease(&self.id);
        let keep = !self.unkept.contains(&job.kind);
        let recorded = match outcome {
            Ok(()) => {
                debug!("job done");
                if keep {
                    jobs::complete(&self.pool, lease).await
                } else {
                    jobs::delete(&self.pool, lease).await
                }
            }
            Err(error) if error.is_permanent() || job.attempt >= job.max_attempts => {
                error!(%error, attempts = job.attempt, "job failed for the last time");
                if keep {
                    jobs::mark_dead(&self.pool, lease, error.message()).await
                } else {
                    jobs::delete(&self.pool, lease).await
                }
            }
            Err(error) => {
                let attempt = u32::try_from(job.attempt).unwrap_or(1);
                let delay = self.config.backoff.delay(attempt, rand::random());
                warn!(%error, retry_in = ?delay, "job failed");
                jobs::retry(&self.pool, lease, delay, error.message()).await
            }
        };

        match recorded {
            Ok(true) => {}
            Ok(false) => warn!("job was taken back while it ran; its outcome was dropped"),
            Err(error) => {
                error!(%error, "could not record the job's outcome; it runs again once its claim expires");
            }
        }
    }

    async fn maintain(&self) {
        match jobs::reclaim_expired(&self.pool, self.config.lease_timeout).await {
            Ok(0) => {}
            Ok(count) => warn!(
                jobs = count,
                "took back jobs whose worker stopped responding"
            ),
            Err(error) => warn!(%error, "could not take back expired jobs"),
        }
        match jobs::delete_finished(&self.pool, self.config.retention).await {
            Ok(0) => {}
            Ok(count) => debug!(jobs = count, "deleted finished jobs"),
            Err(error) => warn!(%error, "could not delete finished jobs"),
        }
        // A job taken back from a dead worker on its last attempt is marked dead, with no
        // run left to delete it as it ends.
        if !self.unkept.is_empty() {
            match jobs::delete_finished_of(&self.pool, &self.unkept).await {
                Ok(0) => {}
                Ok(count) => debug!(jobs = count, "deleted finished jobs that are not kept"),
                Err(error) => warn!(%error, "could not delete finished jobs that are not kept"),
            }
        }
    }

    async fn drain(&self, mut tasks: JoinSet<()>, mut running: Running) {
        if tasks.is_empty() {
            return;
        }
        info!(jobs = tasks.len(), "waiting for running jobs to finish");

        let deadline = sleep(self.config.stop_timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                joined = tasks.join_next_with_id() => match joined {
                    Some(joined) => {
                        finished(joined, &mut running);
                    }
                    None => return,
                },
                () = &mut deadline => break,
            }
        }

        tasks.abort_all();
        while let Some(joined) = tasks.join_next_with_id().await {
            // A task can finish between the deadline and its cancellation.
            if let Ok((task, ())) = joined {
                running.remove(&task);
            }
        }

        let cancelled: Vec<Uuid> = running.into_values().map(|(id, _)| id).collect();
        match jobs::release(&self.pool, &self.id, &cancelled).await {
            Ok(count) => warn!(
                jobs = count,
                "cancelled jobs that outlasted the stop timeout"
            ),
            Err(error) => {
                error!(%error, "could not queue cancelled jobs again; they run once their claim expires");
            }
        }
    }
}

/// The jobs running, by their task: each job's id and kind.
type Running = HashMap<Id, (Uuid, String)>;

/// Forgets a job whose task ended, answering its kind.
fn finished(joined: Result<(Id, ()), JoinError>, running: &mut Running) -> Option<String> {
    match joined {
        Ok((task, ())) => running.remove(&task).map(|(_, kind)| kind),
        Err(error) => {
            let job = running.remove(&error.id());
            error!(%error, ?job, "job task failed; the job runs again once its claim expires");
            job.map(|(_, kind)| kind)
        }
    }
}

pub(super) fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_refuses_a_run_timeout_past_the_lease() {
        let config = WorkerConfig {
            run_timeout: Duration::from_secs(60),
            lease_timeout: Duration::from_secs(60),
            ..WorkerConfig::default()
        };

        assert!(config.validate().is_err());
        assert_eq!(WorkerConfig::default().validate(), Ok(()));
    }

    #[test]
    fn panic_messages_are_read_from_either_string_type() {
        let literal: Box<dyn Any + Send> = Box::new("boom");
        let formatted: Box<dyn Any + Send> = Box::new(String::from("bang"));
        let other: Box<dyn Any + Send> = Box::new(7);

        assert_eq!(panic_message(literal.as_ref()), "boom");
        assert_eq!(panic_message(formatted.as_ref()), "bang");
        assert_eq!(panic_message(other.as_ref()), "no message");
    }
}
