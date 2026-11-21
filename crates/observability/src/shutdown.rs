//! Ordered shutdown: flushes are registered, never discovered.
//!
//! Hooks run in reverse registration order, like `defer`. Telemetry registers its flushes
//! first, so they run last and the logger stays alive to report failures from everything
//! registered after it.

use std::any::Any;
use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::watch;

/// The error a hook may return.
pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

type HookFuture = Pin<Box<dyn Future<Output = Result<(), BoxError>> + Send>>;
type HookFn = Box<dyn FnOnce() -> HookFuture + Send>;

/// How long one hook may run before it is abandoned and the next one starts.
pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// A cloneable registry of shutdown hooks. Clones share the same hooks.
#[derive(Clone)]
pub struct Shutdown {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    started: watch::Sender<bool>,
    hook_timeout: Duration,
}

#[derive(Default)]
struct State {
    hooks: Vec<Hook>,
    ran: bool,
}

struct Hook {
    name: Cow<'static, str>,
    run: HookFn,
}

impl Shutdown {
    /// A registry whose hooks each get [`DEFAULT_HOOK_TIMEOUT`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_hook_timeout(DEFAULT_HOOK_TIMEOUT)
    }

    /// A registry whose hooks each get `timeout`.
    #[must_use]
    pub fn with_hook_timeout(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                started: watch::Sender::new(false),
                hook_timeout: timeout,
            }),
        }
    }

    /// Registers a hook that [`run`](Self::run) awaits before the hooks registered earlier.
    ///
    /// A hook registered after shutdown has begun would never run, so it is dropped with a
    /// warning instead of being silently lost.
    pub fn on_shutdown<F, Fut, E>(&self, name: impl Into<Cow<'static, str>>, hook: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<BoxError>,
    {
        let name = name.into();
        let mut state = self.lock();
        if state.ran {
            drop(state);
            tracing::warn!(hook = %name, "shutdown already started, hook will not run");
            return;
        }
        state.hooks.push(Hook {
            name,
            run: Box::new(move || Box::pin(async move { hook().await.map_err(Into::into) })),
        });
    }

    /// Runs every hook in reverse registration order and joins their failures.
    ///
    /// Every hook runs even when an earlier one fails, panics or exceeds its timeout. Each
    /// failure is also logged as it happens, because the log writer is flushed by the last
    /// hook and anything logged after `run` returns may be lost. Calling `run` again, or
    /// concurrently, returns `Ok` without doing anything.
    ///
    /// Must be called from within a Tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns [`ShutdownError`] naming every hook that failed.
    pub async fn run(&self) -> Result<(), ShutdownError> {
        let hooks = {
            let mut state = self.lock();
            if state.ran {
                return Ok(());
            }
            state.ran = true;
            std::mem::take(&mut state.hooks)
        };
        self.inner.started.send_replace(true);

        let mut failures = Vec::new();
        for hook in hooks.into_iter().rev() {
            tracing::debug!(hook = %hook.name, "running shutdown hook");
            if let Err(cause) = self.run_hook(hook.run).await {
                tracing::error!(hook = %hook.name, error = %cause, "shutdown hook failed");
                failures.push(HookFailure {
                    name: hook.name,
                    cause,
                });
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(ShutdownError { failures })
        }
    }

    async fn run_hook(&self, run: HookFn) -> Result<(), HookError> {
        let task = tokio::spawn(run());
        let abort = task.abort_handle();
        match tokio::time::timeout(self.inner.hook_timeout, task).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => Err(HookError::Failed(error)),
            Ok(Err(join)) if join.is_panic() => Err(HookError::Panicked(panic_message(
                join.into_panic().as_ref(),
            ))),
            Ok(Err(_)) => Err(HookError::Cancelled),
            Err(_) => {
                abort.abort();
                Err(HookError::TimedOut(self.inner.hook_timeout))
            }
        }
    }

    /// Whether [`run`](Self::run) has been called.
    #[must_use]
    pub fn is_started(&self) -> bool {
        *self.inner.started.borrow()
    }

    /// Resolves once [`run`](Self::run) has been called, so long-running work such as a CPU
    /// profile can stop early instead of holding up its hook.
    pub async fn started(&self) {
        let mut started = self.inner.started.subscribe();
        // The sender lives in `self`, so the channel cannot close while this is awaited.
        let _ = started.wait_for(|started| *started).await;
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock cannot leave the hook list half-written.
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Shutdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        f.debug_struct("Shutdown")
            .field(
                "hooks",
                &state
                    .hooks
                    .iter()
                    .map(|hook| &hook.name)
                    .collect::<Vec<_>>(),
            )
            .field("ran", &state.ran)
            .field("hook_timeout", &self.inner.hook_timeout)
            .finish()
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

/// Every hook that failed during [`Shutdown::run`], in the order they ran.
#[derive(Debug)]
pub struct ShutdownError {
    /// The failed hooks.
    pub failures: Vec<HookFailure>,
}

impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("shutdown hooks failed: ")?;
        for (index, failure) in self.failures.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{failure}")?;
        }
        Ok(())
    }
}

impl Error for ShutdownError {}

/// One hook that failed.
#[derive(Debug)]
pub struct HookFailure {
    /// The name the hook was registered under.
    pub name: Cow<'static, str>,
    /// Why it failed.
    pub cause: HookError,
}

impl fmt::Display for HookFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.cause)
    }
}

/// Why a hook failed.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// The hook returned an error.
    #[error(transparent)]
    Failed(BoxError),
    /// The hook ran past the per-hook timeout and was abandoned.
    #[error("timed out after {0:?}")]
    TimedOut(Duration),
    /// The hook panicked.
    #[error("panicked: {0}")]
    Panicked(String),
    /// The runtime cancelled the hook, usually because it is shutting down itself.
    #[error("cancelled")]
    Cancelled,
}

/// Resolves on SIGINT or SIGTERM, for `axum::serve(..).with_graceful_shutdown(..)`.
///
/// If a handler cannot be installed the error is logged and the other signal still works.
pub async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "cannot listen for SIGINT");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                terminate.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("received SIGINT, shutting down"),
        () = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    fn recorder() -> (Arc<Mutex<Vec<&'static str>>>, Shutdown) {
        (Arc::new(Mutex::new(Vec::new())), Shutdown::new())
    }

    fn register(shutdown: &Shutdown, log: &Arc<Mutex<Vec<&'static str>>>, name: &'static str) {
        let log = Arc::clone(log);
        shutdown.on_shutdown(name, move || async move {
            log.lock().unwrap().push(name);
            Ok::<_, io::Error>(())
        });
    }

    fn explode() -> Result<(), io::Error> {
        panic!("boom")
    }

    #[tokio::test]
    async fn runs_hooks_in_reverse_registration_order() {
        let (log, shutdown) = recorder();
        for name in ["logs", "traces", "database", "http"] {
            register(&shutdown, &log, name);
        }

        shutdown.run().await.unwrap();

        assert_eq!(*log.lock().unwrap(), ["http", "database", "traces", "logs"]);
    }

    #[tokio::test]
    async fn runs_every_hook_and_joins_failures() {
        let (log, shutdown) = recorder();
        register(&shutdown, &log, "first");
        shutdown.on_shutdown("broken", || async {
            Err::<(), _>(io::Error::other("disk full"))
        });
        shutdown.on_shutdown("panicking", || async { explode() });
        register(&shutdown, &log, "last");

        let error = shutdown.run().await.unwrap_err();

        assert_eq!(*log.lock().unwrap(), ["last", "first"]);
        let names: Vec<_> = error.failures.iter().map(|f| f.name.as_ref()).collect();
        assert_eq!(names, ["panicking", "broken"]);
        assert_eq!(
            error.to_string(),
            "shutdown hooks failed: panicking: panicked: boom; broken: disk full"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn abandons_a_hook_after_its_timeout_and_continues() {
        let (log, _) = recorder();
        let shutdown = Shutdown::with_hook_timeout(Duration::from_secs(2));
        register(&shutdown, &log, "after");
        shutdown.on_shutdown("stuck", || async {
            std::future::pending::<()>().await;
            Ok::<(), io::Error>(())
        });

        let error = shutdown.run().await.unwrap_err();

        assert_eq!(*log.lock().unwrap(), ["after"]);
        assert_eq!(error.failures.len(), 1);
        assert!(matches!(
            error.failures[0].cause,
            HookError::TimedOut(timeout) if timeout == Duration::from_secs(2)
        ));
        assert_eq!(
            error.to_string(),
            "shutdown hooks failed: stuck: timed out after 2s"
        );
    }

    #[tokio::test]
    async fn second_run_is_a_no_op() {
        let (log, shutdown) = recorder();
        register(&shutdown, &log, "once");

        shutdown.run().await.unwrap();
        shutdown.run().await.unwrap();

        assert_eq!(*log.lock().unwrap(), ["once"]);
    }

    #[tokio::test]
    async fn clones_share_hooks_and_late_hooks_are_dropped() {
        let (log, shutdown) = recorder();
        let clone = shutdown.clone();
        register(&clone, &log, "from clone");
        assert!(!shutdown.is_started());

        shutdown.run().await.unwrap();
        register(&clone, &log, "too late");
        shutdown.run().await.unwrap();

        assert!(clone.is_started());
        assert_eq!(*log.lock().unwrap(), ["from clone"]);
    }

    #[tokio::test]
    async fn started_resolves_when_run_begins() {
        let shutdown = Shutdown::new();
        let waiter = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.started().await }
        });

        shutdown.run().await.unwrap();

        waiter.await.unwrap();
        shutdown.started().await;
    }
}
