//! Starting the server's parts, and stopping them in order.
//!
//! Each part registers how it stops with the telemetry's shutdown registry as it starts,
//! and the registry runs the stops in reverse order. On SIGINT or SIGTERM, `/readyz` turns
//! to 503 at once, the MCP sessions end, then the HTTP server stops accepting and drains,
//! the job worker stops, the gauges stop if metrics are on, the live documents close their
//! connections and are saved, the hourly OAuth, workspace and auth cleanups stop, the auth
//! work begun after earlier answers finishes, the database pool closes, the admin listener
//! stops, and the telemetry flushes last, so what the other parts log while stopping is
//! still written.

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use neon_app::attachments::AttachmentService;
use neon_app::auth::{AuthService, CLEANUP_INTERVAL, Cleanup, NewAccount};
use neon_app::clock::Clock;
use neon_app::comments::CommentService;
use neon_app::crypto::Envelope;
use neon_app::databases::{DatabaseService, LiveDatabases};
use neon_app::history::HistoryService;
use neon_app::jobs::{self, LISTEN_CONNECTIONS, Worker, WorkerConfig};
use neon_app::mail::{MailJob, MailQueue};
use neon_app::notifications::{FeedLimits, InboxFeeds, InboxService, NotificationMailJob};
use neon_app::oauth::{Cleanup as OAuthCleanup, OAuthService};
use neon_app::pages::{AccessChanges, Fanout, PageService};
use neon_app::periodic::Periodic;
use neon_app::ratelimit::{MemoryLimiter, RateLimiter};
use neon_app::search::SearchService;
use neon_app::settings::{SettingsService, mail_set_up, registry};
use neon_app::sync::{self, Broadcaster, LocalBroadcaster, SyncService};
use neon_app::transfer::TransferService;
use neon_app::webhooks::{DELIVERY_SLOTS, DeliveryJob, Services, WebhookService};
use neon_app::workspaces::{AgentService, Cleanup as WorkspaceCleanup, WorkspaceService};
use neon_domain::auth::{Role, User};
use neon_domain::duration;
use neon_domain::settings::{Pinned, Secret};
use neon_observability::Telemetry;
use neon_observability::admin::{self, Readiness};
use neon_observability::shutdown::{DEFAULT_HOOK_TIMEOUT, Shutdown};
use neon_store::PoolConfig;
use sqlx::{Connection as _, PgPool};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::{AdminConfig, Config, DatabaseConfig, Loaded};
use crate::gauges::{self, Gauges};
use crate::http::{self, AppState, WebConfig};
use crate::mail::SmtpMailer;
use crate::mcp::{self, Mcp};
use crate::storage::S3Connector;
use crate::telemetry;
use crate::webhooks::HttpTransport;

/// How long stopping waits for open requests before closing their connections.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a client may take to send a request's headers, which is also how long an
/// idle keep-alive connection stays open. Without it, a client that never finishes its
/// headers would hold a connection, and stopping would wait for it.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Added to the bound a stop sets itself, such as [`DRAIN_TIMEOUT`], for the time the
/// shutdown registry gives it, so the registry never abandons a stop still within its bound.
const STOP_MARGIN: Duration = Duration::from_secs(10);

/// How long `/readyz` waits for the database to answer.
const PING_TIMEOUT: Duration = Duration::from_secs(2);

/// How long stopping waits for auth work begun after earlier answers, such as mail after a
/// request to join. Each piece gives up after a minute itself.
const BACKGROUND_STOP: Duration = Duration::from_secs(60);

/// How long stopping waits for the live documents to be saved.
const SAVE_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the server looks for digests that are due.
const DIGEST_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Runs the server until SIGINT or SIGTERM, then stops it.
pub async fn serve(loaded: Loaded) -> anyhow::Result<()> {
    with_telemetry(loaded, |loaded, telemetry| async move {
        let mut signals = ShutdownSignals::install().context("listening for shutdown signals")?;
        let started = Arc::new(OnceLock::new());
        let admin = &loaded.config.observability.admin;
        admin::spawn(admin, &telemetry, readiness(Arc::clone(&started)))
            .await
            .with_context(|| format!("starting the admin listener on {}", admin.addr))?;
        let pool = start(loaded, telemetry.shutdown()).await?;
        started.get_or_init(|| pool);

        let signal = signals.recv().await;
        info!(signal, "shutting down");
        Ok(())
    })
    .await
}

/// Makes an admin, whatever the registration mode and however many admins there are, for
/// `neon admin create`. Applies pending migrations first, as `serve` does.
pub async fn create_admin(loaded: Loaded, account: NewAccount) -> anyhow::Result<User> {
    let mut created = None;
    with_telemetry(loaded, |loaded, _| async {
        let Loaded {
            config, settings, ..
        } = loaded;
        let pool = connect(&config.database, 2).await?;
        let made = async {
            if config.database.auto_migrate {
                apply_migrations(&pool).await?;
            }
            let envelope = envelope(&config)?;
            let settings = load_settings(&pool, settings, envelope.clone()).await?;
            let auth = AuthService::builder(pool.clone(), settings)
                .envelope(envelope)
                .build()
                .await
                .context("starting the auth service")?;
            let user = auth
                .bootstrap_admin(NewAccount {
                    role: Role::Admin,
                    ..account
                })
                .await
                .context("creating the admin")?;
            auth.settle().await;
            Ok::<_, anyhow::Error>(user)
        }
        .await;
        pool.close().await;
        created = Some(made?);
        Ok(())
    })
    .await?;
    created.context("the admin was not created")
}

/// Applies pending migrations, then closes the connection.
pub async fn migrate(loaded: Loaded) -> anyhow::Result<()> {
    with_telemetry(loaded, |loaded, _| async move {
        let pool = connect(&loaded.config.database, 1).await?;
        let applied = apply_migrations(&pool).await;
        pool.close().await;
        applied
    })
    .await
}

/// Stores pages' text for search again, those whose text lacks their latest changes or
/// `all`, for `neon search reindex`, and says how many pages' text changed. Applies pending
/// migrations first, as `serve` does.
pub async fn reindex(loaded: Loaded, all: bool) -> anyhow::Result<u64> {
    let mut stored = 0;
    with_telemetry(loaded, |loaded, _| async {
        let Loaded { config, .. } = loaded;
        let pool = connect(&config.database, 2).await?;
        let done = async {
            if config.database.auto_migrate {
                apply_migrations(&pool).await?;
            }
            SearchService::builder(pool.clone())
                .build()
                .reindex(all)
                .await
                .context("storing pages' search text")
        }
        .await;
        pool.close().await;
        stored = done?;
        Ok(())
    })
    .await?;
    Ok(stored)
}

/// Installs telemetry, runs `command`, then runs the shutdown registry, which stops what
/// the command started and flushes the telemetry last. A failure is logged before that, so
/// it reaches the logs as well as the exit status.
async fn with_telemetry<F, Fut>(loaded: Loaded, command: F) -> anyhow::Result<()>
where
    F: FnOnce(Loaded, Telemetry) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let telemetry = telemetry::init(&loaded.config.observability).context("starting telemetry")?;
    report(&loaded);
    let result = command(loaded, telemetry.clone()).await;
    if let Err(error) = &result {
        error!(error = %format!("{error:#}"), "stopping after a failure");
    }
    let stopped = telemetry.shutdown().run().await;
    result?;
    Ok(stopped?)
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

/// Answers `/readyz`: not ready until `started` holds the pool, which happens once the
/// HTTP server listens, then ready while the database answers within [`PING_TIMEOUT`].
fn readiness(started: Arc<OnceLock<PgPool>>) -> Readiness {
    Readiness::new(move || {
        let pool = started.get().cloned();
        async move {
            let pool = pool.ok_or_else(|| "starting".to_owned())?;
            ping(&pool).await
        }
    })
}

async fn ping(pool: &PgPool) -> Result<(), String> {
    let ping = async { pool.acquire().await?.ping().await };
    match tokio::time::timeout(PING_TIMEOUT, ping).await {
        Ok(Ok(())) => Ok(()),
        // sqlx errors never show the connection URL.
        Ok(Err(error)) => Err(format!("the database is unreachable: {error}")),
        Err(_) => Err(format!(
            "the database did not answer within {}",
            duration::format(PING_TIMEOUT)
        )),
    }
}

/// Connects, prepares the database and starts every part, registering each one's stop.
/// Returns the pool once the HTTP server listens.
async fn start(loaded: Loaded, shutdown: &Shutdown) -> anyhow::Result<PgPool> {
    let Loaded {
        config, settings, ..
    } = loaded;

    // The job worker opens its listening connection beside the pool, so the pool gets the
    // rest of the configured connections.
    let pool_size = config
        .database
        .max_connections
        .saturating_sub(LISTEN_CONNECTIONS)
        .max(1);
    let pool = connect(&config.database, pool_size).await?;
    close_on_shutdown(shutdown, pool.clone());
    if config.database.auto_migrate {
        apply_migrations(&pool).await?;
    }
    let envelope = envelope(&config)?;
    let settings = load_settings(&pool, settings, envelope.clone()).await?;
    let public_url = config.server.public_url.as_ref();
    let origin = public_url.map(|url| url.origin().ascii_serialization());
    let mail = MailQueue::new(
        Arc::clone(&settings),
        envelope.clone(),
        origin.as_deref(),
        Clock::system(),
    );
    report_mail(&mail, &settings);
    // One set of buckets for every use case, so each key is counted once.
    let limiter: Arc<dyn RateLimiter> = Arc::new(MemoryLimiter::new());
    // These services tell the live documents and the database feeds whose access to pages
    // may have changed, and whose sessions or tokens ended, so they close or limit the
    // connections of those who lost it.
    let (documents_listener, feed) = sync::access_feed();
    let (databases_listener, databases_feed) = sync::access_feed();
    let (inbox_listener, inbox_feed) = sync::access_feed();
    let (mcp_listener, mcp_feed) = mcp::access_feed();
    let access: Arc<dyn AccessChanges> = Arc::new(Fanout::new(vec![
        documents_listener,
        databases_listener,
        inbox_listener,
        mcp_listener,
    ]));
    // One broadcaster for the live documents and the database feeds, in rooms of their own.
    let broadcaster: Arc<dyn Broadcaster> = Arc::new(LocalBroadcaster::new());
    let auth = AuthService::builder(pool.clone(), Arc::clone(&settings))
        .envelope(envelope.clone())
        .limiter(Arc::clone(&limiter))
        .mail(mail.clone())
        .access_changes(Arc::clone(&access))
        .build()
        .await
        .context("starting the auth service")?;
    let workspaces = WorkspaceService::builder(pool.clone(), Arc::clone(&settings))
        .envelope(envelope.clone())
        .limiter(Arc::clone(&limiter))
        .access_changes(Arc::clone(&access))
        .broadcaster(Arc::clone(&broadcaster))
        .build();
    let agents = AgentService::new(workspaces.clone());
    let oauth = OAuthService::builder(pool.clone(), Arc::clone(&settings))
        .limiter(Arc::clone(&limiter))
        .access_changes(Arc::clone(&access))
        .build();
    let pages = PageService::builder(pool.clone(), Arc::clone(&settings))
        .access_changes(access)
        .broadcaster(Arc::clone(&broadcaster))
        .build();
    let live = SyncService::builder(pool.clone(), pages.clone(), auth.clone(), feed)
        .broadcaster(Arc::clone(&broadcaster))
        .limiter(Arc::clone(&limiter))
        .build();
    let history = HistoryService::builder(
        pool.clone(),
        Arc::clone(&settings),
        pages.clone(),
        live.clone(),
    )
    .build();
    // Searches take half the pool at most, so the other requests always find connections.
    let search = SearchService::builder(pool.clone())
        .limiter(Arc::clone(&limiter))
        .parallel(usize::try_from(pool_size / 2).unwrap_or(1))
        .build();
    let databases = DatabaseService::builder(pages.clone()).build();
    let feeds = LiveDatabases::builder(databases.clone(), auth.clone(), databases_feed).build();
    let attachments = AttachmentService::builder(
        pool.clone(),
        Arc::clone(&settings),
        pages.clone(),
        live.clone(),
        config.server.data_dir.clone(),
        Arc::new(S3Connector),
    )
    .limiter(Arc::clone(&limiter))
    .build();
    report_storage(&attachments);
    let comments = CommentService::new(pages.clone(), live.clone());
    let inbox = InboxService::new(pages.clone(), mail.clone(), envelope.clone());
    let inbox_feeds = InboxFeeds::new(
        auth.clone(),
        Arc::clone(&broadcaster),
        inbox_feed,
        FeedLimits::default(),
    );
    let transfer = TransferService::builder(
        pages.clone(),
        live.clone(),
        databases.clone(),
        attachments.clone(),
    )
    .limiter(Arc::clone(&limiter))
    .build();
    let webhooks = WebhookService::builder(
        pool.clone(),
        Arc::clone(&settings),
        Services {
            workspaces: workspaces.clone(),
            agents: agents.clone(),
            pages: pages.clone(),
            databases: databases.clone(),
            sync: live.clone(),
            comments: comments.clone(),
        },
        Arc::new(HttpTransport::default()),
    )
    .envelope(envelope)
    .limiter(Arc::clone(&limiter))
    .build();
    let mcp = Mcp::new(
        mcp::Services {
            settings: Arc::clone(&settings),
            auth: auth.clone(),
            workspaces: workspaces.clone(),
            pages: pages.clone(),
            sync: live.clone(),
            history: history.clone(),
            search: search.clone(),
            databases: databases.clone(),
            comments: comments.clone(),
            inbox: inbox.clone(),
            broadcaster,
        },
        limiter,
        mcp_feed,
    );
    ensure_first_admin(&auth, &config.admin).await?;
    start_auth_upkeep(shutdown, &auth);
    start_workspace_upkeep(shutdown, &workspaces, &pages, &history, &attachments);
    start_oauth_upkeep(shutdown, &oauth);
    start_notification_upkeep(shutdown, &inbox);
    save_on_shutdown(shutdown, &live);
    close_feeds_on_shutdown(shutdown, &feeds);
    close_inbox_feeds_on_shutdown(shutdown, &inbox_feeds);
    let listener = TcpListener::bind(config.server.listen)
        .await
        .with_context(|| format!("listening on {}", config.server.listen))?;
    let mailer = SmtpMailer::new(Arc::clone(&settings), public_url);
    // Deliveries take slots of their own, at most DELIVERY_SLOTS, beside mail's.
    let jobs = WorkerConfig {
        concurrency: WorkerConfig::default().concurrency + DELIVERY_SLOTS,
        ..WorkerConfig::default()
    };
    let worker = Worker::new(pool.clone(), jobs)
        .handler(MailJob::new(mail, Arc::new(mailer)))
        .handler(DeliveryJob::new(webhooks.clone()))
        .handler(NotificationMailJob::new(inbox.clone()))
        .span(jobs::run_span)
        .hook(jobs::RedHook::new());
    let web = WebConfig::new(
        config.server.public_url.as_ref(),
        config.server.trusted_proxies.clone(),
    );
    let router = http::router(
        AppState {
            settings,
            auth,
            workspaces,
            agents,
            oauth,
            pages,
            sync: live,
            history,
            search,
            databases,
            live: feeds,
            comments,
            inbox,
            inbox_feeds,
            transfer,
            attachments,
            webhooks,
            web: Arc::new(web),
            mcp: mcp.clone(),
        },
        config.observability.tracing.trust_incoming,
    );
    let sample_gauges = config.observability.metrics.enabled;
    start_parts(shutdown, &pool, sample_gauges, worker, listener, router).await?;
    end_mcp_sessions_on_shutdown(shutdown, mcp);
    Ok(pool)
}

/// The keys `secrets.keys` lists. The load checked them, so this only fails on a
/// configuration built some other way.
fn envelope(config: &Config) -> anyhow::Result<Envelope> {
    let keys = config.secrets.keyring().context("reading secrets.keys")?;
    if keys.is_none() {
        info!(
            "no secrets.keys: invite codes cannot be copied again, and secret settings cannot be saved"
        );
    }
    Ok(Envelope::new(keys))
}

/// Says where attachments are kept, and warns when that store cannot be opened, so the
/// first upload is not the first to know.
fn report_storage(attachments: &AttachmentService) {
    match attachments.blobs() {
        Ok(_) => info!("attachments are kept where attachments.storage says"),
        Err(error) => warn!(%error, "the blob store attachments.storage names cannot be opened"),
    }
}

/// Says whether mail can go, and warns when the settings set it up but something else it
/// needs is missing, such as a secret key.
fn report_mail(mail: &MailQueue, settings: &SettingsService) {
    match mail.status().problem {
        None => info!("mail goes out through mail.host"),
        Some(problem) if mail_set_up(&settings.current()) => {
            warn!(%problem, "mail is set up but cannot go");
        }
        Some(problem) => info!(%problem, "mail is off"),
    }
}

/// Makes the first admin from `admin.email` and `admin.password` while no admin can sign
/// in. Their values are never logged.
async fn ensure_first_admin(auth: &AuthService, admin: &AdminConfig) -> anyhow::Result<()> {
    let (Some(email), Some(password)) = (&admin.email, &admin.password) else {
        return Ok(());
    };
    let created = auth
        .ensure_admin(email, &Secret::new(password.expose()))
        .await
        .context("creating the first admin from admin.email and admin.password")?;
    match created {
        Some(user) => info!(user_id = %user.id, "first admin created from admin.email"),
        None => info!("an admin can sign in already; admin.email and admin.password are not used"),
    }
    Ok(())
}

/// Starts the hourly auth cleanup, and registers its stop and the wait for the auth work
/// begun after answers. Both stop before the pool closes, as they use it.
fn start_auth_upkeep(shutdown: &Shutdown, auth: &AuthService) {
    let settling = auth.clone();
    on_stop(
        shutdown,
        "auth background work",
        BACKGROUND_STOP + STOP_MARGIN,
        async move {
            if tokio::time::timeout(BACKGROUND_STOP, settling.settle())
                .await
                .is_err()
            {
                warn!("auth work begun after answers did not finish in time");
            }
        },
    );
    let cleanup = Cleanup::spawn(auth.clone(), CLEANUP_INTERVAL);
    on_stop(
        shutdown,
        "auth cleanup",
        DEFAULT_HOOK_TIMEOUT,
        cleanup.stop(),
    );
}

/// Starts the hourly cleanup of ended invitations and invite links, of the pages' trash, of
/// the versions the pages' history no longer keeps and of the attachments nothing shows,
/// which stops before the pool closes.
fn start_workspace_upkeep(
    shutdown: &Shutdown,
    workspaces: &WorkspaceService,
    pages: &PageService,
    history: &HistoryService,
    attachments: &AttachmentService,
) {
    let cleanup = WorkspaceCleanup::spawn(
        workspaces.clone(),
        pages.clone(),
        history.clone(),
        attachments.clone(),
        CLEANUP_INTERVAL,
    );
    on_stop(
        shutdown,
        "workspace cleanup",
        DEFAULT_HOOK_TIMEOUT,
        cleanup.stop(),
    );
}

/// Starts the hourly cleanup of OAuth's expired codes and tokens, spent grants and unused
/// clients, which stops before the pool closes.
fn start_oauth_upkeep(shutdown: &Shutdown, oauth: &OAuthService) {
    let cleanup = OAuthCleanup::spawn(oauth.clone(), CLEANUP_INTERVAL);
    on_stop(
        shutdown,
        "oauth cleanup",
        DEFAULT_HOOK_TIMEOUT,
        cleanup.stop(),
    );
}

/// Saves the live documents and closes their connections when stopping, after the HTTP
/// server stopped taking new ones and before the pool closes.
fn save_on_shutdown(shutdown: &Shutdown, live: &SyncService) {
    let live = live.clone();
    on_stop(
        shutdown,
        "live documents",
        SAVE_TIMEOUT + STOP_MARGIN,
        async move {
            if tokio::time::timeout(SAVE_TIMEOUT, live.shutdown())
                .await
                .is_err()
            {
                warn!("live documents were not all saved in time");
            }
        },
    );
}

/// Ends the MCP sessions first when stopping, so their event streams close and the HTTP
/// server's drain does not wait for them.
fn end_mcp_sessions_on_shutdown(shutdown: &Shutdown, mcp: Mcp) {
    on_stop(shutdown, "mcp sessions", DEFAULT_HOOK_TIMEOUT, async move {
        mcp.shutdown().await
    });
}

/// Closes the database feeds when stopping, after the HTTP server stopped taking new ones
/// and before the pool closes, so their clients reconnect to the next server.
fn close_feeds_on_shutdown(shutdown: &Shutdown, feeds: &LiveDatabases) {
    let feeds = feeds.clone();
    on_stop(
        shutdown,
        "database feeds",
        DEFAULT_HOOK_TIMEOUT,
        async move { feeds.shutdown().await },
    );
}

/// Starts mailing the digests that are due, every few minutes, which stops before the pool
/// closes.
fn start_notification_upkeep(shutdown: &Shutdown, inbox: &InboxService) {
    let inbox = inbox.clone();
    let digests = Periodic::spawn("notification digests", DIGEST_CHECK_INTERVAL, move || {
        let inbox = inbox.clone();
        async move { inbox.send_digests().await.map(|_| ()) }
    });
    on_stop(
        shutdown,
        "notification digests",
        DEFAULT_HOOK_TIMEOUT,
        digests.stop(),
    );
}

/// Closes the inbox feeds when stopping, after the HTTP server stopped taking new ones and
/// before the pool closes, so their clients reconnect to the next server.
fn close_inbox_feeds_on_shutdown(shutdown: &Shutdown, feeds: &InboxFeeds) {
    let feeds = feeds.clone();
    on_stop(shutdown, "inbox feeds", DEFAULT_HOOK_TIMEOUT, async move {
        feeds.shutdown().await
    });
}

/// Closes the pool when stopping, after every part registered later, which all use it.
fn close_on_shutdown(shutdown: &Shutdown, pool: PgPool) {
    on_stop(
        shutdown,
        "database pool",
        DEFAULT_HOOK_TIMEOUT,
        async move {
            pool.close().await;
            info!("database pool closed");
        },
    );
}

/// Starts the parts that run on the pool, in the reverse of the order they stop in: the
/// gauges, the job worker, then the HTTP server, which stops first so no request is cut
/// off by the parts behind it. The gauges are sampled only when `sample_gauges` is set,
/// since with metrics off nothing records them.
async fn start_parts(
    shutdown: &Shutdown,
    pool: &PgPool,
    sample_gauges: bool,
    worker: Worker,
    listener: TcpListener,
    router: Router,
) -> anyhow::Result<()> {
    if sample_gauges {
        let gauges = Gauges::spawn(pool.clone(), gauges::INTERVAL);
        on_stop(shutdown, "gauges", DEFAULT_HOOK_TIMEOUT, gauges.stop());
    }

    let worker = worker.start().await.context("starting the job worker")?;
    let bound = worker.stop_timeout() + STOP_MARGIN;
    on_stop(shutdown, "job worker", bound, worker.stop());

    let http = Http::spawn(listener, router, HEADER_READ_TIMEOUT)?;
    on_stop(
        shutdown,
        "http",
        DRAIN_TIMEOUT + STOP_MARGIN,
        http.stop(DRAIN_TIMEOUT),
    );
    Ok(())
}

/// Registers a stop that logs its own problems instead of failing.
fn on_stop(
    shutdown: &Shutdown,
    name: &'static str,
    timeout: Duration,
    stop: impl Future<Output = ()> + Send + 'static,
) {
    shutdown.on_shutdown_with_timeout(name, timeout, move || async move {
        stop.await;
        Ok::<(), Infallible>(())
    });
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

async fn load_settings(
    pool: &PgPool,
    pinned: Pinned,
    envelope: Envelope,
) -> anyhow::Result<Arc<SettingsService>> {
    let (settings, warnings) = SettingsService::load(pool.clone(), registry(), pinned, envelope)
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

type Connection = http1::UpgradeableConnection<TokioIo<TcpStream>, TowerToHyperService<WithPeer>>;

/// The router, with each request told the address of the connection it came on, for the
/// client address rate limits count.
#[derive(Clone)]
struct WithPeer {
    router: Router,
    peer: SocketAddr,
}

impl tower::Service<hyper::Request<Incoming>> for WithPeer {
    type Response = <Router as tower::Service<hyper::Request<Incoming>>>::Response;
    type Error = Infallible;
    type Future = <Router as tower::Service<hyper::Request<Incoming>>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        tower::Service::<hyper::Request<Incoming>>::poll_ready(&mut self.router, cx)
    }

    fn call(&mut self, mut request: hyper::Request<Incoming>) -> Self::Future {
        request.extensions_mut().insert(ConnectInfo(self.peer));
        self.router.call(request)
    }
}

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
                Ok((stream, peer)) => {
                    let service = TowerToHyperService::new(WithPeer {
                        router: router.clone(),
                        peer,
                    });
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
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use async_trait::async_trait;
    use axum::routing::get;
    use neon_app::jobs::{Handler, Job, JobError, NewJob};
    use neon_observability::config::AdminConfig;
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
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

    /// Sends `GET path` on a connection of its own. `None` when the connection is refused.
    async fn status_of(addr: SocketAddr, path: &str) -> Option<(u16, String)> {
        let request = format!("GET {path} HTTP/1.1\r\nHost: neon\r\nConnection: close\r\n\r\n");
        let mut stream = TcpStream::connect(addr).await.ok()?;
        stream.write_all(request.as_bytes()).await.ok()?;
        let response = read_until_closed(&mut stream).await;
        let status = response.split(' ').nth(1)?.parse().ok()?;
        let (_, body) = response.split_once("\r\n\r\n")?;
        Some((status, body.to_owned()))
    }

    async fn job_status(pool: &PgPool, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM jobs WHERE id::text = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("the job exists")
    }

    const KIND: &str = "test.slow";

    /// Takes a moment over each job.
    struct SlowJob;

    #[async_trait]
    impl Handler for SlowJob {
        fn kind(&self) -> &'static str {
            KIND
        }

        async fn run(&self, _: &Job) -> Result<(), JobError> {
            sleep(Duration::from_millis(200)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn readiness_fails_while_the_database_is_unreachable() {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://neon@127.0.0.1:1/neon")
            .expect("a valid URL");

        let error = ping(&pool).await.expect_err("nothing listens on port 1");

        assert!(error.starts_with("the database "), "{error}");
    }

    #[sqlx::test(migrator = "neon_store::MIGRATOR")]
    async fn stopping_turns_traffic_away_then_stops_http_the_worker_and_the_pool_in_order(
        pool: PgPool,
    ) {
        let telemetry = Telemetry::noop();
        let shutdown = telemetry.shutdown();
        // Stands in for the telemetry's flushes, which are registered first and so run last.
        let pool_closed_at_flush = Arc::new(Mutex::new(None));
        shutdown.on_shutdown("telemetry", {
            let seen = Arc::clone(&pool_closed_at_flush);
            let pool = pool.clone();
            move || async move {
                *seen.lock().unwrap() = Some(pool.is_closed());
                Ok::<(), Infallible>(())
            }
        });
        let started = Arc::new(OnceLock::new());
        let admin_config = AdminConfig {
            enabled: true,
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        };
        let admin = admin::spawn(&admin_config, &telemetry, readiness(Arc::clone(&started)))
            .await
            .expect("the admin listener starts")
            .expect("it is enabled");
        assert_eq!(
            status_of(admin, "/readyz").await,
            Some((503, "starting".to_owned()))
        );

        // Once stopping has begun, the request queues a job and answers when a worker has
        // claimed it, so it only succeeds if the worker still runs while requests drain.
        let request_started = Arc::new(Notify::new());
        let proceed = Arc::new(Notify::new());
        let router = Router::new().route(
            "/slow",
            get({
                let request_started = Arc::clone(&request_started);
                let proceed = Arc::clone(&proceed);
                let pool = pool.clone();
                move || async move {
                    request_started.notify_one();
                    proceed.notified().await;
                    let id = jobs::enqueue(&pool, &NewJob::new(KIND, json!({})))
                        .await
                        .expect("queued")
                        .to_string();
                    let deadline = Instant::now() + PATIENCE;
                    while job_status(&pool, &id).await != "running" {
                        assert!(Instant::now() < deadline, "no worker claimed the job");
                        sleep(Duration::from_millis(20)).await;
                    }
                    id
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().expect("bound");
        let worker = Worker::new(
            pool.clone(),
            WorkerConfig {
                poll_interval: Duration::from_millis(50),
                stop_timeout: PATIENCE,
                ..WorkerConfig::default()
            },
        )
        .handler(SlowJob);
        close_on_shutdown(shutdown, pool.clone());
        start_parts(shutdown, &pool, true, worker, listener, router)
            .await
            .expect("the parts start");
        started.get_or_init(|| pool.clone());
        assert_eq!(
            status_of(admin, "/readyz").await,
            Some((200, "ok".to_owned()))
        );

        let mut client = send(addr, "GET /slow HTTP/1.1\r\nHost: neon\r\n\r\n").await;
        request_started.notified().await;
        let stopping = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.run().await }
        });

        // Traffic is turned away first, while the open request still runs.
        let deadline = Instant::now() + PATIENCE;
        while status_of(admin, "/readyz").await != Some((503, "shutting down".to_owned())) {
            assert!(Instant::now() < deadline, "/readyz still reports ready");
            sleep(Duration::from_millis(20)).await;
        }
        assert!(refuses_connections(addr).await);
        proceed.notify_one();

        let response = read_until_closed(&mut client).await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        let (_, job) = response.split_once("\r\n\r\n").expect("a body");
        timeout(PATIENCE * 2, stopping)
            .await
            .expect("stopping ends")
            .expect("the stop task ran")
            .expect("every part stopped");

        // The worker finished the job it claimed during the drain and recorded it as done,
        // so the pool was still open while the worker stopped.
        let check = PgPoolOptions::new()
            .max_connections(1)
            .connect_with((*pool.connect_options()).clone())
            .await
            .expect("the test database is reachable");
        assert_eq!(job_status(&check, job).await, "done");
        check.close().await;
        // The pool closed before the telemetry flushed, and the admin listener is gone.
        assert_eq!(*pool_closed_at_flush.lock().unwrap(), Some(true));
        assert_eq!(status_of(admin, "/readyz").await, None);
    }
}
