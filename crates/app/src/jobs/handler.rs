use std::error::Error;
use std::fmt;

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use neon_store::jobs::Job;

/// Runs the jobs of one kind.
///
/// A job may run more than once: after a failure, and when its worker died before it
/// could record the outcome. Handlers must be safe to repeat.
#[async_trait]
pub trait Handler: Send + Sync + 'static {
    /// The kind of job this handler runs, like `mail.send`.
    fn kind(&self) -> &'static str;

    /// Whether its jobs are kept once they are done or dead, for the worker's retention,
    /// so they can be inspected. Kinds whose payload must not outlive the run, such as
    /// mail carrying one-time links, say no: their jobs are deleted as they finish.
    fn keeps_finished(&self) -> bool {
        true
    }

    /// The most of its jobs one worker runs at once, out of its concurrency. A kind that may
    /// have many jobs, such as webhook deliveries, takes a share of the slots so the others
    /// always find one free. `None`, as by default, lets it take every slot.
    fn max_running(&self) -> Option<usize> {
        None
    }

    async fn run(&self, job: &Job) -> Result<(), JobError>;
}

/// Why a run failed.
///
/// Any error converts into a retryable one, so handlers can use `?`.
#[derive(Debug)]
pub struct JobError {
    message: String,
    permanent: bool,
}

impl JobError {
    /// A failure that may pass, like a mail server being down: the job runs again after a
    /// backoff, until it runs out of attempts.
    pub fn retry(error: impl fmt::Display) -> Self {
        Self {
            message: error.to_string(),
            permanent: false,
        }
    }

    /// A failure retrying cannot fix, like a payload that does not parse: the job is
    /// marked dead at once.
    pub fn permanent(error: impl fmt::Display) -> Self {
        Self {
            message: error.to_string(),
            permanent: true,
        }
    }

    pub fn is_permanent(&self) -> bool {
        self.permanent
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl<E: Error + Send + Sync + 'static> From<E> for JobError {
    fn from(error: E) -> Self {
        let mut message = error.to_string();
        let mut source = error.source();
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        Self::retry(message)
    }
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// A job run, boxed so [`RunHook`]s can wrap it.
pub type RunFuture = BoxFuture<'static, Result<(), JobError>>;

/// Wraps every job run, for concerns that apply to all kinds, such as metrics per kind.
///
/// The run includes the handler's timeout. A hook that drops its future without
/// awaiting it cancels the run, which then counts as a failure.
pub trait RunHook: Send + Sync + 'static {
    fn around(&self, job: &Job, run: RunFuture) -> RunFuture;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("sending mail")]
    struct Send(#[source] std::io::Error);

    #[test]
    fn errors_convert_with_their_causes() {
        let error = JobError::from(Send(std::io::Error::other("connection refused")));

        assert_eq!(error.message(), "sending mail: connection refused");
        assert!(!error.is_permanent());
        assert!(JobError::permanent("bad payload").is_permanent());
    }
}
