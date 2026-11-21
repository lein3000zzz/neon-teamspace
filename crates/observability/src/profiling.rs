//! CPU and heap profiles, pulled from the admin listener or pushed to Pyroscope.
//!
//! The modes exclude each other. Both sample the CPU with the pprof-rs copy inside the
//! Pyroscope agent, which keeps one process-wide profiler on one `SIGPROF` timer: a pull
//! profile while the agent runs would stop the agent's sampling, and the reverse.
//! [`ProfilingMode`](crate::config::ProfilingMode) holds exactly one.
//!
//! The standalone `pprof` crate is deliberately not used next to it: both export the same
//! `perf_signal_handler` symbol, so linking the two sends one profiler's samples into the
//! other's state.
//!
//! # Pull
//!
//! The admin listener serves:
//!
//! - `GET /debug/pprof/profile?seconds=N`: a CPU profile over `N` seconds (default 30, at
//!   most 60) as uncompressed pprof protobuf. One runs at a time; a second request gets
//!   `409 Conflict`. Shutdown cuts a running profile short instead of waiting for it.
//! - `GET /debug/pprof/heap`: the sampled live heap from jemalloc as gzipped pprof protobuf.
//!   It answers `501 Not Implemented` with the reason when heap profiles are unavailable.
//!
//! Both open with `go tool pprof http://127.0.0.1:9090/debug/pprof/profile` or any pprof viewer.
//!
//! # Push
//!
//! A Pyroscope agent per profile type uploads to `profiling.push.server_address` every
//! `upload_interval_seconds`, under the service name with the configured tags. The agents stop
//! during shutdown, after sending their last batch.
//!
//! # Heap profiles
//!
//! Heap profiles come from jemalloc's sampling profiler and only work on Linux. Two things are
//! needed, and missing either one turns the heap endpoint into a `501` and the heap push agent
//! into a warning:
//!
//! 1. The binary uses jemalloc as its global allocator, through the re-export here, so it needs
//!    no allocator dependency of its own:
//!
//!    ```no_run
//!    #[cfg(target_os = "linux")]
//!    #[global_allocator]
//!    static ALLOCATOR: neon_observability::profiling::Jemalloc =
//!        neon_observability::profiling::Jemalloc;
//!    # fn main() {}
//!    ```
//!
//! 2. The process starts with profiling switched on in jemalloc, which it reads once at
//!    startup. In the Dockerfile:
//!
//!    ```dockerfile
//!    ENV _RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19
//!    ```
//!
//!    [`MALLOC_CONF_ENV`] and [`MALLOC_CONF`] hold the same name and value. `lg_prof_sample:19`
//!    samples one allocation per 512 KiB on average, which keeps the overhead small. Other
//!    allocators ignore the variable.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::StatusCode;
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, HeaderValue};
use prost::Message as _;
use pyroscope::backend::{
    Backend as _, BackendConfig, BackendImpl, BackendUninitialized, Pprof, PprofConfig, ReportData,
};
use pyroscope::pyroscope::{PyroscopeAgentBuilder, PyroscopeAgentRunning};
use pyroscope::{PyroscopeAgent, PyroscopeError};
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::ServiceInfo;
use crate::config::{Profile, ProfilingConfig};
use crate::shutdown::Shutdown;

#[cfg(target_os = "linux")]
pub use tikv_jemallocator::Jemalloc;

/// The variable jemalloc reads its options from in this build, where its symbols carry the
/// `_rjem_` prefix.
pub const MALLOC_CONF_ENV: &str = "_RJEM_MALLOC_CONF";

/// jemalloc options that turn heap profiling on with a sample every 512 KiB.
pub const MALLOC_CONF: &str = "prof:true,prof_active:true,lg_prof_sample:19";

/// Length of a CPU profile when the request does not say.
pub const DEFAULT_CPU_SECONDS: u64 = 30;
/// Longest CPU profile a request may ask for, so one request cannot hold the profiler forever.
pub const MAX_CPU_SECONDS: u64 = 60;

/// CPU samples per second, in pull and push mode alike.
const SAMPLE_RATE: u32 = 100;

/// Name the Pyroscope agent reports itself under.
const SPY_NAME: &str = "pyroscope-rs";

/// What pull mode serves, shared by the admin handlers.
#[derive(Debug)]
pub(crate) struct Pull {
    cpu: bool,
    heap: bool,
    /// One permit: there is a single process-wide CPU profiler.
    cpu_slot: Arc<Semaphore>,
    shutdown: Shutdown,
}

impl Pull {
    pub(crate) fn new(config: &ProfilingConfig, shutdown: Shutdown) -> Self {
        Self {
            cpu: config.profiles.contains(&Profile::Cpu),
            heap: config.profiles.contains(&Profile::Heap),
            cpu_slot: Arc::new(Semaphore::new(1)),
            shutdown,
        }
    }
}

/// The `/debug/pprof/*` routes.
pub(crate) fn router(pull: Arc<Pull>) -> Router {
    Router::new()
        .route("/debug/pprof/profile", get(cpu))
        .route("/debug/pprof/heap", get(heap))
        .with_state(pull)
}

#[derive(Debug, Deserialize)]
struct CpuParams {
    seconds: Option<u64>,
}

async fn cpu(State(pull): State<Arc<Pull>>, Query(params): Query<CpuParams>) -> Response {
    if !pull.cpu {
        return unavailable("cpu profiles are off; add cpu to observability.profiling.profiles");
    }
    let seconds = params.seconds.unwrap_or(DEFAULT_CPU_SECONDS);
    if !(1..=MAX_CPU_SECONDS).contains(&seconds) {
        return text(
            StatusCode::BAD_REQUEST,
            format!("seconds must be between 1 and {MAX_CPU_SECONDS}"),
        );
    }
    let Ok(slot) = Arc::clone(&pull.cpu_slot).try_acquire_owned() else {
        return text(StatusCode::CONFLICT, "a CPU profile is already running");
    };

    // The profiler runs on a blocking thread until the duration passes or `stop` is dropped,
    // which happens on shutdown and when the client goes away and this future is dropped.
    let (stop, stopped) = mpsc::channel::<Infallible>();
    let duration = Duration::from_secs(seconds);
    let mut task = tokio::task::spawn_blocking(move || {
        let profile = record_cpu(duration, &stopped);
        drop(slot);
        profile
    });
    let result = tokio::select! {
        result = &mut task => result,
        () = pull.shutdown.started() => {
            drop(stop);
            task.await
        }
    };

    match result {
        Ok(Ok(profile)) => pprof_body(profile, "profile.pb"),
        Ok(Err(error)) => {
            tracing::error!(%error, "cpu profile failed");
            text(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cpu profile failed: {error}"),
            )
        }
        Err(error) => {
            tracing::error!(%error, "cpu profile task failed");
            text(StatusCode::INTERNAL_SERVER_ERROR, "cpu profile task failed")
        }
    }
}

/// Samples for `duration` or until `stopped` disconnects, then encodes the samples as pprof.
fn record_cpu(
    duration: Duration,
    stopped: &Receiver<Infallible>,
) -> Result<Vec<u8>, PyroscopeError> {
    let mut profiler = Pprof::new(
        PprofConfig {
            sample_rate: SAMPLE_RATE,
        },
        BackendConfig::default(),
    );
    let started_at = SystemTime::now();
    let started = Instant::now();
    profiler.initialize()?;
    // Returns on timeout or when the sender is dropped; nothing is ever sent.
    let _ = stopped.recv_timeout(duration);
    let batch = profiler.report()?;
    // Dropping the backend stops the sampling timer and the signal handler.
    drop(profiler);

    let reports = match batch.data {
        ReportData::Reports(reports) => reports,
        ReportData::RawPprof(profile) => return Ok(profile),
    };
    let profile = pyroscope::encode::pprof::encode(
        &reports,
        SAMPLE_RATE,
        nanos(started_at.duration_since(UNIX_EPOCH).unwrap_or_default()),
        nanos(started.elapsed()),
    );
    Ok(profile.encode_to_vec())
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

async fn heap(State(pull): State<Arc<Pull>>) -> Response {
    if !pull.heap {
        return unavailable("heap profiles are off; add heap to observability.profiling.profiles");
    }
    dump_heap().await
}

#[cfg(not(target_os = "linux"))]
async fn dump_heap() -> Response {
    unavailable("heap profiles need jemalloc, which this build only links on Linux")
}

#[cfg(target_os = "linux")]
async fn dump_heap() -> Response {
    let Some(control) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return unavailable(heap_setup_hint());
    };
    let control = Arc::clone(control);
    // Dumping writes a file and symbolizes every sampled stack, which can take a while.
    let dumped = tokio::task::spawn_blocking(move || {
        let mut control = control.blocking_lock();
        control.activated().then(|| control.dump_pprof())
    })
    .await;

    match dumped {
        Ok(Some(Ok(profile))) => pprof_body(profile, "heap.pb.gz"),
        Ok(None) => unavailable(heap_setup_hint()),
        Ok(Some(Err(error))) => {
            tracing::error!(error = %format!("{error:#}"), "heap profile failed");
            text(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("heap profile failed: {error:#}"),
            )
        }
        Err(error) => {
            tracing::error!(%error, "heap profile task failed");
            text(
                StatusCode::INTERNAL_SERVER_ERROR,
                "heap profile task failed",
            )
        }
    }
}

#[cfg(target_os = "linux")]
fn heap_setup_hint() -> String {
    format!(
        "jemalloc heap profiling is off; use neon_observability::profiling::Jemalloc as the \
         global allocator and start the process with {MALLOC_CONF_ENV}={MALLOC_CONF}"
    )
}

fn pprof_body(profile: Vec<u8>, filename: &'static str) -> Response {
    let disposition = HeaderValue::try_from(format!("attachment; filename=\"{filename}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    (
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (CONTENT_DISPOSITION, disposition),
        ],
        profile,
    )
        .into_response()
}

fn unavailable(reason: impl Into<String>) -> Response {
    text(StatusCode::NOT_IMPLEMENTED, reason)
}

fn text(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

/// The running Pyroscope agents, one per profile type.
pub(crate) struct Push {
    agents: Vec<(Profile, PyroscopeAgent<PyroscopeAgentRunning>)>,
}

impl std::fmt::Debug for Push {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push")
            .field(
                "agents",
                &self
                    .agents
                    .iter()
                    .map(|(profile, _)| profile)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Push {
    /// Starts an agent for each configured profile type. An agent that fails to start stops the
    /// ones started before it.
    pub(crate) fn start(
        config: &ProfilingConfig,
        service: &ServiceInfo,
    ) -> Result<Self, PyroscopeError> {
        let mut push = Self { agents: Vec::new() };
        for profile in &config.profiles {
            if push.agents.iter().any(|(started, _)| started == profile) {
                continue;
            }
            let Some(backend) = backend(*profile) else {
                continue;
            };
            match start_agent(config, service, backend) {
                Ok(agent) => push.agents.push((*profile, agent)),
                Err(error) => {
                    if let Err(stop_error) = push.stop() {
                        tracing::warn!(error = %stop_error, "cannot stop the started Pyroscope agents");
                    }
                    return Err(error);
                }
            }
        }
        Ok(push)
    }

    /// Stops every agent, which flushes its last batch. Blocks until they are done.
    pub(crate) fn stop(self) -> Result<(), StopError> {
        let mut failures = Vec::new();
        for (profile, agent) in self.agents {
            match agent.stop() {
                Ok(ready) => ready.shutdown(),
                Err(error) => failures.push(format!("{profile:?}: {error}")),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(StopError(failures.join("; ")))
        }
    }
}

/// Some Pyroscope agents failed to stop.
#[derive(Debug, thiserror::Error)]
#[error("cannot stop Pyroscope agents: {0}")]
pub(crate) struct StopError(String);

fn start_agent(
    config: &ProfilingConfig,
    service: &ServiceInfo,
    backend: BackendImpl<BackendUninitialized>,
) -> Result<PyroscopeAgent<PyroscopeAgentRunning>, PyroscopeError> {
    let address = config.push.server_address.as_deref().unwrap_or_default();
    let tags = config
        .push
        .tags
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    PyroscopeAgentBuilder::new(
        address,
        &service.name,
        SAMPLE_RATE,
        SPY_NAME,
        env!("CARGO_PKG_VERSION"),
        backend,
    )
    .tags(tags)
    .upload_interval(Duration::from_secs(config.push.upload_interval_seconds))
    .build()?
    .start()
}

fn backend(profile: Profile) -> Option<BackendImpl<BackendUninitialized>> {
    match profile {
        Profile::Cpu => Some(pyroscope::backend::pprof_backend(
            PprofConfig {
                sample_rate: SAMPLE_RATE,
            },
            BackendConfig::default(),
        )),
        Profile::Heap => heap_backend(),
    }
}

#[cfg(target_os = "linux")]
fn heap_backend() -> Option<BackendImpl<BackendUninitialized>> {
    let active = jemalloc_pprof::PROF_CTL
        .as_ref()
        .and_then(|control| control.try_lock().ok().map(|control| control.activated()))
        .unwrap_or(false);
    if active {
        Some(pyroscope::backend::jemalloc_backend())
    } else {
        tracing::warn!("{}; heap profiles are not pushed", heap_setup_hint());
        None
    }
}

#[cfg(not(target_os = "linux"))]
fn heap_backend() -> Option<BackendImpl<BackendUninitialized>> {
    tracing::warn!(
        "heap profiles need jemalloc, which this build only links on Linux; heap profiles are \
         not pushed"
    );
    None
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use http::Request;
    use tower::ServiceExt as _;

    use super::*;
    use crate::config::ProfilingMode;

    fn pull(profiles: Vec<Profile>) -> (Arc<Pull>, Shutdown) {
        let shutdown = Shutdown::new();
        let config = ProfilingConfig {
            mode: ProfilingMode::Pull,
            profiles,
            ..ProfilingConfig::default()
        };
        (Arc::new(Pull::new(&config, shutdown.clone())), shutdown)
    }

    async fn get_bytes(pull: &Arc<Pull>, uri: &str) -> (StatusCode, Vec<u8>) {
        let response = router(Arc::clone(pull))
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, body.to_vec())
    }

    async fn get(pull: &Arc<Pull>, uri: &str) -> (StatusCode, String) {
        let (status, body) = get_bytes(pull, uri).await;
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn rejects_out_of_range_durations() {
        let (pull, _) = pull(vec![Profile::Cpu]);
        for uri in [
            "/debug/pprof/profile?seconds=0",
            "/debug/pprof/profile?seconds=61",
            "/debug/pprof/profile?seconds=soon",
        ] {
            assert_eq!(get(&pull, uri).await.0, StatusCode::BAD_REQUEST, "{uri}");
        }
    }

    #[tokio::test]
    async fn profiles_that_are_off_answer_501() {
        let (pull, _) = pull(vec![]);
        let (status, body) = get(&pull, "/debug/pprof/profile").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains("add cpu"), "{body}");
        let (status, body) = get(&pull, "/debug/pprof/heap").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains("add heap"), "{body}");
    }

    #[tokio::test]
    async fn heap_without_jemalloc_profiling_answers_501_with_the_fix() {
        let (pull, _) = pull(vec![Profile::Heap]);
        let (status, body) = get(&pull, "/debug/pprof/heap").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains("jemalloc"), "{body}");
    }

    #[tokio::test]
    async fn a_second_cpu_profile_gets_409() {
        let (pull, _) = pull(vec![Profile::Cpu]);
        let _running = Arc::clone(&pull.cpu_slot).try_acquire_owned().unwrap();
        let (status, _) = get(&pull, "/debug/pprof/profile?seconds=1").await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    /// The only test that runs the real profiler, so no two profiles race in one process.
    #[tokio::test]
    async fn shutdown_cuts_a_cpu_profile_short_and_returns_pprof() {
        let (pull, shutdown) = pull(vec![Profile::Cpu]);
        let request = tokio::spawn({
            let pull = Arc::clone(&pull);
            async move { get_bytes(&pull, "/debug/pprof/profile?seconds=60").await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;

        let started = Instant::now();
        shutdown.run().await.unwrap();
        let (status, body) = request.await.unwrap();

        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert!(started.elapsed() < Duration::from_secs(30));
        let profile = pyroscope::encode::r#gen::google::Profile::decode(body.as_slice()).unwrap();
        let sample_type = &profile.sample_type[0];
        let name = |index: i64| profile.string_table[usize::try_from(index).unwrap()].as_str();
        assert_eq!(
            (name(sample_type.r#type), name(sample_type.unit)),
            ("cpu", "nanoseconds")
        );
        assert_eq!(profile.period, 10_000_000);
        assert!(profile.duration_nanos > 0);
        let slot = Arc::clone(&pull.cpu_slot).try_acquire_owned();
        assert!(slot.is_ok(), "the profiler slot was not released");
    }
}
