//! Logs, metrics, traces and profiles, the internal admin listener, and ordered shutdown.
//!
//! `tracing` and `metrics` already are vendor-neutral facades, so application code only uses
//! their macros. This crate installs the backends once per process with [`init`] and adds the
//! pieces the facades leave out:
//!
//! - [`red`]: rate, errors and duration of a subsystem's operations, behind a guard.
//! - [`http`]: a tower layer that traces and measures every axum request.
//! - [`propagation`]: W3C trace context through headers, job rows and webhooks.
//! - [`errors`]: expected errors that leave the span status unset, and real failures.
//! - [`profiling`]: CPU and heap profiles, pulled or pushed.
//! - [`admin`]: the internal listener with `/metrics`, `/livez`, `/readyz` and profiles.
//! - [`shutdown`]: flushes and stops that run in reverse order on SIGTERM.
//!
//! ```no_run
//! use neon_observability::admin::{self, Readiness};
//! use neon_observability::red::Buckets;
//! use neon_observability::{Config, ServiceInfo, http};
//!
//! async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
//!     let service = ServiceInfo::new("neon", env!("CARGO_PKG_VERSION"));
//!     let telemetry = neon_observability::init(
//!         &config,
//!         &service,
//!         &[(http::SUBSYSTEM, Buckets::Medium), ("jobs", Buckets::Long)],
//!     )?;
//!     admin::spawn(&config.admin, &telemetry, Readiness::always()).await?;
//!
//!     // Start the app and register its stops with `telemetry.shutdown().on_shutdown(..)`.
//!
//!     neon_observability::shutdown::shutdown_signal().await;
//!     telemetry.shutdown().run().await?;
//!     Ok(())
//! }
//! ```
//!
//! # Footguns this crate takes care of
//!
//! - **No trace ids without export.** An OpenTelemetry tracer provider is always installed, so
//!   every span has a trace id that logs, job rows and webhooks carry even while export is off.
//! - **`traceparent` never written.** OpenTelemetry's default propagator does nothing; [`init`]
//!   installs W3C TraceContext plus Baggage.
//! - **Logs without trace ids.** JSON log lines inside a span carry top-level `trace_id` and
//!   `span_id` taken from the OpenTelemetry context, so they match the exported trace.
//! - **A quiet log level hiding spans.** The level filters log output only. Spans at `info`
//!   and above always reach the tracer, and a more verbose level lowers the traced level with
//!   it, so lines logged inside `debug` spans carry ids as well.
//! - **Expected errors failing spans.** Only `ERROR` events and [`errors::record_unexpected`]
//!   set the span status to error. A `warn!` with an `error` field, a 4xx response or
//!   [`errors::record_expected`] leave it unset.
//! - **Unrecorded outcomes.** A [`red::RedGuard`] dropped without a result, by `?`, an early
//!   return or a cancelled future, counts as an error instead of disappearing.
//! - **Summaries instead of histograms.** Every duration histogram gets buckets, the
//!   subsystem's preset or a default, so they aggregate across instances. Histograms of
//!   sizes, whose names end in `_bytes`, get buckets of bytes, 64 to 16 MiB.
//! - **Unbounded label values.** HTTP metrics use route templates, a fixed `unmatched` route
//!   and `_OTHER` for unknown methods, never raw paths.
//! - **Secrets in URLs.** Request spans, and so every log line inside a request, record the
//!   route template as `url.path` and never the query string, because paths carry webhook
//!   tokens and invite codes.
//! - **Forged trace context.** The public port does not continue a caller's `traceparent` or
//!   keep its `baggage` unless `tracing.trust_incoming` says so: a stranger's sampled flag
//!   would override `sample_ratio`, and its baggage would travel on to job rows and webhooks.
//! - **Cancelled requests that look fine.** A request dropped before its response fails its
//!   span and counts as a RED error.
//! - **Two CPU profilers at once.** Profiling is off, pull or push, never two of them.
//! - **Empty heap profiles.** Heap profiles answer `501` and are not pushed unless the binary
//!   runs on `profiling::Jemalloc`, the Linux allocator that reports it is in use; see
//!   [`profiling`].
//! - **Lost last words.** Telemetry flushes are registered first and therefore run last, after
//!   every component registered later had the chance to log its own shutdown.
//! - **Public diagnostics.** The admin listener defaults to loopback and warns when bound
//!   elsewhere.

pub mod admin;
pub mod config;
pub mod errors;
pub mod http;
mod logging;
pub mod profiling;
mod prometheus;
pub mod propagation;
pub mod red;
pub mod shutdown;
#[cfg(test)]
mod testing;
mod traces;

use std::convert::Infallible;
use std::io::IsTerminal as _;
use std::sync::Arc;

use metrics_exporter_prometheus::{PrometheusHandle, PrometheusRecorder};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt;

pub use crate::config::Config;
use crate::config::{InvalidConfig, ProfilingMode};
use crate::profiling::{Pull, Push};
use crate::red::Buckets;
use crate::shutdown::{BoxError, Shutdown};

/// The service that emits the telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    /// `service.name` on spans and the Pyroscope application name.
    pub name: String,
    /// `service.version` on spans.
    pub version: String,
}

impl ServiceInfo {
    /// Describes the service `name` at `version`.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// The installed telemetry: the shutdown registry and what the admin listener serves.
#[derive(Debug, Clone)]
pub struct Telemetry {
    shutdown: Shutdown,
    metrics: Option<PrometheusHandle>,
    pull: Option<Arc<Pull>>,
}

impl Telemetry {
    /// Telemetry that installs nothing, for tests and tools. It has its own empty shutdown
    /// registry, serves no metrics and no profiles, and can be created any number of times.
    #[must_use]
    pub fn noop() -> Self {
        Self {
            shutdown: Shutdown::new(),
            metrics: None,
            pull: None,
        }
    }

    /// The registry the telemetry flushes are registered with. Register application
    /// components here so they stop before the telemetry they log and trace through.
    #[must_use]
    pub fn shutdown(&self) -> &Shutdown {
        &self.shutdown
    }
}

/// Why [`init`] failed.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The configuration has errors.
    #[error(transparent)]
    Config(#[from] InvalidConfig),
    /// A RED subsystem name is not a valid metric name fragment.
    #[error(
        "RED subsystem {0:?} must start with a lowercase letter and contain only lowercase \
         letters, digits and underscores"
    )]
    Subsystem(String),
    /// [`init`] was called outside a Tokio runtime.
    #[error("telemetry must be initialized inside a Tokio runtime")]
    NoRuntime,
    /// The log level directive does not parse.
    #[error("invalid log level: {0}")]
    LogLevel(#[from] tracing_subscriber::filter::ParseError),
    /// The OTLP exporter cannot be built.
    #[error("cannot build the OTLP span exporter: {0}")]
    Exporter(#[from] opentelemetry_otlp::ExporterBuildError),
    /// The Prometheus recorder cannot be built.
    #[error("cannot build the Prometheus recorder: {0}")]
    Metrics(#[from] metrics_exporter_prometheus::BuildError),
    /// The Pyroscope agent cannot start.
    #[error("cannot start the Pyroscope agent: {0}")]
    Profiler(#[from] pyroscope::PyroscopeError),
    /// A global subscriber or metrics recorder is already installed in this process.
    #[error("telemetry is already initialized in this process")]
    AlreadyInitialized,
}

/// Installs logs, metrics, traces and profiling for the whole process. Call it once, first
/// thing in `main`, inside the Tokio runtime.
///
/// `subsystems` lists each RED subsystem with its histogram bucket preset, such as
/// `(http::SUBSYSTEM, Buckets::Medium)`. Subsystems left out still get histograms, with the
/// [`Buckets::Medium`] preset.
///
/// The returned [`Telemetry`] owns a [`Shutdown`] registry that already holds, in the order
/// they will run last: the profiler stop, the tracer flush and the log flush.
///
/// # Errors
///
/// Returns [`InitError`] when the configuration is invalid, a subsystem name is malformed,
/// there is no Tokio runtime, a backend cannot be built, or telemetry is already installed.
pub fn init(
    config: &Config,
    service: &ServiceInfo,
    subsystems: &[(&str, Buckets)],
) -> Result<Telemetry, InitError> {
    let warnings = config.validate()?;
    if let Some((invalid, _)) = subsystems
        .iter()
        .find(|(subsystem, _)| !red::is_valid_subsystem(subsystem))
    {
        return Err(InitError::Subsystem((*invalid).to_owned()));
    }
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| InitError::NoRuntime)?;

    // Everything fallible is built before any global is installed.
    let provider = traces::provider(&config.tracing, service)?;
    let (writer, log_guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .thread_name("neon-log-writer")
        .finish(std::io::stdout());
    let log_filter = logging::filter(&config.logging)?;
    let spans = traces::layer(&provider, service, &log_filter);
    let logs = logging::layer(
        config.logging.format,
        log_filter,
        writer,
        std::io::stdout().is_terminal(),
    );
    let recorder = config
        .metrics
        .enabled
        .then(|| prometheus::recorder(subsystems))
        .transpose()?;
    let push = match config.profiling.mode {
        ProfilingMode::Push => Some(Push::start(&config.profiling, service)?),
        ProfilingMode::Off | ProfilingMode::Pull => None,
    };

    let subscriber = tracing_subscriber::registry().with(spans).with(logs);
    let metrics = match install(recorder, subscriber) {
        Ok(metrics) => metrics,
        Err(error) => {
            abandon(push, &provider);
            return Err(error);
        }
    };
    opentelemetry::global::set_tracer_provider(provider.clone());
    propagation::install_propagator();

    let shutdown = Shutdown::new();
    shutdown.on_shutdown("logs", || {
        // Dropping the guard blocks until the writer thread has written every buffered line.
        blocking(move || {
            drop(log_guard);
            Ok::<(), Infallible>(())
        })
    });
    shutdown.on_shutdown("traces", || blocking(move || provider.shutdown()));
    if let Some(push) = push {
        let timeout = push.stop_timeout(crate::shutdown::DEFAULT_HOOK_TIMEOUT);
        shutdown.on_shutdown_with_timeout("profiler", timeout, || blocking(move || push.stop()));
    }
    if let Some(handle) = &metrics {
        for (subsystem, _) in subsystems {
            red::describe(subsystem);
        }
        runtime.spawn(upkeep(handle.clone(), shutdown.clone()));
    }
    let pull = (config.profiling.mode == ProfilingMode::Pull)
        .then(|| Arc::new(Pull::new(&config.profiling, shutdown.clone())));

    for warning in &warnings {
        tracing::warn!(setting = warning.field, "{}", warning.message);
    }
    tracing::info!(
        service = %service.name,
        version = %service.version,
        metrics = config.metrics.enabled,
        trace_export = config.tracing.enabled,
        profiling = ?config.profiling.mode,
        "telemetry initialized"
    );
    Ok(Telemetry {
        shutdown,
        metrics,
        pull,
    })
}

/// Installs the global metrics recorder, then the global subscriber, which also routes `log`
/// records into `tracing`.
fn install(
    recorder: Option<PrometheusRecorder>,
    subscriber: impl SubscriberInitExt,
) -> Result<Option<PrometheusHandle>, InitError> {
    let handle = recorder
        .map(|recorder| {
            let handle = recorder.handle();
            metrics::set_global_recorder(recorder).map(|()| handle)
        })
        .transpose()
        .map_err(|_| InitError::AlreadyInitialized)?;
    subscriber
        .try_init()
        .map_err(|_| InitError::AlreadyInitialized)?;
    Ok(handle)
}

/// Stops what was started before a global could not be installed.
fn abandon(push: Option<Push>, provider: &SdkTracerProvider) {
    if let Some(push) = push
        && let Err(error) = push.stop()
    {
        tracing::warn!(%error, "cannot stop the Pyroscope agents");
    }
    if let Err(error) = provider.shutdown() {
        tracing::warn!(%error, "cannot shut the tracer provider down");
    }
}

/// Runs a flush that blocks on a thread meant for blocking, away from the async workers.
async fn blocking<E>(flush: impl FnOnce() -> Result<(), E> + Send + 'static) -> Result<(), BoxError>
where
    E: Into<BoxError> + Send + 'static,
{
    tokio::task::spawn_blocking(flush)
        .await?
        .map_err(Into::into)
}

/// Drains histogram samples periodically, which the exporter otherwise only does on scrape.
async fn upkeep(handle: PrometheusHandle, shutdown: Shutdown) {
    let mut ticks = tokio::time::interval(prometheus::UPKEEP_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.started() => return,
            _ = ticks.tick() => handle.run_upkeep(),
        }
    }
}
