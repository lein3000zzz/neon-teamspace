//! Starting the server's parts, and stopping them in order.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use neon_app::jobs::{Worker, WorkerConfig, WorkerHandle};
use neon_app::settings::{SettingsService, registry};
use neon_domain::duration;
use neon_domain::settings::Pinned;
use neon_store::PoolConfig;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::{DatabaseConfig, Loaded};
use crate::http::{self, AppState};

/// How long stopping waits for open requests before closing their connections.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs the server until SIGINT or SIGTERM, then stops it.
pub async fn serve(loaded: Loaded) -> anyhow::Result<()> {
    let mut signals = ShutdownSignals::install().context("listening for shutdown signals")?;
    let server = Server::start(loaded).await?;

    let signal = signals.recv().await;
    info!(signal, "shutting down");
    server.stop().await;
    Ok(())
}

/// Applies pending migrations, then closes the connection.
pub async fn migrate(loaded: Loaded) -> anyhow::Result<()> {
    let pool = connect(&loaded.config.database).await?;
    apply_migrations(&pool).await?;
    pool.close().await;
    Ok(())
}

/// The running parts.
struct Server {
    http: Http,
    worker: WorkerHandle,
    pool: PgPool,
}

impl Server {
    async fn start(loaded: Loaded) -> anyhow::Result<Self> {
        let Loaded {
            config,
            settings,
            file,
        } = loaded;
        match &file {
            Some(path) => info!(file = %path.display(), "configuration read"),
            None => info!("no config file; using the defaults and the environment"),
        }

        let pool = connect(&config.database).await?;
        if config.database.auto_migrate {
            apply_migrations(&pool).await?;
        }
        let settings = load_settings(&pool, settings).await?;
        let listener = TcpListener::bind(config.server.listen)
            .await
            .with_context(|| format!("listening on {}", config.server.listen))?;
        let worker = Worker::new(pool.clone(), WorkerConfig::default())
            .start()
            .await
            .context("starting the job worker")?;
        let http = Http::spawn(listener, http::router(AppState { settings }))?;

        Ok(Self { http, worker, pool })
    }

    /// Stops in dependency order: no new requests, open ones finish, then the job
    /// worker stops, and the pool closes last because both use it until then. Each step
    /// is its own call, so an ordered shutdown registry can take them one by one.
    async fn stop(self) {
        self.http.stop(DRAIN_TIMEOUT).await;
        self.worker.stop().await;
        self.pool.close().await;
        info!("stopped");
    }
}

async fn connect(database: &DatabaseConfig) -> anyhow::Result<PgPool> {
    neon_store::connect(PoolConfig {
        url: database.url.expose(),
        max_connections: database.max_connections,
    })
    .await
    // The URL's Display hides its password.
    .with_context(|| format!("connecting to {}", database.url))
}

async fn apply_migrations(pool: &PgPool) -> anyhow::Result<()> {
    neon_store::migrate(pool)
        .await
        .context("applying database migrations")?;
    info!("database migrations applied");
    Ok(())
}

async fn load_settings(pool: &PgPool, pinned: Pinned) -> anyhow::Result<Arc<SettingsService>> {
    let (settings, warnings) = SettingsService::load(pool.clone(), registry(), pinned)
        .await
        .context("loading runtime settings")?;
    for warning in warnings {
        warn!("{warning}");
    }
    Ok(Arc::new(settings))
}

/// The HTTP server, running until told to stop.
struct Http {
    addr: SocketAddr,
    stop: CancellationToken,
    task: JoinHandle<io::Result<()>>,
}

impl Http {
    fn spawn(listener: TcpListener, router: Router) -> anyhow::Result<Self> {
        let addr = listener.local_addr()?;
        let stop = CancellationToken::new();
        let task = tokio::spawn(
            axum::serve(listener, router)
                .with_graceful_shutdown(stop.clone().cancelled_owned())
                .into_future(),
        );
        info!(%addr, "HTTP server listening");
        Ok(Self { addr, stop, task })
    }

    /// Stops accepting connections and waits up to `timeout` for open requests, then
    /// drops whatever is left.
    async fn stop(mut self, timeout: Duration) {
        self.stop.cancel();
        match tokio::time::timeout(timeout, &mut self.task).await {
            Ok(Ok(Ok(()))) => info!(addr = %self.addr, "HTTP server stopped"),
            Ok(Ok(Err(error))) => error!(%error, "HTTP server failed"),
            Ok(Err(error)) => error!(%error, "HTTP server task failed"),
            Err(_) => {
                warn!(
                    "requests still open after {}; closing them",
                    duration::format(timeout)
                );
                self.task.abort();
            }
        }
    }
}

/// SIGINT and SIGTERM. They are installed before anything starts, so a failure stops
/// the start instead of leaving a server that cannot be stopped cleanly.
#[cfg(unix)]
struct ShutdownSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl ShutdownSignals {
    fn install() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};

        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    async fn recv(&mut self) -> &'static str {
        tokio::select! {
            _ = self.interrupt.recv() => "SIGINT",
            _ = self.terminate.recv() => "SIGTERM",
        }
    }
}

/// Ctrl-C, where there are no Unix signals.
#[cfg(not(unix))]
struct ShutdownSignals;

#[cfg(not(unix))]
impl ShutdownSignals {
    fn install() -> io::Result<Self> {
        Ok(Self)
    }

    async fn recv(&mut self) -> &'static str {
        if let Err(error) = tokio::signal::ctrl_c().await {
            error!(%error, "cannot listen for Ctrl-C; stopping");
        }
        "Ctrl-C"
    }
}
