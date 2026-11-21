//! Starting the server's parts, and stopping them in order.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use neon_app::jobs::{LISTEN_CONNECTIONS, Worker, WorkerConfig, WorkerHandle};
use neon_app::settings::{SettingsService, registry};
use neon_domain::duration;
use neon_domain::settings::Pinned;
use neon_store::PoolConfig;
use sqlx::PgPool;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::{DatabaseConfig, Loaded};
use crate::http::{self, AppState};

/// How long stopping waits for open requests before closing their connections.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a client may take to send a request's headers, which is also how long an
/// idle keep-alive connection stays open. Without it, a client that never finishes its
/// headers would hold a connection, and stopping would wait for it.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

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
    report(&loaded);
    let pool = connect(&loaded.config.database, 1).await?;
    apply_migrations(&pool).await?;
    pool.close().await;
    Ok(())
}

/// Logs where the configuration came from, and the variables it ignored.
fn report(loaded: &Loaded) {
    match &loaded.file {
        Some(path) => info!(file = %path.display(), "configuration read"),
        None => info!("no config file; using the defaults and the environment"),
    }
    if !loaded.ignored_vars.is_empty() {
        warn!(
            variables = %loaded.ignored_vars.join(", "),
            "ignoring NEON_ environment variables that set no configuration key"
        );
    }
}

/// The running parts.
struct Server {
    http: Http,
    worker: WorkerHandle,
    pool: PgPool,
}

impl Server {
    async fn start(loaded: Loaded) -> anyhow::Result<Self> {
        report(&loaded);
        let Loaded {
            config, settings, ..
        } = loaded;

        // The job worker opens its listening connection beside the pool, so the pool
        // gets the rest of the configured connections.
        let pool_size = config
            .database
            .max_connections
            .saturating_sub(LISTEN_CONNECTIONS)
            .max(1);
        let pool = connect(&config.database, pool_size).await?;
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
        let router = http::router(AppState { settings });
        let http = Http::spawn(listener, router, HEADER_READ_TIMEOUT)?;

        Ok(Self { http, worker, pool })
    }

    async fn stop(self) {
        for step in self.stop_steps() {
            debug!(step = step.name, "stopping");
            step.run.await;
        }
        info!("stopped");
    }

    /// The steps of stopping, in dependency order: no new requests and the open ones
    /// finish, then the job worker stops, and the pool closes last because both use it
    /// until then. Each step is its own future, so an ordered shutdown registry can take
    /// them one by one.
    fn stop_steps(self) -> [StopStep; 3] {
        let Self { http, worker, pool } = self;
        [
            StopStep::new("http", http.stop(DRAIN_TIMEOUT)),
            StopStep::new("job worker", worker.stop()),
            StopStep::new("database pool", async move { pool.close().await }),
        ]
    }
}

/// One step of stopping the server.
struct StopStep {
    name: &'static str,
    run: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl StopStep {
    fn new(name: &'static str, run: impl Future<Output = ()> + Send + 'static) -> Self {
        Self {
            name,
            run: Box::pin(run),
        }
    }
}

async fn connect(database: &DatabaseConfig, max_connections: u32) -> anyhow::Result<PgPool> {
    neon_store::connect(PoolConfig {
        url: database.url.expose(),
        max_connections,
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

/// The HTTP server, running until told to stop. It speaks HTTP/1.1, with upgrades for
/// WebSockets; TLS and HTTP/2 end at the reverse proxy in front of it.
struct Http {
    addr: SocketAddr,
    stop: CancellationToken,
    /// Accepts connections, and hands back the open ones once it stops.
    accepting: JoinHandle<JoinSet<()>>,
}

type Connection = http1::UpgradeableConnection<TokioIo<TcpStream>, TowerToHyperService<Router>>;

impl Http {
    fn spawn(listener: TcpListener, router: Router, header_timeout: Duration) -> io::Result<Self> {
        let addr = listener.local_addr()?;
        let mut builder = http1::Builder::new();
        builder
            .timer(TokioTimer::new())
            .header_read_timeout(header_timeout);
        let stop = CancellationToken::new();
        let accepting = tokio::spawn(accept(listener, builder, router, stop.clone()));
        info!(%addr, "HTTP server listening");
        Ok(Self {
            addr,
            stop,
            accepting,
        })
    }

    /// Stops accepting connections and closes idle ones, waits up to `timeout` for open
    /// requests to finish, then closes the connections still open, cancelling their
    /// requests.
    async fn stop(self, timeout: Duration) {
        self.stop.cancel();
        let mut connections = match self.accepting.await {
            Ok(connections) => connections,
            Err(error) => {
                error!(%error, "HTTP server task failed");
                return;
            }
        };

        let drained = tokio::time::timeout(timeout, async {
            while let Some(joined) = connections.join_next().await {
                connection_ended(joined);
            }
        })
        .await;
        if drained.is_err() {
            warn!(
                connections = connections.len(),
                "requests still open after {}; closing them",
                duration::format(timeout)
            );
            connections.shutdown().await;
        }
        info!(addr = %self.addr, "HTTP server stopped");
    }
}

/// Accepts connections until `stop` is cancelled, then returns the ones still open. The
/// listener closes on return, so new connections are refused from then on.
async fn accept(
    listener: TcpListener,
    builder: http1::Builder,
    router: Router,
    stop: CancellationToken,
) -> JoinSet<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            Some(joined) = connections.join_next() => connection_ended(joined),
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let service = TowerToHyperService::new(router.clone());
                    let connection = builder
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades();
                    connections.spawn(serve_connection(connection, stop.clone()));
                }
                Err(error) => accept_failed(error).await,
            },
        }
    }
    connections
}

/// Serves one connection. Once `stop` is cancelled it closes when idle: at once if no
/// request is open, else after the response.
async fn serve_connection(connection: Connection, stop: CancellationToken) {
    let mut connection = pin!(connection);
    let result = tokio::select! {
        result = connection.as_mut() => result,
        () = stop.cancelled() => {
            connection.as_mut().graceful_shutdown();
            connection.await
        }
    };
    // Clients that hang up, or are too slow sending headers, end here.
    if let Err(error) = result {
        debug!(%error, "HTTP connection closed with an error");
    }
}

fn connection_ended(joined: Result<(), JoinError>) {
    if let Err(error) = joined {
        error!(%error, "HTTP connection task failed");
    }
}

/// A client that hung up before its connection was accepted is no error of the server's.
/// Anything else, such as running out of file descriptors, is logged, and accepting
/// pauses so the loop does not spin while it lasts.
async fn accept_failed(error: io::Error) {
    if matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    ) {
        return;
    }
    error!(%error, "cannot accept HTTP connections");
    tokio::time::sleep(Duration::from_secs(1)).await;
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use axum::routing::get;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::sync::Notify;
    use tokio::time::{sleep, timeout};

    use super::*;

    /// Long enough for any step here; reaching it means a connection was left open.
    const PATIENCE: Duration = Duration::from_secs(5);

    async fn start(router: Router, header_timeout: Duration) -> Http {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        Http::spawn(listener, router, header_timeout).expect("starts")
    }

    async fn send(addr: SocketAddr, request: &str) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.expect("connects");
        stream.write_all(request.as_bytes()).await.expect("sends");
        stream
    }

    /// Reads until the server closes the connection, and returns what it sent.
    async fn read_until_closed(stream: &mut TcpStream) -> String {
        let mut received = Vec::new();
        let mut buf = [0; 1024];
        let read = timeout(PATIENCE, async {
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => received.extend_from_slice(&buf[..n]),
                }
            }
        })
        .await;
        assert!(read.is_ok(), "the connection stayed open");
        String::from_utf8_lossy(&received).into_owned()
    }

    async fn refuses_connections(addr: SocketAddr) -> bool {
        for _ in 0..50 {
            if TcpStream::connect(addr).await.is_err() {
                return true;
            }
            sleep(Duration::from_millis(20)).await;
        }
        false
    }

    /// A route that signals when it starts, then answers after `delay`.
    fn slow(started: &Arc<Notify>, delay: Duration) -> Router {
        let started = Arc::clone(started);
        Router::new().route(
            "/slow",
            get(move || async move {
                started.notify_one();
                sleep(delay).await;
                "done"
            }),
        )
    }

    /// Set when dropped: a request whose handler is dropped was cancelled.
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn clients_that_do_not_finish_their_headers_are_disconnected() {
        let http = start(Router::new(), Duration::from_millis(200)).await;
        let begun = Instant::now();

        let mut partial = send(http.addr, "GET /slow HTTP/1.1\r\nHost: neon\r\n").await;
        let mut silent = send(http.addr, "").await;

        assert_eq!(read_until_closed(&mut partial).await, "");
        assert_eq!(read_until_closed(&mut silent).await, "");
        assert!(begun.elapsed() >= Duration::from_millis(200));
        http.stop(PATIENCE).await;
    }

    #[tokio::test]
    async fn idle_keep_alive_connections_are_closed() {
        let started = Arc::new(Notify::new());
        let http = start(slow(&started, Duration::ZERO), Duration::from_millis(200)).await;
        let begun = Instant::now();

        let mut client = send(http.addr, "GET /slow HTTP/1.1\r\nHost: neon\r\n\r\n").await;
        let response = read_until_closed(&mut client).await;

        // The response came at once; the connection was then held until it sat idle.
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(response.ends_with("\r\n\r\ndone"), "{response}");
        assert!(begun.elapsed() >= Duration::from_millis(200));
        http.stop(PATIENCE).await;
    }

    #[tokio::test]
    async fn stopping_refuses_new_connections_and_lets_open_requests_finish() {
        let started = Arc::new(Notify::new());
        let answered = Arc::new(AtomicBool::new(false));
        let router = Router::new().route(
            "/slow",
            get({
                let started = Arc::clone(&started);
                let answered = Arc::clone(&answered);
                move || async move {
                    started.notify_one();
                    sleep(Duration::from_secs(1)).await;
                    answered.store(true, Ordering::SeqCst);
                    "done"
                }
            }),
        );
        let http = start(router, PATIENCE).await;
        let addr = http.addr;
        let mut idle = send(addr, "").await;
        let mut busy = send(addr, "GET /slow HTTP/1.1\r\nHost: neon\r\n\r\n").await;
        started.notified().await;

        let stopping = tokio::spawn(http.stop(PATIENCE));

        assert!(refuses_connections(addr).await);
        assert_eq!(read_until_closed(&mut idle).await, "");
        assert!(
            !answered.load(Ordering::SeqCst),
            "the request ended too soon"
        );
        let response = read_until_closed(&mut busy).await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(response.ends_with("done"), "{response}");
        timeout(PATIENCE, stopping)
            .await
            .expect("stops once the request is answered")
            .expect("the stop task ran");
    }

    #[tokio::test]
    async fn stopping_closes_connections_still_open_after_the_drain_timeout() {
        let started = Arc::new(Notify::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        let router = Router::new().route(
            "/hang",
            get({
                let started = Arc::clone(&started);
                let cancelled = Arc::clone(&cancelled);
                move || async move {
                    let _flag = DropFlag(cancelled);
                    started.notify_one();
                    std::future::pending::<()>().await;
                }
            }),
        );
        let http = start(router, Duration::from_secs(60)).await;
        let mut hung = send(http.addr, "GET /hang HTTP/1.1\r\nHost: neon\r\n\r\n").await;
        let mut partial = send(http.addr, "GET /hang HTTP/1.1\r\nHost: neon\r\n").await;
        started.notified().await;

        let begun = Instant::now();
        http.stop(Duration::from_millis(200)).await;

        assert!(
            begun.elapsed() < Duration::from_secs(2),
            "{:?}",
            begun.elapsed()
        );
        assert!(cancelled.load(Ordering::SeqCst), "the request still runs");
        assert_eq!(read_until_closed(&mut hung).await, "");
        assert_eq!(read_until_closed(&mut partial).await, "");
    }

    #[sqlx::test(migrator = "neon_store::MIGRATOR")]
    async fn the_server_stops_http_then_the_worker_then_the_pool(pool: PgPool) {
        let started = Arc::new(Notify::new());
        // The request reads from the database once it is drained, so it only succeeds
        // if the pool is still open while requests finish.
        let router = Router::new().route(
            "/slow",
            get({
                let started = Arc::clone(&started);
                let pool = pool.clone();
                move || async move {
                    started.notify_one();
                    sleep(Duration::from_millis(300)).await;
                    match sqlx::query_scalar::<_, i32>("SELECT 7")
                        .fetch_one(&pool)
                        .await
                    {
                        Ok(n) => n.to_string(),
                        Err(error) => error.to_string(),
                    }
                }
            }),
        );
        let http = start(router, PATIENCE).await;
        let addr = http.addr;
        let worker = Worker::new(pool.clone(), WorkerConfig::default())
            .start()
            .await
            .expect("the worker starts");
        let server = Server {
            http,
            worker,
            pool: pool.clone(),
        };
        let mut client = send(addr, "GET /slow HTTP/1.1\r\nHost: neon\r\n\r\n").await;
        started.notified().await;

        let steps = server.stop_steps();
        let names: Vec<&str> = steps.iter().map(|step| step.name).collect();
        assert_eq!(names, ["http", "job worker", "database pool"]);
        let mut steps = steps.into_iter();

        steps.next().expect("the HTTP step").run.await;
        let response = read_until_closed(&mut client).await;
        assert!(response.ends_with("\r\n\r\n7"), "{response}");
        assert!(refuses_connections(addr).await);
        assert!(!pool.is_closed());

        for step in steps {
            step.run.await;
        }
        assert!(pool.is_closed());
    }
}
