//! Background jobs.
//!
//! Queue a job with [`enqueue`] on the transaction of the change that caused it. A
//! [`Worker`] claims due jobs, runs each with the [`Handler`] registered for its kind,
//! retries failures with backoff, and marks a job dead once it runs out of attempts.

mod handler;
mod worker;

pub use handler::{Handler, JobError, RunFuture, RunHook};
pub use neon_store::jobs::{Job, NewJob, enqueue};
pub use worker::{LISTEN_CONNECTIONS, StartError, Worker, WorkerConfig, WorkerHandle};
