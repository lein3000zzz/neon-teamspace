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
//!   may concern, at once: it closes those that lost the page, makes read-only those that
//!   lost edit access, and lets edit again those that got it back.
//! - **Saving.** Updates applied wait briefly ([`Limits::persist_delay`]) and are logged
//!   together, with their authors, in order. Once enough are logged, and when the document
//!   unloads or the server stops, the whole document is saved as a snapshot in one
//!   transaction that deletes the updates it takes in and records the page's search text,
//!   from the document's blocks, and its latest change.
//! - **Limits** bound messages, documents, each connection's rate and its queue: a client
//!   that falls behind is disconnected rather than holding up the others. See [`Limits`].
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
use tracing::info;
use uuid::Uuid;

use crate::clock::Clock;
use crate::pages::{PageError, PageService};
use document::{Command, Joining};
use limits::Bucket;

pub use access::{AccessFeed, AccessListener, access_feed};
pub use broadcast::{Broadcaster, ClientId, LocalBroadcaster, Next, Outbox, OutboxReceiver};
pub use close::CloseReason;
pub use document::READ_ONLY;
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
    broadcaster: Arc<dyn Broadcaster>,
    limits: Limits,
    clock: Clock,
    red: Red,
    /// Cancelled as the server stops.
    stop: CancellationToken,
    tasks: TaskTracker,
    docs: Mutex<HashMap<Uuid, Slot>>,
    epoch: Arc<AtomicU64>,
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

/// Puts a [`SyncService`] together. Without being told otherwise it serves one process's
/// clients, with the default limits, on the system clock.
pub struct SyncServiceBuilder {
    pool: PgPool,
    pages: PageService,
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
    /// installed, as it creates the metrics.
    pub fn build(self) -> SyncService {
        describe();
        let inner = Arc::new(Inner {
            pool: self.pool,
            pages: self.pages,
            broadcaster: self.broadcaster,
            limits: self.limits,
            clock: self.clock,
            red: Red::new(SUBSYSTEM),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            docs: Mutex::new(HashMap::new()),
            epoch: self.feed.epoch,
        });
        inner
            .tasks
            .spawn(access::watch(Arc::clone(&inner), self.feed.changes));
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
    pub fn builder(pool: PgPool, pages: PageService, feed: AccessFeed) -> SyncServiceBuilder {
        SyncServiceBuilder {
            pool,
            pages,
            feed,
            broadcaster: Arc::new(LocalBroadcaster::new()),
            limits: Limits::default(),
            clock: Clock::system(),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.limits
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
        let (outbox, receiver) = Outbox::new(self.inner.limits.client_queue);
        let mut attempt = 0;
        let (commands, generation) = loop {
            attempt += 1;
            let (commands, generation) = self.slot(&admission, id)?;
            let (done, answer) = oneshot::channel();
            let joining = Joining {
                id,
                user_id: admission.principal.user.id,
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

    /// Stops: every live document closes its connections and is saved, then the access
    /// watcher stops. Returns once all are done.
    pub async fn shutdown(&self) {
        let live = self.inner.docs().len();
        info!(documents = live, "saving the live documents");
        self.inner.stop.cancel();
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
        info!("live documents saved");
    }

    /// The page's live document, started if there is none, with the connection noted in
    /// it for the access watcher.
    fn slot(
        &self,
        admission: &Admission,
        client: ClientId,
    ) -> Result<(mpsc::Sender<Command>, u64), CloseReason> {
        let mut docs = self.inner.docs();
        if self.inner.stop.is_cancelled() {
            return Err(CloseReason::Restarting);
        }
        let slot = match docs.entry(admission.page_id) {
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
    client: ClientId,
    principal: Principal,
    commands: mpsc::Sender<Command>,
}

impl Inner {
    fn docs(&self) -> MutexGuard<'_, HashMap<Uuid, Slot>> {
        self.docs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a page's document out of the registry, if it is still the one of
    /// `generation`.
    fn forget(&self, page_id: Uuid, generation: u64) {
        Self::remove(&mut self.docs(), page_id, generation);
    }

    fn remove(docs: &mut HashMap<Uuid, Slot>, page_id: Uuid, generation: u64) {
        if docs
            .get(&page_id)
            .is_some_and(|slot| slot.generation == generation)
        {
            docs.remove(&page_id);
        }
    }

    /// Takes a connection out of its document's slot.
    fn unregister(&self, page_id: Uuid, generation: u64, client: ClientId) {
        if let Some(slot) = self.docs().get_mut(&page_id)
            && slot.generation == generation
        {
            slot.connections.remove(&client);
        }
    }

    /// The open connections any of `changes` may concern.
    fn concerned(&self, changes: &[crate::pages::AccessChange]) -> Vec<Recheck> {
        let docs = self.docs();
        let mut concerned = Vec::new();
        for (page_id, slot) in docs.iter() {
            for (client, principal) in &slot.connections {
                if changes
                    .iter()
                    .any(|change| change.concerns(slot.workspace_id, principal.user.id))
                {
                    concerned.push(Recheck {
                        page_id: *page_id,
                        client: *client,
                        principal: principal.clone(),
                        commands: slot.commands.clone(),
                    });
                }
            }
        }
        concerned
    }

    /// Asks the rules again what a connection's principal may do with its page, and tells
    /// its document.
    async fn recheck(&self, connection: Recheck) -> Result<(), PageError> {
        let access = match self
            .pages
            .document_access(&connection.principal, connection.page_id)
            .await
        {
            Ok(view) => Some(view.access),
            Err(error) if error.is_expected() => None,
            Err(error) => return Err(error),
        };
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
