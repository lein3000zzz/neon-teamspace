//! Getting messages to clients: each connection's outbox, and the [`Broadcaster`] port that
//! hands a message to every client in a room, those of a page's live document or of a
//! database's feed.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bytes::Bytes;
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::CloseReason;

/// A connection, while it is open. Unique in the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(u64);

impl ClientId {
    /// A new id, unlike any other of the process.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where messages for one client wait until its connection sends them: a queue bounded in
/// length and in bytes, and, once the connection is to close, why. Cheap to clone.
#[derive(Clone)]
pub struct Outbox {
    queue: mpsc::Sender<Bytes>,
    closing: Arc<Closing>,
    max_bytes: usize,
}

/// What an outbox and its receiver share.
struct Closing {
    reason: OnceLock<CloseReason>,
    signal: CancellationToken,
    /// The bytes of the messages waiting.
    waiting: AtomicUsize,
    /// How many messages were queued, and how many of them the connection took to send.
    queued: AtomicU64,
    taken: AtomicU64,
}

/// A message queued in an [`Outbox`], to tell later whether it was taken to be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Queued(u64);

impl Outbox {
    /// An outbox holding at most `capacity` messages and `max_bytes` of them, and the end
    /// its connection reads. A message is taken into an empty outbox whatever its size.
    pub fn new(capacity: usize, max_bytes: usize) -> (Self, OutboxReceiver) {
        let (queue, receiver) = mpsc::channel(capacity.max(1));
        let closing = Arc::new(Closing {
            reason: OnceLock::new(),
            signal: CancellationToken::new(),
            waiting: AtomicUsize::new(0),
            queued: AtomicU64::new(0),
            taken: AtomicU64::new(0),
        });
        (
            Self {
                queue,
                closing: Arc::clone(&closing),
                max_bytes,
            },
            OutboxReceiver {
                queue: receiver,
                closing,
            },
        )
    }

    /// Queues a message, without waiting. A full queue, by count or by bytes, means the
    /// client fell behind: the connection closes instead, as [`CloseReason::TooSlow`].
    /// Returns whether it was queued.
    pub fn send(&self, message: Bytes) -> bool {
        self.queue_message(message).is_some()
    }

    /// Queues a message as [`send`](Self::send) does, and says which it was, so that
    /// [`was_taken`](Self::was_taken) tells later whether the connection took it to send.
    pub fn send_tracked(&self, message: Bytes) -> Option<Queued> {
        self.queue_message(message)
    }

    /// Whether the connection took a message queued earlier to send: it no longer waits.
    pub fn was_taken(&self, queued: Queued) -> bool {
        self.closing.taken.load(Ordering::SeqCst) >= queued.0
    }

    fn queue_message(&self, message: Bytes) -> Option<Queued> {
        if self.is_closing() {
            return None;
        }
        let len = message.len();
        let waiting = self.closing.waiting.fetch_add(len, Ordering::SeqCst);
        if waiting > 0 && waiting.saturating_add(len) > self.max_bytes {
            self.closing.waiting.fetch_sub(len, Ordering::SeqCst);
            self.close(CloseReason::TooSlow);
            return None;
        }
        match self.queue.try_send(message) {
            // Its document's task is the one to queue messages for a client, so the count
            // is this message's place in the queue.
            Ok(()) => Some(Queued(
                self.closing.queued.fetch_add(1, Ordering::SeqCst) + 1,
            )),
            Err(error) => {
                self.closing.waiting.fetch_sub(len, Ordering::SeqCst);
                if matches!(error, TrySendError::Full(_)) {
                    self.close(CloseReason::TooSlow);
                }
                None
            }
        }
    }

    /// Closes the connection for `reason`, unless it is closing already for another.
    pub fn close(&self, reason: CloseReason) {
        if self.closing.reason.set(reason).is_ok() {
            self.closing.signal.cancel();
        }
    }

    pub fn is_closing(&self) -> bool {
        self.closing.reason.get().is_some()
    }
}

/// What a connection does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Sends this message.
    Send(Bytes),
    /// Closes, for this reason.
    Close(CloseReason),
}

/// The end of an outbox its connection reads.
pub struct OutboxReceiver {
    queue: mpsc::Receiver<Bytes>,
    closing: Arc<Closing>,
}

impl OutboxReceiver {
    /// The next message to send, or why the connection closes. Closing goes first: what
    /// still waits is dropped.
    pub async fn next(&mut self) -> Next {
        tokio::select! {
            biased;
            () = self.closing.signal.cancelled() => Next::Close(self.reason()),
            message = self.queue.recv() => match message {
                Some(message) => {
                    self.closing.waiting.fetch_sub(message.len(), Ordering::SeqCst);
                    self.closing.taken.fetch_add(1, Ordering::SeqCst);
                    Next::Send(message)
                }
                // Every sender is gone without closing it: the document is.
                None => Next::Close(self.reason()),
            },
        }
    }

    /// Why the connection closes; an outbox whose senders are all gone without a reason
    /// lost its document to a failure.
    fn reason(&self) -> CloseReason {
        self.closing
            .reason
            .get()
            .copied()
            .unwrap_or(CloseReason::ServerError)
    }

    /// Why the connection closes, once it does.
    pub fn closed(&self) -> Option<CloseReason> {
        self.closing.reason.get().copied()
    }
}

/// Who hears the same messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Room {
    /// The clients that have a page's live document open.
    Page(Uuid),
    /// The clients that follow a database's changes. Guests, who know no other member,
    /// hear the changes without the people they name, in a room of their own.
    Database { id: Uuid, members_hidden: bool },
}

/// Hands messages to the clients in a room. It never waits: a client that cannot take a
/// message falls behind and is disconnected, so it cannot hold up the others.
///
/// One process has every client of its rooms, which [`LocalBroadcaster`] serves. Running a
/// second server process would need an adapter that carries messages between them.
pub trait Broadcaster: Send + Sync {
    /// Adds a client to the room.
    fn join(&self, room: Room, client: ClientId, outbox: Outbox);
    /// Takes a client out of the room.
    fn leave(&self, room: Room, client: ClientId);
    /// Sends a message to every client in the room but `except`.
    fn broadcast(&self, room: Room, message: Bytes, except: Option<ClientId>);
}

/// The broadcaster of a single process: each room's audience, in memory.
#[derive(Default)]
pub struct LocalBroadcaster {
    audiences: Mutex<HashMap<Room, HashMap<ClientId, Outbox>>>,
}

impl LocalBroadcaster {
    pub fn new() -> Self {
        Self::default()
    }

    fn audiences(&self) -> std::sync::MutexGuard<'_, HashMap<Room, HashMap<ClientId, Outbox>>> {
        self.audiences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Broadcaster for LocalBroadcaster {
    fn join(&self, room: Room, client: ClientId, outbox: Outbox) {
        self.audiences()
            .entry(room)
            .or_default()
            .insert(client, outbox);
    }

    fn leave(&self, room: Room, client: ClientId) {
        let mut audiences = self.audiences();
        if let Some(audience) = audiences.get_mut(&room) {
            audience.remove(&client);
            if audience.is_empty() {
                audiences.remove(&room);
            }
        }
    }

    fn broadcast(&self, room: Room, message: Bytes, except: Option<ClientId>) {
        let audiences = self.audiences();
        let Some(audience) = audiences.get(&room) else {
            return;
        };
        for (client, outbox) in audience {
            if Some(*client) != except {
                outbox.send(message.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_full_outbox_closes_its_connection_as_too_slow() {
        let (outbox, mut receiver) = Outbox::new(2, 1024);

        assert!(outbox.send(Bytes::from_static(b"1")));
        assert!(outbox.send(Bytes::from_static(b"2")));
        assert!(!outbox.send(Bytes::from_static(b"3")));

        assert!(outbox.is_closing());
        assert_eq!(receiver.next().await, Next::Close(CloseReason::TooSlow));
        assert_eq!(receiver.closed(), Some(CloseReason::TooSlow));
        // A later reason does not replace the first.
        outbox.close(CloseReason::AccessLost);
        assert_eq!(receiver.next().await, Next::Close(CloseReason::TooSlow));
    }

    #[tokio::test]
    async fn an_outbox_holding_too_many_bytes_closes_its_connection() {
        let (outbox, mut receiver) = Outbox::new(8, 10);

        // Alone, a message larger than the bound is taken, and sent.
        assert!(outbox.send(Bytes::from(vec![1; 16])));
        assert_eq!(receiver.next().await, Next::Send(Bytes::from(vec![1; 16])));
        assert!(outbox.send(Bytes::from(vec![2; 6])));
        assert!(outbox.send(Bytes::from(vec![3; 4])));
        // Sent messages make room.
        assert_eq!(receiver.next().await, Next::Send(Bytes::from(vec![2; 6])));
        assert!(outbox.send(Bytes::from(vec![4; 6])));
        assert!(!outbox.send(Bytes::from(vec![5; 1])));

        assert_eq!(receiver.next().await, Next::Close(CloseReason::TooSlow));
    }

    #[tokio::test]
    async fn an_outbox_tells_when_a_message_was_taken() {
        let (outbox, mut receiver) = Outbox::new(8, 1024);
        let first = outbox
            .send_tracked(Bytes::from_static(b"a"))
            .expect("queued");
        let second = outbox
            .send_tracked(Bytes::from_static(b"b"))
            .expect("queued");

        assert!(!outbox.was_taken(first));
        receiver.next().await;
        assert!(outbox.was_taken(first));
        assert!(!outbox.was_taken(second));
        receiver.next().await;
        assert!(outbox.was_taken(second));
    }

    #[tokio::test]
    async fn messages_come_in_order_until_the_outbox_closes() {
        let (outbox, mut receiver) = Outbox::new(4, 1024);
        outbox.send(Bytes::from_static(b"a"));
        outbox.send(Bytes::from_static(b"b"));

        assert_eq!(receiver.next().await, Next::Send(Bytes::from_static(b"a")));
        assert_eq!(receiver.next().await, Next::Send(Bytes::from_static(b"b")));
        drop(outbox);
        assert_eq!(receiver.next().await, Next::Close(CloseReason::ServerError));
    }

    #[tokio::test]
    async fn broadcasts_reach_the_room_s_audience_but_the_sender() {
        let broadcaster = LocalBroadcaster::new();
        let page = Room::Page(Uuid::from_u128(1));
        let other_page = Room::Page(Uuid::from_u128(2));
        let (a, b, c) = (ClientId::next(), ClientId::next(), ClientId::next());
        let (outbox_a, mut to_a) = Outbox::new(4, 1024);
        let (outbox_b, mut to_b) = Outbox::new(4, 1024);
        let (outbox_c, mut to_c) = Outbox::new(4, 1024);
        broadcaster.join(page, a, outbox_a);
        broadcaster.join(page, b, outbox_b);
        broadcaster.join(other_page, c, outbox_c.clone());

        broadcaster.broadcast(page, Bytes::from_static(b"hi"), Some(a));
        broadcaster.leave(page, b);
        broadcaster.broadcast(page, Bytes::from_static(b"again"), None);

        assert_eq!(to_b.next().await, Next::Send(Bytes::from_static(b"hi")));
        assert_eq!(to_a.next().await, Next::Send(Bytes::from_static(b"again")));
        outbox_c.close(CloseReason::Restarting);
        assert_eq!(to_c.next().await, Next::Close(CloseReason::Restarting));
    }

    #[tokio::test]
    async fn a_database_s_guests_are_in_a_room_of_their_own() {
        let broadcaster = LocalBroadcaster::new();
        let id = Uuid::from_u128(1);
        let members = Room::Database {
            id,
            members_hidden: false,
        };
        let guests = Room::Database {
            id,
            members_hidden: true,
        };
        let (member, guest) = (ClientId::next(), ClientId::next());
        let (outbox_m, mut to_m) = Outbox::new(4, 1024);
        let (outbox_g, mut to_g) = Outbox::new(4, 1024);
        broadcaster.join(members, member, outbox_m);
        broadcaster.join(guests, guest, outbox_g);
        // The page of the same id is another room.
        broadcaster.broadcast(Room::Page(id), Bytes::from_static(b"page"), None);
        broadcaster.broadcast(members, Bytes::from_static(b"full"), None);
        broadcaster.broadcast(guests, Bytes::from_static(b"hidden"), None);

        assert_eq!(to_m.next().await, Next::Send(Bytes::from_static(b"full")));
        assert_eq!(to_g.next().await, Next::Send(Bytes::from_static(b"hidden")));
    }
}
