//! Background jobs.
//!
//! Queue a job with [`enqueue`] on the transaction of the change that caused it; it keeps
//! the current trace context. A [`Worker`] claims due jobs, runs each with the [`Handler`]
//! registered for its kind, retries failures with backoff, and marks a job dead once it
//! runs out of attempts. With [`run_span`] and [`RedHook`], each run continues the trace it
//! was queued in and is measured.

mod handler;
mod telemetry;
mod worker;

pub use handler::{Handler, JobError, RunFuture, RunHook};
pub use neon_store::jobs::{Job, NewJob, enqueue};
pub use telemetry::{RedHook, SUBSYSTEM, run_span};
pub use worker::{LISTEN_CONNECTIONS, StartError, Worker, WorkerConfig, WorkerHandle};
