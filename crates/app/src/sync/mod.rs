//! Real-time sync: the live documents of open pages, the y-sync protocol their connections
//! speak, saving them, and closing the connections of people who lost access.
//!
//! - **Live documents.** The first connection to a page loads its Yjs document, its last
//!   snapshot with the updates logged after it, into a task of its own that owns the
//!   document and its awareness: cursors and presence. Every connection to the page talks
//!   to that task, which applies what they send in turn and hands what everyone must hear
//!   to the [`Broadcaster`]. Once the last connection leaves, the document stays a while
//!   ([`Limits::idle_unload`]), then is saved and unloads.
//! - **The protocol** is y-websocket's: y-protocols' sync step 1, sync step 2 and update
//!   messages, and awareness messages, each in a binary WebSocket message. The server
//!   starts with its sync step 1 and the awareness states it has, answers a client's sync
//!   step 1 with sync step 2, and sends every update it applies to every other client, and
//!   every awareness change to every client.
//! - **Access** is the rules': [`SyncService::admit`] asks the page service what the
//!   principal may do with the page's document before the connection opens. One that may
//!   only view gets every update but may send none that changes the document; one that
//!   does is closed with [`CloseReason::ReadOnly`]. When access changes, the
//!   [`AccessListener`] tells the service, which looks again at each connection the change
//!   may concern, at once: it checks that the session or token the connection opened with
//!   still holds, closing it with [`CloseReason::SignedOut`] when not, then closes those
//!   that lost the page, makes read-only those that lost edit access, and lets edit again
//!   those that got it back. Every connection is looked at so again every
//!   [`Limits::revalidate_interval`], for what no change tells of, such as a session
//!   expiring.
//! - **Saving.** Updates applied wait briefly ([`Limits::persist_delay`]) and are logged
//!   together, with their authors, in order. Once enough are logged, and when the document
//!   unloads or the server stops, the whole document is saved as a snapshot in one
//!   transaction that deletes the updates it takes in and records the page's search text,
//!   from the document's blocks, and its latest change.
//! - **Limits** bound messages, documents, awareness states, each connection's rates and its
//!   queue, and how many connections a person and a page may have: a client that falls
//!   behind is disconnected rather than holding up the others. See [`Limits`].
//!
//! The connection's transport, the WebSocket with its pings, is the server's: it hands each
//! message to its [`Connection`] and sends what the connection gives back.
//!
//! Loads, persists and compactions are RED metrics of [`SUBSYSTEM`], and gauges count the
//! live documents and open connections. Nothing logs a document's content.

mod access;
mod broadcast;
mod close;
mod document;
mod limits;
mod protocol;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use metrics::Unit;
use neon_domain::auth::Principal;
use neon_domain::pages::PageAccess;
use neon_observability::red::Red;
use sqlx::PgPool;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};
use uuid::Uuid;
use yrs::Transact as _;

use crate::auth::AuthService;
use crate::clock::Clock;
use crate::pages::{PageError, PageService};
use document::{Command, Joining, Presence};
use limits::Bucket;

pub use access::{AccessFeed, AccessListener, access_feed};
pub use broadcast::{Broadcaster, ClientId, LocalBroadcaster, Next, Outbox, OutboxReceiver};
pub use close::CloseReason;
pub use document::{LoadError, READ_ONLY};
pub use limits::Limits;

/// RED subsystem of the sync's work: loading, logging and compacting documents.
pub const SUBSYSTEM: &str = "sync";

/// Gauges, and the metrics of what the connections carry.
const LIVE_DOCUMENTS: &str = "neon_sync_live_documents";
const OPEN_CONNECTIONS: &str = "neon_sync_open_connections";
const MESSAGE_SIZE: &str = "neon_sync_message_size_bytes";
const MESSAGES: &str = "neon_sync_messages_total";
const DISCONNECTS: &str = "neon_sync_disconnects_total";

/// How many commands may wait for a live document. A connection whose message does not
/// fit waits, and with it reading from its socket.
const COMMAND_QUEUE: usize = 256;

/// How often a connection tries to open a document that keeps unloading under it.
const JOIN_ATTEMPTS: usize = 5;

/// The live documents. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct SyncService {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pool: PgPool,
    pages: PageService,
    auth: AuthService,
    broadcaster: Arc<dyn Broadcaster>,
    limits: Limits,
    clock: Clock,
    red: Red,
    /// Cancelled as the server stops.
    stop: CancellationToken,
    tasks: TaskTracker,
    registry: Mutex<Registry>,
    epoch: Arc<AtomicU64>,
}

/// The live documents, and how many connections each person has open.
#[derive(Default)]
struct Registry {
    docs: HashMap<Uuid, Slot>,
    users: HashMap<Uuid, usize>,
}

/// A live document in the registry.
struct Slot {
    /// Which task holds it: a page unloaded and opened again gets a new one.
    generation: u64,
    workspace_id: Uuid,
    commands: mpsc::Sender<Command>,
    /// Its connections, and whom each acts for, for the access watcher.
    connections: HashMap<ClientId, Principal>,
}

impl Registry {
    /// Whether `user_id` may open one more connection, to `page_id`.
    fn has_room(&self, limits: &Limits, user_id: Uuid, page_id: Uuid) -> bool {
        let theirs = self.users.get(&user_id).copied().unwrap_or(0);
        let page = self
            .docs
            .get(&page_id)
            .map_or(0, |slot| slot.connections.len());
        theirs < limits.connections_per_user && page < limits.connections_per_page
    }

    /// Takes a connection out of its document's slot, if it is in it.
    fn unregister(&mut self, page_id: Uuid, generation: u64, client: ClientId) {
        let taken = self
            .docs
            .get_mut(&page_id)
            .filter(|slot| slot.generation == generation)
            .and_then(|slot| slot.connections.remove(&client));
        if let Some(principal) = taken {
            self.forget_one(principal.user.id);
        }
    }

    /// Takes a page's document out, if it is still the one of `generation`, with any
    /// connection still noted in it.
    fn remove(&mut self, page_id: Uuid, generation: u64) {
        if self
            .docs
            .get(&page_id)
            .is_some_and(|slot| slot.generation == generation)
            && let Some(slot) = self.docs.remove(&page_id)
        {
            for principal in slot.connections.into_values() {
                self.forget_one(principal.user.id);
            }
        }
    }

    fn forget_one(&mut self, user_id: Uuid) {
        if let Entry::Occupied(mut count) = self.users.entry(user_id) {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
            }
        }
    }
}

/// Puts a [`SyncService`] together. Without being told otherwise it serves one process's
/// clients, with the default limits, on the system clock.
pub struct SyncServiceBuilder {
    pool: PgPool,
    pages: PageService,
    auth: AuthService,
    feed: AccessFeed,
    broadcaster: Arc<dyn Broadcaster>,
    limits: Limits,
    clock: Clock,
}

impl SyncServiceBuilder {
    pub fn broadcaster(mut self, broadcaster: Arc<dyn Broadcaster>) -> Self {
        self.broadcaster = broadcaster;
        self
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Starts the access watcher, so call it inside the runtime, after telemetry is
    /// installed, as it creates the metrics. The watcher holds the service weakly: once
    /// every clone of the service and every live document is gone, it stops too.
    pub fn build(self) -> SyncService {
        describe();
        let inner = Arc::new(Inner {
            pool: self.pool,
            pages: self.pages,
            auth: self.auth,
            broadcaster: self.broadcaster,
            limits: self.limits,
            clock: self.clock,
            red: Red::new(SUBSYSTEM),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            registry: Mutex::new(Registry::default()),
            epoch: self.feed.epoch,
        });
        inner.tasks.spawn(access::watch(
            Arc::downgrade(&inner),
            inner.stop.clone(),
            inner.limits.revalidate_interval,
            self.feed.changes,
        ));
        SyncService { inner }
    }
}

fn describe() {
    metrics::describe_gauge!(
        LIVE_DOCUMENTS,
        Unit::Count,
        "Pages' documents held in memory, open or about to unload."
    );
    metrics::describe_gauge!(
        OPEN_CONNECTIONS,
        Unit::Count,
        "Open WebSocket connections to live documents."
    );
    metrics::describe_histogram!(
        MESSAGE_SIZE,
        Unit::Bytes,
        "Sizes of the messages connections carry, by direction: in from clients, out to them."
    );
    metrics::describe_counter!(
        MESSAGES,
        Unit::Count,
        "y-sync and awareness messages clients sent, by kind."
    );
    metrics::describe_counter!(
        DISCONNECTS,
        Unit::Count,
        "Connections ended, by cause: the client, an unresponsive client, or the server's close reason."
    );
    // Every cause from the start, so rates of those that never happened read as zero.
    let causes = CloseReason::ALL
        .map(Ending::Closed)
        .into_iter()
        .chain([Ending::ByClient, Ending::Unresponsive]);
    for ending in causes {
        metrics::counter!(DISCONNECTS, "cause" => ending.cause()).absolute(0);
    }
}

/// A principal the rules let open a page's document, with what they may do there.
#[derive(Debug, Clone)]
pub struct Admission {
    principal: Principal,
    page_id: Uuid,
    workspace_id: Uuid,
    access: PageAccess,
    /// How many access changes were heard before the rules were asked.
    epoch: u64,
}

impl Admission {
    pub fn access(&self) -> PageAccess {
        self.access
    }
}

impl SyncService {
    /// The live documents of `pages`, whose connections' sessions and tokens `auth` checks
    /// again as access changes, and whose changes `feed` brings.
    pub fn builder(
        pool: PgPool,
        pages: PageService,
        auth: AuthService,
        feed: AccessFeed,
    ) -> SyncServiceBuilder {
        SyncServiceBuilder {
            pool,
            pages,
            auth,
            feed,
            broadcaster: Arc::new(LocalBroadcaster::new()),
            limits: Limits::default(),
            clock: Clock::system(),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Whether the principal may open one more connection to the page, as
    /// [`Limits::connections_per_user`] and [`Limits::connections_per_page`] allow. Ask it
    /// before the upgrade; [`connect`](Self::connect) holds to the same limits.
    pub fn has_room(&self, admission: &Admission) -> bool {
        self.inner.registry().has_room(
            &self.inner.limits,
            admission.principal.user.id,
            admission.page_id,
        )
    }

    /// Asks the rules whether the principal may open the page's document, and whether to
    /// edit it or only view it. A page they may not see is not found.
    pub async fn admit(&self, principal: Principal, page_id: Uuid) -> Result<Admission, PageError> {
        let epoch = self.inner.epoch.load(Ordering::SeqCst);
        let view = self
            .inner
            .pages
            .document_access(&principal, page_id)
            .await?;
        Ok(Admission {
            workspace_id: view.page.workspace_id,
            access: view.access,
            principal,
            page_id,
            epoch,
        })
    }

    /// Opens the page's document for an admitted principal, loading it if nobody has it
    /// open. The connection's client gets the server's sync step 1 first.
    pub async fn connect(&self, admission: Admission) -> Result<Connection, CloseReason> {
        let id = ClientId::next();
        let limits = &self.inner.limits;
        let (outbox, receiver) = Outbox::new(limits.client_queue, limits.client_queue_bytes);
        let mut attempt = 0;
        let (commands, generation) = loop {
            attempt += 1;
            let (commands, generation) = self.slot(&admission, id)?;
            let (done, answer) = oneshot::channel();
            let user = &admission.principal.user;
            let joining = Joining {
                id,
                user_id: user.id,
                presence: Presence {
                    id: user.id,
                    name: user.display_name.clone(),
                    hue: user.hue(),
                },
                access: admission.access,
                outbox: outbox.clone(),
            };
            let answered = match commands
                .send(Command::Join {
                    client: joining,
                    done,
                })
                .await
            {
                Ok(()) => answer.await.ok(),
                Err(_) => None,
            };
            match answered {
                Some(Ok(())) => break (commands, generation),
                Some(Err(reason)) => {
                    self.inner.unregister(admission.page_id, generation, id);
                    return Err(reason);
                }
                // The document unloaded under the connection: open it again.
                None if attempt < JOIN_ATTEMPTS => {
                    self.inner.unregister(admission.page_id, generation, id);
                }
                None => {
                    self.inner.unregister(admission.page_id, generation, id);
                    return Err(CloseReason::ServerError);
                }
            }
        };
        metrics::gauge!(OPEN_CONNECTIONS).increment(1.0);
        let limits = &self.inner.limits;
        let connection = Connection {
            id,
            page_id: admission.page_id,
            generation,
            commands: commands.clone(),
            outbox: receiver,
            bucket: Bucket::new(limits.messages_per_second, limits.message_burst),
            inner: Arc::clone(&self.inner),
            left: false,
        };
        if self.inner.epoch.load(Ordering::SeqCst) != admission.epoch {
            // A change may have committed after the rules were asked, and before the
            // watcher could see this connection.
            let recheck = Recheck {
                page_id: admission.page_id,
                generation,
                client: id,
                principal: admission.principal,
                commands,
            };
            if self.inner.recheck(recheck).await.is_err() {
                outbox.close(CloseReason::ServerError);
            }
        }
        Ok(connection)
    }

    /// A page's document as blocks: the live document's, with what was sent to it and is not
    /// saved yet, while the page is open; otherwise the one saved, its snapshot with the
    /// updates logged after it. It opens no live document. A page without a document has
    /// an empty one. Blocks that do not read, which the editor would not show either, are
    /// left out, as [`neon_blocks::read_lenient`] reads, and logged by count. Whether the
    /// caller may read it is not asked here.
    pub async fn content(&self, page_id: Uuid) -> Result<neon_blocks::Document, LoadError> {
        let live = self
            .inner
            .registry()
            .docs
            .get(&page_id)
            .map(|slot| slot.commands.clone());
        if let Some(commands) = live {
            let (done, answer) = oneshot::channel();
            if commands.send(Command::Read { done }).await.is_ok()
                && let Ok(read) = answer.await
            {
                return Ok(readable(page_id, read));
            }
            // It unloaded meanwhile, and saved everything first.
        }
        let saved = neon_store::documents::saved(&self.inner.pool, page_id).await?;
        let doc = document::assemble(saved.snapshot.as_ref(), &saved.updates)?;
        let fragment = neon_blocks::fragment(&doc);
        let read = neon_blocks::read_lenient(&doc.transact(), &fragment);
        Ok(readable(page_id, read))
    }

    /// Stops: every live document closes its connections and is saved, then the access
    /// watcher stops. Returns once all are done.
    pub async fn shutdown(&self) {
        let live = self.inner.registry().docs.len();
        info!(documents = live, "saving the live documents");
        self.inner.stop.cancel();
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
        info!("live documents saved");
    }

    /// The page's live document, started if there is none, with the connection noted in
    /// it for the access watcher, when the limits of connections leave room for it.
    fn slot(
        &self,
        admission: &Admission,
        client: ClientId,
    ) -> Result<(mpsc::Sender<Command>, u64), CloseReason> {
        let mut registry = self.inner.registry();
        if self.inner.stop.is_cancelled() {
            return Err(CloseReason::Restarting);
        }
        let user_id = admission.principal.user.id;
        if !registry.has_room(&self.inner.limits, user_id, admission.page_id) {
            return Err(CloseReason::TooManyConnections);
        }
        *registry.users.entry(user_id).or_default() += 1;
        let slot = match registry.docs.entry(admission.page_id) {
            Entry::Occupied(slot) => slot.into_mut(),
            Entry::Vacant(vacant) => {
                static GENERATION: AtomicU64 = AtomicU64::new(1);
                let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
                let (commands, received) = mpsc::channel(COMMAND_QUEUE);
                self.inner.tasks.spawn(document::run(
                    Arc::clone(&self.inner),
                    admission.page_id,
                    generation,
                    received,
                ));
                vacant.insert(Slot {
                    generation,
                    workspace_id: admission.workspace_id,
                    commands,
                    connections: HashMap::new(),
                })
            }
        };
        slot.connections.insert(client, admission.principal.clone());
        Ok((slot.commands.clone(), slot.generation))
    }
}

/// A connection the access watcher looks at again.
struct Recheck {
    page_id: Uuid,
    generation: u64,
    client: ClientId,
    principal: Principal,
    commands: mpsc::Sender<Command>,
}

impl Inner {
    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a page's document out of the registry, if it is still the one of
    /// `generation`.
    fn forget(&self, page_id: Uuid, generation: u64) {
        self.registry().remove(page_id, generation);
    }

    /// Takes a connection out of its document's slot.
    fn unregister(&self, page_id: Uuid, generation: u64, client: ClientId) {
        self.registry().unregister(page_id, generation, client);
    }

    /// The open connections whose person and workspace `keep` says to look at again.
    fn connections(&self, mut keep: impl FnMut(Uuid, &Principal) -> bool) -> Vec<Recheck> {
        let registry = self.registry();
        let mut found = Vec::new();
        for (page_id, slot) in &registry.docs {
            for (client, principal) in &slot.connections {
                if keep(slot.workspace_id, principal) {
                    found.push(Recheck {
                        page_id: *page_id,
                        generation: slot.generation,
                        client: *client,
                        principal: principal.clone(),
                        commands: slot.commands.clone(),
                    });
                }
            }
        }
        found
    }

    /// Checks again that a connection's session or token holds, then asks the rules what
    /// its principal may do with its page now, and tells its document.
    async fn recheck(&self, connection: Recheck) -> Result<(), RecheckError> {
        let principal = match self.auth.refresh(&connection.principal).await {
            Ok(principal) => principal,
            Err(crate::auth::AuthError::Unauthenticated) => {
                let _ = connection
                    .commands
                    .send(Command::Close {
                        client: connection.client,
                        reason: CloseReason::SignedOut,
                    })
                    .await;
                return Ok(());
            }
            Err(error) => return Err(RecheckError::Auth(error)),
        };
        let access = match self
            .pages
            .document_access(&principal, connection.page_id)
            .await
        {
            Ok(view) => Some(view.access),
            Err(error) if error.is_expected() => None,
            Err(error) => return Err(RecheckError::Page(error)),
        };
        // The next look starts from what holds now: the account's role, the token's scopes.
        if let Some(slot) = self.registry().docs.get_mut(&connection.page_id)
            && slot.generation == connection.generation
            && let Some(held) = slot.connections.get_mut(&connection.client)
        {
            *held = principal;
        }
        // A document that is gone has closed its connections already.
        let _ = connection
            .commands
            .send(Command::Access {
                client: connection.client,
                access,
            })
            .await;
        Ok(())
    }
}

/// Why a connection could not be looked at again: it keeps what it had until the next look.
#[derive(Debug, thiserror::Error)]
enum RecheckError {
    #[error(transparent)]
    Auth(#[from] crate::auth::AuthError),
    #[error(transparent)]
    Page(#[from] PageError),
}

/// The blocks of a lenient read, telling the log how many were left out, never what.
fn readable(page_id: Uuid, read: neon_blocks::Lenient) -> neon_blocks::Document {
    if read.dropped > 0 {
        warn!(%page_id, blocks = read.dropped, "blocks of a document do not read; left out");
    }
    read.document
}

/// How a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// The server closed it, for this reason.
    Closed(CloseReason),
    /// The client closed it, or went away.
    ByClient,
    /// The client stopped answering pings.
    Unresponsive,
}

impl Ending {
    /// A short name, for logs and metric labels: the close reason's, `client` or
    /// `unresponsive`.
    pub fn cause(self) -> &'static str {
        match self {
            Self::Closed(reason) => reason.name(),
            Self::ByClient => "client",
            Self::Unresponsive => "unresponsive",
        }
    }
}

/// One client's connection to a live document, for its transport: what the client sends
/// goes in by [`receive`](Self::receive), and [`next`](Self::next) says what to send it or
/// when to close. End it with [`leave`](Self::leave).
pub struct Connection {
    id: ClientId,
    page_id: Uuid,
    generation: u64,
    commands: mpsc::Sender<Command>,
    outbox: OutboxReceiver,
    bucket: Bucket,
    inner: Arc<Inner>,
    left: bool,
}

impl Connection {
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// Hands a message from the client to the document, in order. It waits while the
    /// document is busy. An error is why the connection must close now.
    pub async fn receive(&mut self, message: Bytes) -> Result<(), CloseReason> {
        metrics::histogram!(MESSAGE_SIZE, "direction" => "in").record(message.len() as f64);
        if let Some(reason) = self.outbox.closed() {
            return Err(reason);
        }
        if message.len() > self.inner.limits.max_message {
            return Err(CloseReason::MessageTooBig);
        }
        if !self.bucket.take() {
            return Err(CloseReason::TooManyMessages);
        }
        let sent = self
            .commands
            .send(Command::Message {
                client: self.id,
                message,
            })
            .await;
        sent.map_err(|_| self.outbox.closed().unwrap_or(CloseReason::ServerError))
    }

    /// The next message for the client, or why the connection closes.
    pub async fn next(&mut self) -> Next {
        let next = self.outbox.next().await;
        if let Next::Send(message) = &next {
            metrics::histogram!(MESSAGE_SIZE, "direction" => "out").record(message.len() as f64);
        }
        next
    }

    /// Takes the connection out of its document once its socket closed, which the
    /// document tells the others. `ending` is how it ended, for the metrics.
    pub async fn leave(mut self, ending: Ending) {
        self.left = true;
        metrics::counter!(DISCONNECTS, "cause" => ending.cause()).increment(1);
        // A document that is gone has nobody to tell.
        let _ = self.commands.send(Command::Leave { client: self.id }).await;
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.left {
            // Dropped without leaving, as when its task was cancelled: the document must
            // still hear, after anything the connection sent before.
            match self.commands.try_send(Command::Leave { client: self.id }) {
                Ok(()) | Err(TrySendError::Closed(_)) => {}
                Err(TrySendError::Full(left)) => {
                    let commands = self.commands.clone();
                    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                        runtime.spawn(async move {
                            let _ = commands.send(left).await;
                        });
                    }
                }
            }
        }
        self.inner
            .unregister(self.page_id, self.generation, self.id);
        metrics::gauge!(OPEN_CONNECTIONS).decrement(1.0);
    }
}
