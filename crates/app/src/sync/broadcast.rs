//! Getting a live document's messages to its clients: each connection's outbox, and the
//! [`Broadcaster`] port that hands a message to every client of a page.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where messages for one client wait until its connection sends them: a queue of bounded
/// length, and, once the connection is to close, why. Cheap to clone.
#[derive(Clone)]
pub struct Outbox {
    queue: mpsc::Sender<Bytes>,
    closing: Arc<Closing>,
}

struct Closing {
    reason: OnceLock<CloseReason>,
    signal: CancellationToken,
}

impl Outbox {
    /// An outbox holding at most `capacity` messages, and the end its connection reads.
    pub fn new(capacity: usize) -> (Self, OutboxReceiver) {
        let (queue, receiver) = mpsc::channel(capacity.max(1));
        let closing = Arc::new(Closing {
            reason: OnceLock::new(),
            signal: CancellationToken::new(),
        });
        (
            Self {
                queue,
                closing: Arc::clone(&closing),
            },
            OutboxReceiver {
                queue: receiver,
                closing,
            },
        )
    }

    /// Queues a message, without waiting. A full queue means the client fell behind: the
    /// connection closes instead, as [`CloseReason::TooSlow`]. Returns whether it was queued.
    pub fn send(&self, message: Bytes) -> bool {
        if self.is_closing() {
            return false;
        }
        match self.queue.try_send(message) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.close(CloseReason::TooSlow);
                false
            }
            Err(TrySendError::Closed(_)) => false,
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
                Some(message) => Next::Send(message),
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

/// Hands a live document's messages to the clients that have its page open. It never
/// waits: a client that cannot take a message falls behind and is disconnected, so it
/// cannot hold up the others.
///
/// One process has every client of its documents, which [`LocalBroadcaster`] serves. Running
/// a second server process would need an adapter that carries messages between them.
pub trait Broadcaster: Send + Sync {
    /// Adds a client to the page's audience.
    fn join(&self, page_id: Uuid, client: ClientId, outbox: Outbox);
    /// Takes a client out of the page's audience.
    fn leave(&self, page_id: Uuid, client: ClientId);
    /// Sends a message to every client of the page but `except`.
    fn broadcast(&self, page_id: Uuid, message: Bytes, except: Option<ClientId>);
}

/// The broadcaster of a single process: each page's audience, in memory.
#[derive(Default)]
pub struct LocalBroadcaster {
    audiences: Mutex<HashMap<Uuid, HashMap<ClientId, Outbox>>>,
}

impl LocalBroadcaster {
    pub fn new() -> Self {
        Self::default()
    }

    fn audiences(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, HashMap<ClientId, Outbox>>> {
        self.audiences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Broadcaster for LocalBroadcaster {
    fn join(&self, page_id: Uuid, client: ClientId, outbox: Outbox) {
        self.audiences()
            .entry(page_id)
            .or_default()
            .insert(client, outbox);
    }

    fn leave(&self, page_id: Uuid, client: ClientId) {
        let mut audiences = self.audiences();
        if let Some(audience) = audiences.get_mut(&page_id) {
            audience.remove(&client);
            if audience.is_empty() {
                audiences.remove(&page_id);
            }
        }
    }

    fn broadcast(&self, page_id: Uuid, message: Bytes, except: Option<ClientId>) {
        let audiences = self.audiences();
        let Some(audience) = audiences.get(&page_id) else {
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
        let (outbox, mut receiver) = Outbox::new(2);

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
    async fn messages_come_in_order_until_the_outbox_closes() {
        let (outbox, mut receiver) = Outbox::new(4);
        outbox.send(Bytes::from_static(b"a"));
        outbox.send(Bytes::from_static(b"b"));

        assert_eq!(receiver.next().await, Next::Send(Bytes::from_static(b"a")));
        assert_eq!(receiver.next().await, Next::Send(Bytes::from_static(b"b")));
        drop(outbox);
        assert_eq!(receiver.next().await, Next::Close(CloseReason::ServerError));
    }

    #[tokio::test]
    async fn broadcasts_reach_the_page_s_audience_but_the_sender() {
        let broadcaster = LocalBroadcaster::new();
        let page = Uuid::from_u128(1);
        let other_page = Uuid::from_u128(2);
        let (a, b, c) = (ClientId::next(), ClientId::next(), ClientId::next());
        let (outbox_a, mut to_a) = Outbox::new(4);
        let (outbox_b, mut to_b) = Outbox::new(4);
        let (outbox_c, mut to_c) = Outbox::new(4);
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
}
