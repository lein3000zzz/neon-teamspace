//! Following a database live: the connections that hear its changes, and closing or
//! downgrading those of people who lost access.
//!
//! - **Admission** is the rules': [`LiveDatabases::admit`] asks what the principal may do
//!   with the database before the connection opens. One they may not see is not found.
//! - **Events** are the JSON messages [`events`](super::events) lists. A connection joins
//!   its database's room of the [`Broadcaster`], or the guests' room for a guest, and is
//!   sent `ready` once it hears every change; nothing it missed before is sent again, so a
//!   client reads the database afresh when it connects.
//! - **Access.** When access changes, the [`AccessFeed`] tells the watcher, which looks again
//!   at each connection the change may concern: it checks that the session or token the
//!   connection opened with still holds, closing it as signed out when not, closes those
//!   that lost the database, and tells those whose access changed with an `access` message,
//!   moving one between the members' and guests' rooms when their role changed. Every
//!   [`LiveLimits::revalidate_interval`] it looks so at every connection.
//! - **Limits** bound how many connections a person and a database may have, each
//!   connection's queue, and what a client may send: nothing is expected of it, and what
//!   it sends is read and dropped, within a small size and rate.
//!
//! Gauges count the open connections, and counters the events told and the connections
//! ended, by cause.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures_util::StreamExt as _;
use metrics::Unit;
use neon_domain::auth::{Principal, Scope};
use neon_domain::pages::PageAccess;
use neon_domain::workspaces::Action;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::events::access_message;
use super::{DatabaseError, DatabaseService};
use crate::auth::{AuthError, AuthService};
use crate::pages::AccessChange;
use crate::sync::{
    AccessFeed, Broadcaster, Bucket, ClientId, CloseReason, Ending, Next, Outbox, OutboxReceiver,
    Room,
};

/// Gauges and counters of the feeds.
const OPEN_CONNECTIONS: &str = "neon_databases_open_connections";
const DISCONNECTS: &str = "neon_databases_disconnects_total";
pub(crate) const EVENTS: &str = "neon_databases_events_total";

/// How many connections the watcher looks at again at once.
const RECHECKS_AT_ONCE: usize = 8;

/// How many and how fast. [`LiveLimits::default`] holds what the server runs with; tests
/// shorten the times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveLimits {
    /// How many feeds one person may have open on this server, and one database. One more
    /// is refused before its upgrade, with `429`, or closed as too many connections when
    /// several came at once.
    pub connections_per_user: usize,
    pub connections_per_database: usize,
    /// How many messages, and bytes of them, may wait to be sent to a client. One that
    /// falls further behind is disconnected, and reconnects to read afresh.
    pub client_queue: usize,
    pub client_queue_bytes: usize,
    /// The largest message a client may send, which is dropped: larger closes it.
    pub max_message: usize,
    /// How many messages a client may send per second, on average, and at once.
    pub messages_per_second: u32,
    pub message_burst: u32,
    /// How long sending one message may take.
    pub write_timeout: Duration,
    /// How often the server pings each client; one that sent nothing, not even a pong,
    /// between two pings is taken for gone.
    pub ping_interval: Duration,
    /// How often every connection is looked at again, as an access change would make it.
    pub revalidate_interval: Duration,
}

impl Default for LiveLimits {
    fn default() -> Self {
        Self {
            connections_per_user: 20,
            connections_per_database: 200,
            client_queue: 256,
            client_queue_bytes: 8 * 1024 * 1024,
            max_message: 4096,
            messages_per_second: 5,
            message_burst: 20,
            write_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(30),
            revalidate_interval: Duration::from_secs(300),
        }
    }
}

/// The feeds of every database followed. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct LiveDatabases {
    inner: Arc<Inner>,
}

struct Inner {
    databases: DatabaseService,
    auth: AuthService,
    broadcaster: Arc<dyn Broadcaster>,
    limits: LiveLimits,
    registry: Mutex<Registry>,
    stop: CancellationToken,
    tasks: TaskTracker,
    epoch: Arc<AtomicU64>,
}

#[derive(Default)]
struct Registry {
    databases: HashMap<Uuid, HashMap<ClientId, Follower>>,
    users: HashMap<Uuid, usize>,
}

/// An open connection, for the watcher and the rooms.
struct Follower {
    principal: Principal,
    workspace_id: Uuid,
    access: PageAccess,
    members_hidden: bool,
    outbox: Outbox,
}

fn room(database_id: Uuid, members_hidden: bool) -> Room {
    Room::Database {
        id: database_id,
        members_hidden,
    }
}

impl Registry {
    fn has_room(&self, limits: &LiveLimits, user_id: Uuid, database_id: Uuid) -> bool {
        let theirs = self.users.get(&user_id).copied().unwrap_or(0);
        let database = self.databases.get(&database_id).map_or(0, HashMap::len);
        theirs < limits.connections_per_user && database < limits.connections_per_database
    }

    /// Takes a connection out, with what room it was in.
    fn remove(&mut self, database_id: Uuid, client: ClientId) -> Option<Follower> {
        let followers = self.databases.get_mut(&database_id)?;
        let follower = followers.remove(&client)?;
        if followers.is_empty() {
            self.databases.remove(&database_id);
        }
        if let Some(count) = self.users.get_mut(&follower.principal.user.id) {
            *count -= 1;
            if *count == 0 {
                self.users.remove(&follower.principal.user.id);
            }
        }
        Some(follower)
    }
}

/// Puts [`LiveDatabases`] together, with the default limits unless told otherwise.
pub struct LiveDatabasesBuilder {
    databases: DatabaseService,
    auth: AuthService,
    feed: AccessFeed,
    limits: LiveLimits,
}

impl LiveDatabasesBuilder {
    pub fn limits(mut self, limits: LiveLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Starts the access watcher, so call it inside the runtime, after telemetry is
    /// installed, as it creates the metrics. The watcher holds the feeds weakly, and stops
    /// once they are gone.
    pub fn build(self) -> LiveDatabases {
        describe();
        let inner = Arc::new(Inner {
            broadcaster: Arc::clone(self.databases.pages().broadcaster()),
            databases: self.databases,
            auth: self.auth,
            limits: self.limits,
            registry: Mutex::new(Registry::default()),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            epoch: self.feed.epoch,
        });
        inner.tasks.spawn(watch(
            Arc::downgrade(&inner),
            inner.stop.clone(),
            inner.limits.revalidate_interval,
            self.feed.changes,
        ));
        LiveDatabases { inner }
    }
}

fn describe() {
    metrics::describe_gauge!(
        OPEN_CONNECTIONS,
        Unit::Count,
        "Open WebSocket connections following databases."
    );
    metrics::describe_counter!(
        DISCONNECTS,
        Unit::Count,
        "Database feeds ended, by cause: the client, an unresponsive client, or the server's close reason."
    );
    metrics::describe_counter!(
        EVENTS,
        Unit::Count,
        "Changes told to those following databases, by type."
    );
    let causes = CloseReason::ALL
        .map(Ending::Closed)
        .into_iter()
        .chain([Ending::ByClient, Ending::Unresponsive]);
    for ending in causes {
        metrics::counter!(DISCONNECTS, "cause" => ending.cause()).absolute(0);
    }
}

/// A principal the rules let follow a database, with what they may do with it.
#[derive(Debug, Clone)]
pub struct Admission {
    principal: Principal,
    database_id: Uuid,
    workspace_id: Uuid,
    access: PageAccess,
    members_hidden: bool,
    /// How many access changes were heard before the rules were asked.
    epoch: u64,
}

impl Admission {
    pub fn access(&self) -> PageAccess {
        self.access
    }
}

impl LiveDatabases {
    /// The feeds of the databases of `databases`, whose followers' sessions and tokens
    /// `auth` checks again as access changes, and whose changes `feed` brings.
    pub fn builder(
        databases: DatabaseService,
        auth: AuthService,
        feed: AccessFeed,
    ) -> LiveDatabasesBuilder {
        LiveDatabasesBuilder {
            databases,
            auth,
            feed,
            limits: LiveLimits::default(),
        }
    }

    pub fn limits(&self) -> &LiveLimits {
        &self.inner.limits
    }

    /// Asks the rules whether the principal may follow the database. One they may not see,
    /// and a page that is no database, are not found.
    pub async fn admit(
        &self,
        principal: Principal,
        database_id: Uuid,
    ) -> Result<Admission, DatabaseError> {
        let epoch = self.inner.epoch.load(Ordering::SeqCst);
        let followed = self
            .inner
            .databases
            .follow_access(&principal, database_id)
            .await?;
        Ok(Admission {
            principal,
            database_id,
            workspace_id: followed.workspace_id,
            access: followed.access,
            members_hidden: followed.members_hidden,
            epoch,
        })
    }

    /// Whether the principal may open one more feed, of the database, as the limits allow.
    /// Ask it before the upgrade; [`connect`](Self::connect) holds to the same limits.
    pub fn has_room(&self, admission: &Admission) -> bool {
        self.inner.registry().has_room(
            &self.inner.limits,
            admission.principal.user.id,
            admission.database_id,
        )
    }

    /// Opens an admitted principal's feed. Its first message is `ready`.
    pub fn connect(&self, admission: Admission) -> Result<Follow, CloseReason> {
        if self.inner.stop.is_cancelled() {
            return Err(CloseReason::Restarting);
        }
        let limits = &self.inner.limits;
        let id = ClientId::next();
        let (outbox, receiver) = Outbox::new(limits.client_queue, limits.client_queue_bytes);
        {
            let mut registry = self.inner.registry();
            let user_id = admission.principal.user.id;
            if !registry.has_room(limits, user_id, admission.database_id) {
                return Err(CloseReason::TooManyConnections);
            }
            *registry.users.entry(user_id).or_default() += 1;
            registry
                .databases
                .entry(admission.database_id)
                .or_default()
                .insert(
                    id,
                    Follower {
                        principal: admission.principal.clone(),
                        workspace_id: admission.workspace_id,
                        access: admission.access,
                        members_hidden: admission.members_hidden,
                        outbox: outbox.clone(),
                    },
                );
            // In the registry's lock, so a recheck finds the room the connection is in.
            self.inner.broadcaster.join(
                room(admission.database_id, admission.members_hidden),
                id,
                outbox.clone(),
            );
        }
        outbox.send(access_message(
            "ready",
            admission.database_id,
            admission.access,
        ));
        metrics::gauge!(OPEN_CONNECTIONS).increment(1.0);
        let follow = Follow {
            id,
            database_id: admission.database_id,
            receiver,
            bucket: Bucket::new(limits.messages_per_second, limits.message_burst),
            inner: Arc::clone(&self.inner),
            left: false,
        };
        if self.inner.epoch.load(Ordering::SeqCst) != admission.epoch {
            // A change may have committed after the rules were asked, and before the
            // watcher could see this connection.
            let inner = Arc::clone(&self.inner);
            let recheck = Recheck {
                database_id: admission.database_id,
                client: id,
                principal: admission.principal,
            };
            self.inner.tasks.spawn(async move {
                if let Err(error) = inner.recheck(recheck).await {
                    warn!(%error, "cannot look again at a database feed's access");
                }
            });
        }
        Ok(follow)
    }

    /// Stops: every feed closes, to reconnect to the next server, then the watcher stops.
    pub async fn shutdown(&self) {
        self.inner.stop.cancel();
        let outboxes: Vec<Outbox> = self
            .inner
            .registry()
            .databases
            .values()
            .flat_map(|followers| followers.values().map(|follower| follower.outbox.clone()))
            .collect();
        info!(connections = outboxes.len(), "closing the database feeds");
        for outbox in outboxes {
            outbox.close(CloseReason::Restarting);
        }
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
    }
}

/// What following needs to know of a database for one person.
pub(crate) struct Followed {
    pub(crate) workspace_id: Uuid,
    pub(crate) access: PageAccess,
    pub(crate) members_hidden: bool,
}

impl DatabaseService {
    /// What the principal may do with a database they follow: edit it when they edit it,
    /// it is out of the trash and their token, if any, has the write scope; else view it.
    pub(crate) async fn follow_access(
        &self,
        principal: &Principal,
        database_id: Uuid,
    ) -> Result<Followed, DatabaseError> {
        self.observe("follow", async {
            let mut conn = self.pages().pool().acquire().await?;
            let opened = self.open(&mut conn, principal, database_id, false).await?;
            Self::check(&opened.context, Action::ViewPage)?;
            let edits =
                principal.can(Scope::Write) && opened.context.check(Action::EditPage).is_ok();
            Ok(Followed {
                workspace_id: opened.database.workspace_id,
                access: if edits {
                    PageAccess::Edit
                } else {
                    PageAccess::View
                },
                members_hidden: opened.context.members_hidden(),
            })
        })
        .await
    }
}

/// One client's feed of a database, for its transport: [`next`](Self::next) says what to
/// send it or when to close, and what the client sends goes to
/// [`receive`](Self::receive), to be dropped. End it with [`leave`](Self::leave).
pub struct Follow {
    id: ClientId,
    database_id: Uuid,
    receiver: OutboxReceiver,
    bucket: Bucket,
    inner: Arc<Inner>,
    left: bool,
}

impl Follow {
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// The next message for the client, a JSON text, or why the feed closes.
    pub async fn next(&mut self) -> Next {
        self.receiver.next().await
    }

    /// Takes note of a message of `len` bytes from the client, which the feed does not
    /// read. An error is why it must close now.
    pub fn receive(&mut self, len: usize) -> Result<(), CloseReason> {
        if let Some(reason) = self.receiver.closed() {
            return Err(reason);
        }
        if len > self.inner.limits.max_message {
            return Err(CloseReason::MessageTooBig);
        }
        if !self.bucket.take() {
            return Err(CloseReason::TooManyMessages);
        }
        Ok(())
    }

    /// Ends the feed once its socket closed. `ending` is how it ended, for the metrics.
    pub fn leave(mut self, ending: Ending) {
        self.left = true;
        metrics::counter!(DISCONNECTS, "cause" => ending.cause()).increment(1);
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        if !self.left {
            metrics::counter!(DISCONNECTS, "cause" => Ending::ByClient.cause()).increment(1);
        }
        let mut registry = self.inner.registry();
        if let Some(follower) = registry.remove(self.database_id, self.id) {
            self.inner
                .broadcaster
                .leave(room(self.database_id, follower.members_hidden), self.id);
        }
        drop(registry);
        metrics::gauge!(OPEN_CONNECTIONS).decrement(1.0);
    }
}

/// A connection the watcher looks at again.
struct Recheck {
    database_id: Uuid,
    client: ClientId,
    principal: Principal,
}

/// Why a connection could not be looked at again: it keeps what it had until the next look.
#[derive(Debug, thiserror::Error)]
enum RecheckError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

impl Inner {
    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The connections whose workspace and person `keep` says to look at again.
    fn followers(&self, mut keep: impl FnMut(Uuid, &Principal) -> bool) -> Vec<Recheck> {
        let registry = self.registry();
        let mut found = Vec::new();
        for (database_id, followers) in &registry.databases {
            for (client, follower) in followers {
                if keep(follower.workspace_id, &follower.principal) {
                    found.push(Recheck {
                        database_id: *database_id,
                        client: *client,
                        principal: follower.principal.clone(),
                    });
                }
            }
        }
        found
    }

    /// The connections any of `changes` may concern, each once.
    async fn concerned(&self, changes: &[AccessChange]) -> Vec<Recheck> {
        let mut found: HashMap<ClientId, Recheck> = HashMap::new();
        for change in changes {
            let candidates = self
                .followers(|workspace, principal| change.concerns(workspace, principal.user.id));
            let candidates = match change.subtree() {
                Some(root) if !candidates.is_empty() => self.within(root, candidates).await,
                _ => candidates,
            };
            for recheck in candidates {
                found.entry(recheck.client).or_insert(recheck);
            }
        }
        found.into_values().collect()
    }

    /// The connections to `root`, a database under it, or one that is gone. When the pages
    /// cannot be read, all of them, to be safe.
    async fn within(&self, root: Uuid, candidates: Vec<Recheck>) -> Vec<Recheck> {
        let databases: Vec<Uuid> = candidates
            .iter()
            .map(|recheck| recheck.database_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let pool = self.databases.pages().pool();
        match neon_store::pages::within(pool, root, &databases).await {
            Ok(under) => {
                let under: HashSet<Uuid> = under.into_iter().collect();
                candidates
                    .into_iter()
                    .filter(|recheck| under.contains(&recheck.database_id))
                    .collect()
            }
            Err(error) => {
                warn!(%error, "cannot tell which followed databases a change reaches; looking at all");
                candidates
            }
        }
    }

    /// Checks again that a connection's session or token holds, then asks the rules what
    /// its principal may do with the database now, and closes, tells or moves it.
    async fn recheck(&self, recheck: Recheck) -> Result<(), RecheckError> {
        let principal = match self.auth.refresh(&recheck.principal).await {
            Ok(principal) => principal,
            Err(AuthError::Unauthenticated) => {
                self.close(recheck.database_id, recheck.client, CloseReason::SignedOut);
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let followed = match self
            .databases
            .follow_access(&principal, recheck.database_id)
            .await
        {
            Ok(followed) => Some(followed),
            Err(error) if error.is_expected() => None,
            Err(error) => return Err(error.into()),
        };
        let mut registry = self.registry();
        let Some(follower) = registry
            .databases
            .get_mut(&recheck.database_id)
            .and_then(|followers| followers.get_mut(&recheck.client))
        else {
            return Ok(());
        };
        follower.principal = principal;
        let Some(followed) = followed else {
            info!(
                client = %recheck.client,
                user_id = %follower.principal.user.id,
                "closing a database feed: access lost"
            );
            follower.outbox.close(CloseReason::AccessLost);
            return Ok(());
        };
        if followed.members_hidden != follower.members_hidden {
            self.broadcaster.leave(
                room(recheck.database_id, follower.members_hidden),
                recheck.client,
            );
            self.broadcaster.join(
                room(recheck.database_id, followed.members_hidden),
                recheck.client,
                follower.outbox.clone(),
            );
            follower.members_hidden = followed.members_hidden;
        }
        if followed.access != follower.access {
            follower.access = followed.access;
            follower.outbox.send(access_message(
                "access",
                recheck.database_id,
                followed.access,
            ));
            info!(
                client = %recheck.client,
                user_id = %follower.principal.user.id,
                access = %followed.access,
                "database feed access changed"
            );
        }
        Ok(())
    }

    fn close(&self, database_id: Uuid, client: ClientId, reason: CloseReason) {
        let registry = self.registry();
        if let Some(follower) = registry
            .databases
            .get(&database_id)
            .and_then(|followers| followers.get(&client))
        {
            follower.outbox.close(reason);
        }
    }
}

/// Looks again at the connections each change concerns, and at every connection every
/// `every`, until the feeds stop or are gone.
async fn watch(
    inner: Weak<Inner>,
    stop: CancellationToken,
    every: Duration,
    mut changes: mpsc::UnboundedReceiver<AccessChange>,
) {
    let mut revalidate = interval_at(Instant::now() + every, every);
    revalidate.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let batch = tokio::select! {
            () = stop.cancelled() => return,
            change = changes.recv() => match change {
                Some(change) => {
                    let mut batch = vec![change];
                    while let Ok(change) = changes.try_recv() {
                        batch.push(change);
                    }
                    Some(batch)
                }
                None => return,
            },
            _ = revalidate.tick() => None,
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let rechecks = match &batch {
            Some(batch) => inner.concerned(batch).await,
            None => inner.followers(|_, _| true),
        };
        debug!(
            changes = batch.as_ref().map_or(0, Vec::len),
            connections = rechecks.len(),
            "looking again at database feeds' access"
        );
        let inner = &inner;
        futures_util::stream::iter(rechecks)
            .for_each_concurrent(RECHECKS_AT_ONCE, |recheck| async move {
                if let Err(error) = inner.recheck(recheck).await {
                    warn!(%error, "cannot look again at a database feed's access; it keeps it for now");
                }
            })
            .await;
    }
}
