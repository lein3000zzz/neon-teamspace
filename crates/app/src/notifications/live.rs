//! Following an inbox live: the connections that hear their person's inbox change, and
//! closing those whose session or token ended.
//!
//! - **Admission** is for anyone signed in: a feed is their own inbox, which tells that it
//!   changed and nothing of what it holds, so the client reads it again through the API,
//!   which checks every notification.
//! - **Events** are JSON messages: `{"type":"ready"}` first, once the connection hears
//!   every change, and `{"type":"inbox"}` each time a notification of theirs is made,
//!   read, unread, archived or restored, or an invitation of theirs changes.
//! - **Access.** When someone's way in changes, their feeds are looked at again: one whose
//!   session or token ended closes as signed out. Every
//!   [`FeedLimits::revalidate_interval`] every feed is looked at so.
//! - **Limits** bound how many feeds a person may have open, each feed's queue, and what a
//!   client may send: nothing is expected of it, and what it sends is read and dropped,
//!   within a small size and rate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt as _;
use metrics::Unit;
use neon_domain::auth::Principal;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::{AuthError, AuthService};
use crate::pages::AccessChange;
use crate::standing::owner_of;
use crate::sync::{
    AccessFeed, Broadcaster, Bucket, ClientId, CloseReason, Ending, Next, Outbox, OutboxReceiver,
    Room,
};

const OPEN_CONNECTIONS: &str = "neon_notifications_open_connections";
const DISCONNECTS: &str = "neon_notifications_disconnects_total";

/// How many feeds the watcher looks at again at once.
const RECHECKS_AT_ONCE: usize = 8;

/// How many and how fast. [`FeedLimits::default`] holds what the server runs with; tests
/// shorten the times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedLimits {
    /// How many feeds one person may have open on this server: one for each tab.
    pub connections_per_user: usize,
    /// How many messages, and bytes of them, may wait to be sent to a client.
    pub client_queue: usize,
    pub client_queue_bytes: usize,
    /// The largest message a client may send, which is dropped: larger closes it.
    pub max_message: usize,
    /// How many messages a client may send per second, on average, and at once.
    pub messages_per_second: u32,
    pub message_burst: u32,
    /// How long sending one message may take.
    pub write_timeout: Duration,
    /// How often the server pings each client.
    pub ping_interval: Duration,
    /// How often every feed is looked at again.
    pub revalidate_interval: Duration,
}

impl Default for FeedLimits {
    fn default() -> Self {
        Self {
            connections_per_user: 20,
            client_queue: 64,
            client_queue_bytes: 64 * 1024,
            max_message: 1024,
            messages_per_second: 5,
            message_burst: 20,
            write_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(30),
            revalidate_interval: Duration::from_secs(300),
        }
    }
}

/// Every inbox followed. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct InboxFeeds {
    inner: Arc<Inner>,
}

struct Inner {
    auth: AuthService,
    broadcaster: Arc<dyn Broadcaster>,
    limits: FeedLimits,
    registry: Mutex<HashMap<ClientId, Follower>>,
    stop: CancellationToken,
    tasks: TaskTracker,
    epoch: Arc<AtomicU64>,
}

struct Follower {
    principal: Principal,
    outbox: Outbox,
}

impl Inner {
    fn registry(&self) -> MutexGuard<'_, HashMap<ClientId, Follower>> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn feeds_of(&self, user_id: Uuid) -> usize {
        self.registry()
            .values()
            .filter(|follower| follower.principal.user.id == user_id)
            .count()
    }

    /// Checks again that a feed's session or token holds, and closes it when not.
    async fn recheck(&self, client: ClientId, principal: Principal) -> Result<(), AuthError> {
        match self.auth.refresh(&principal).await {
            Ok(principal) => {
                if let Some(follower) = self.registry().get_mut(&client) {
                    follower.principal = principal;
                }
                Ok(())
            }
            Err(AuthError::Unauthenticated) => {
                if let Some(follower) = self.registry().get(&client) {
                    info!(%client, user_id = %principal.user.id, "closing an inbox feed: signed out");
                    follower.outbox.close(CloseReason::SignedOut);
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

/// Whether an access change may have ended the way in of `principal`: a change to their
/// account or their owner's, their leaving a workspace, or whom tokens act as.
fn concerns(change: &AccessChange, principal: &Principal) -> bool {
    let theirs = |user: Uuid| user == principal.user.id || owner_of(principal) == Some(user);
    match *change {
        AccessChange::Account { user_id }
        | AccessChange::Workspace {
            user_id: Some(user_id),
            ..
        } => theirs(user_id),
        AccessChange::Agents => owner_of(principal).is_some() || principal.workspace().is_some(),
        AccessChange::Workspace { user_id: None, .. }
        | AccessChange::Pages { .. }
        | AccessChange::Teamspace { .. } => false,
    }
}

/// The first message of a feed.
fn ready() -> Bytes {
    Bytes::from_static(br#"{"type":"ready"}"#)
}

impl InboxFeeds {
    /// The feeds of the inboxes `broadcaster` tells of, whose sessions and tokens `auth`
    /// checks again as `feed` tells of access changes. Starts the watcher, so call it
    /// inside the runtime, after telemetry is installed.
    pub fn new(
        auth: AuthService,
        broadcaster: Arc<dyn Broadcaster>,
        feed: AccessFeed,
        limits: FeedLimits,
    ) -> Self {
        metrics::describe_gauge!(
            OPEN_CONNECTIONS,
            Unit::Count,
            "Open WebSocket connections following inboxes."
        );
        metrics::describe_counter!(
            DISCONNECTS,
            Unit::Count,
            "Inbox feeds ended, by cause: the client, an unresponsive client, or the server's close reason."
        );
        let inner = Arc::new(Inner {
            auth,
            broadcaster,
            limits,
            registry: Mutex::new(HashMap::new()),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            epoch: feed.epoch,
        });
        inner.tasks.spawn(watch(
            Arc::downgrade(&inner),
            inner.stop.clone(),
            limits.revalidate_interval,
            feed.changes,
        ));
        Self { inner }
    }

    pub fn limits(&self) -> &FeedLimits {
        &self.inner.limits
    }

    /// Whether the principal may open one more feed. Ask it before the upgrade;
    /// [`connect`](Self::connect) holds to the same limit.
    pub fn has_room(&self, principal: &Principal) -> bool {
        self.inner.feeds_of(principal.user.id) < self.inner.limits.connections_per_user
    }

    /// Opens the principal's feed of their inbox. Its first message is `ready`.
    pub fn connect(&self, principal: Principal) -> Result<InboxFeed, CloseReason> {
        if self.inner.stop.is_cancelled() {
            return Err(CloseReason::Restarting);
        }
        let limits = &self.inner.limits;
        let epoch = self.inner.epoch.load(Ordering::SeqCst);
        let id = ClientId::next();
        let (outbox, receiver) = Outbox::new(limits.client_queue, limits.client_queue_bytes);
        let user_id = principal.user.id;
        {
            let mut registry = self.inner.registry();
            let theirs = registry
                .values()
                .filter(|follower| follower.principal.user.id == user_id)
                .count();
            if theirs >= limits.connections_per_user {
                return Err(CloseReason::TooManyConnections);
            }
            registry.insert(
                id,
                Follower {
                    principal: principal.clone(),
                    outbox: outbox.clone(),
                },
            );
            self.inner
                .broadcaster
                .join(Room::Inbox(user_id), id, outbox.clone());
        }
        outbox.send(ready());
        metrics::gauge!(OPEN_CONNECTIONS).increment(1.0);
        if self.inner.epoch.load(Ordering::SeqCst) != epoch {
            let inner = Arc::clone(&self.inner);
            self.inner.tasks.spawn(async move {
                if let Err(error) = inner.recheck(id, principal).await {
                    warn!(%error, "cannot look again at an inbox feed");
                }
            });
        }
        Ok(InboxFeed {
            id,
            user_id,
            receiver,
            bucket: Bucket::new(limits.messages_per_second, limits.message_burst),
            inner: Arc::clone(&self.inner),
            left: false,
        })
    }

    /// Stops: every feed closes, to reconnect to the next server, then the watcher stops.
    pub async fn shutdown(&self) {
        self.inner.stop.cancel();
        let outboxes: Vec<Outbox> = self
            .inner
            .registry()
            .values()
            .map(|follower| follower.outbox.clone())
            .collect();
        info!(connections = outboxes.len(), "closing the inbox feeds");
        for outbox in outboxes {
            outbox.close(CloseReason::Restarting);
        }
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
    }
}

/// One client's feed of its inbox, for its transport: [`next`](Self::next) says what to
/// send it or when to close, and what the client sends goes to
/// [`receive`](Self::receive), to be dropped. End it with [`leave`](Self::leave).
pub struct InboxFeed {
    id: ClientId,
    user_id: Uuid,
    receiver: OutboxReceiver,
    bucket: Bucket,
    inner: Arc<Inner>,
    left: bool,
}

impl InboxFeed {
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

impl Drop for InboxFeed {
    fn drop(&mut self) {
        if !self.left {
            metrics::counter!(DISCONNECTS, "cause" => Ending::ByClient.cause()).increment(1);
        }
        self.inner.registry().remove(&self.id);
        self.inner
            .broadcaster
            .leave(Room::Inbox(self.user_id), self.id);
        metrics::gauge!(OPEN_CONNECTIONS).decrement(1.0);
    }
}

/// Looks again at the feeds each change concerns, and at every feed every `every`, until
/// the feeds stop or are gone.
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
        let rechecks: Vec<(ClientId, Principal)> = inner
            .registry()
            .iter()
            .filter(|(_, follower)| {
                batch.as_ref().is_none_or(|batch| {
                    batch
                        .iter()
                        .any(|change| concerns(change, &follower.principal))
                })
            })
            .map(|(client, follower)| (*client, follower.principal.clone()))
            .collect();
        let inner = &inner;
        futures_util::stream::iter(rechecks)
            .for_each_concurrent(RECHECKS_AT_ONCE, |(client, principal)| async move {
                if let Err(error) = inner.recheck(client, principal).await {
                    warn!(%error, "cannot look again at an inbox feed; it stays open for now");
                }
            })
            .await;
    }
}
