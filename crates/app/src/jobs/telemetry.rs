//! Traces and RED metrics of job runs.
//!
//! [`run_span`] makes each run's span for [`Worker::span`](super::Worker::span), continuing
//! the trace stored in the job's row. [`RedHook`] is a [`RunHook`] that records how each run
//! ended, on that span and in the RED metrics of [`SUBSYSTEM`], with the job's kind as the
//! operation:
//!
//! | The run | Span status | RED `status` |
//! |---|---|---|
//! | succeeded | unset | `ok` |
//! | failed and will be retried, a timeout included | unset, with the error recorded as expected | `ok` |
//! | failed permanently, or on its last attempt, so the job is dead | error | `error` |
//! | panicked | error | `error` |
//! | was cancelled before it ended, by a worker that stopped | error | `error` |
//!
//! A failure that a retry may fix is part of normal operation, as a 4xx answer is for HTTP:
//! error rates and failed traces then show the jobs that were given up on.

use std::panic::{AssertUnwindSafe, resume_unwind};

use futures_util::FutureExt as _;
use neon_observability::errors::{mark_expected, mark_unexpected};
use neon_observability::propagation::set_parent_from_traceparent;
use neon_observability::red::{Red, RedGuard};
use neon_store::jobs::Job;
use tracing::{Span, info_span};

use super::handler::{RunFuture, RunHook};
use super::worker::panic_message;

/// RED subsystem of job runs.
pub const SUBSYSTEM: &str = "jobs";

/// Span status description of a run dropped before it ended.
const CANCELLED: &str = "the run was cancelled before it ended";

/// The span a run is recorded in, named `job <kind>`. Its parent is the `traceparent` stored
/// with the job, so the run joins the trace of the request that queued it, minutes or hours
/// later. A job without one, or with one that does not parse, starts a new trace.
pub fn run_span(job: &Job) -> Span {
    let span = info_span!(
        parent: None,
        "job",
        otel.name = %format!("job {}", job.kind),
        otel.kind = "consumer",
        job.kind = %job.kind,
        job.id = %job.id,
        job.attempt = job.attempt,
    );
    if let Some(traceparent) = &job.traceparent {
        set_parent_from_traceparent(&span, traceparent);
    }
    span
}

/// Records how every run ended; see the [module docs](self).
#[derive(Debug, Clone)]
pub struct RedHook {
    red: Red,
}

impl RedHook {
    /// A hook recording under [`SUBSYSTEM`]. Create it after telemetry is initialized, like
    /// any [`Red`].
    pub fn new() -> Self {
        Self {
            red: Red::new(SUBSYSTEM),
        }
    }
}

impl Default for RedHook {
    fn default() -> Self {
        Self::new()
    }
}

impl RunHook for RedHook {
    fn around(&self, job: &Job, run: RunFuture) -> RunFuture {
        let red = self.red.clone();
        let kind = job.kind.clone();
        let last_attempt = job.attempt >= job.max_attempts;
        Box::pin(async move {
            let mut outcome = Outcome {
                span: Span::current(),
                guard: Some(red.start(kind)),
            };
            match AssertUnwindSafe(run).catch_unwind().await {
                Ok(Ok(())) => {
                    outcome.finish(true);
                    Ok(())
                }
                Ok(Err(error)) if error.is_permanent() || last_attempt => {
                    mark_unexpected(&outcome.span, error.message());
                    outcome.finish(false);
                    Err(error)
                }
                Ok(Err(error)) => {
                    mark_expected(&outcome.span, error.message());
                    outcome.finish(true);
                    Err(error)
                }
                Err(panic) => {
                    mark_unexpected(
                        &outcome.span,
                        format!("the handler panicked: {}", panic_message(panic.as_ref())),
                    );
                    outcome.finish(false);
                    // The worker catches it again and records the failed attempt.
                    resume_unwind(panic)
                }
            }
        })
    }
}

/// A run's span and RED guard. Dropped before the run ended, it fails the span, and the
/// guard counts the run as an error.
struct Outcome {
    span: Span,
    guard: Option<RedGuard>,
}

impl Outcome {
    fn finish(&mut self, ok: bool) {
        if let Some(guard) = self.guard.take() {
            guard.finish(ok);
        }
    }
}

impl Drop for Outcome {
    fn drop(&mut self) {
        if self.guard.is_some() {
            mark_unexpected(&self.span, CANCELLED);
        }
    }
}
