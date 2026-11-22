//! Work that runs on an interval for as long as the server does, such as the hourly
//! cleanups.
//!
//! A periodic task rather than a job: it needs no retries, since the next run catches up,
//! and every server process may run it, as its deletes do not conflict.

use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

/// A running periodic task: once at start, then every interval until stopped.
pub struct Periodic {
    name: &'static str,
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Periodic {
    /// Runs `run` now and then every `interval`. A run that fails is logged, naming the task
    /// `name`, and the next run tries again.
    pub fn spawn<F, Fut, E>(name: &'static str, interval: Duration, mut run: F) -> Self
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send,
        E: Display,
    {
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(interval);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = stopped.cancelled() => return,
                    _ = ticks.tick() => {
                        if let Err(error) = run().await {
                            warn!(%error, task = name, "a periodic task failed; the next run tries again");
                        }
                    }
                }
            }
        });
        Self { name, stop, task }
    }

    /// Stops, waiting for a run in progress, so the pool can close after it.
    pub async fn stop(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            error!(%error, task = self.name, "a periodic task failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn runs_at_once_then_every_interval_until_stopped() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let task = Periodic::spawn("test", Duration::from_secs(60), move || {
            let runs = Arc::clone(&counted);
            async move {
                let run = runs.fetch_add(1, Ordering::SeqCst);
                if run == 1 {
                    Err("the second run fails")
                } else {
                    Ok(())
                }
            }
        });

        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            3,
            "a failed run does not stop it"
        );
        task.stop().await;
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
    }
}
