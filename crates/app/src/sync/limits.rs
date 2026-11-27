//! The bounds live documents and their connections keep to, and the rate limit of each
//! connection.

use std::time::Duration;

use tokio::time::Instant;

/// How large, how fast and how long. [`Limits::default`] holds the values the server runs
/// with; tests shorten the times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest message a client may send, in bytes. A larger one closes the connection.
    pub max_message: usize,
    /// The largest a document may grow, in bytes: its last snapshot and the updates logged
    /// after it. A change that would make it larger is refused, unless it only deletes.
    pub max_document: usize,
    /// How many messages a connection may send per second, on average.
    pub messages_per_second: u32,
    /// How many it may send at once, above the average, before it is closed.
    pub message_burst: u32,
    /// How many messages may wait to be sent to a client. One that falls further behind is
    /// disconnected, so it cannot hold up the others.
    pub client_queue: usize,
    /// How many bytes of messages may wait to be sent to a client, likewise. Room for a
    /// whole document's sync step 2 and a few updates; a message alone in the queue is
    /// taken whatever its size.
    pub client_queue_bytes: usize,
    /// How many sync step 1 messages, each answered with what the client lacks, a
    /// connection may send per second, on average, and at once above that. A connection
    /// sends one as it opens; more close it as too many messages. One that comes while the
    /// answer to the last still waits to be sent is not answered again: the client gets
    /// that answer, and every update after it.
    pub sync_requests_per_second: u32,
    pub sync_request_burst: u32,
    /// How many connections one person may have open on this server, across every page,
    /// and how many one page may have. One more is refused before its upgrade, with
    /// `429`, or closed as too many connections when several came at once.
    pub connections_per_user: usize,
    pub connections_per_page: usize,
    /// How long sending one message to a client may take: long enough for a whole large
    /// document over a slow link.
    pub write_timeout: Duration,
    /// How often the server pings each client. A client that sent nothing, not even a pong,
    /// between two pings is taken for gone.
    pub ping_interval: Duration,
    /// How long a document stays in memory after its last client left.
    pub idle_unload: Duration,
    /// How long incoming updates wait, to be logged together.
    pub persist_delay: Duration,
    /// How many bytes of updates may wait before they are logged at once.
    pub persist_bytes: usize,
    /// How many logged updates, or bytes of them, make the document compact into a new
    /// snapshot.
    pub compact_updates: usize,
    pub compact_bytes: usize,
    /// How many awareness client ids, cursors and presence, one connection may hold.
    pub awareness_clients: usize,
    /// The largest awareness state, a client's cursor and name as JSON, in bytes. A larger
    /// one closes the connection as a message too big.
    pub max_awareness_state: usize,
    /// How many awareness messages a connection may send per second, on average, and at
    /// once above that. Past them its awareness messages are dropped, not applied nor
    /// sent on: the next one tells the same, and browsers renew theirs every 15 seconds.
    pub awareness_per_second: u32,
    pub awareness_burst: u32,
    /// How often every open connection is looked at again, as an access change would make
    /// it: whether its session or token still holds, and what the rules let it do now.
    /// It catches what no change tells of, such as a session expiring.
    pub revalidate_interval: Duration,
    /// How long a document must go without changes for its mentions to settle: then, when
    /// it mentions someone it did not before, it is saved as a snapshot, which tells them.
    pub mention_settle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_message: 4 * 1024 * 1024,
            max_document: 16 * 1024 * 1024,
            messages_per_second: 100,
            message_burst: 1000,
            client_queue: 512,
            client_queue_bytes: 24 * 1024 * 1024,
            sync_requests_per_second: 1,
            sync_request_burst: 10,
            connections_per_user: 20,
            connections_per_page: 200,
            write_timeout: Duration::from_secs(30),
            ping_interval: Duration::from_secs(30),
            idle_unload: Duration::from_secs(30),
            persist_delay: Duration::from_millis(250),
            persist_bytes: 256 * 1024,
            compact_updates: 500,
            compact_bytes: 4 * 1024 * 1024,
            awareness_clients: 8,
            max_awareness_state: 8 * 1024,
            awareness_per_second: 20,
            awareness_burst: 100,
            revalidate_interval: Duration::from_secs(300),
            mention_settle: Duration::from_secs(10),
        }
    }
}

/// A token bucket: `burst` tokens, refilled at `rate` a second.
#[derive(Debug)]
pub(crate) struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    filled_at: Instant,
}

impl Bucket {
    pub(crate) fn new(rate: u32, burst: u32) -> Self {
        let burst = f64::from(burst.max(1));
        Self {
            rate: f64::from(rate),
            burst,
            tokens: burst,
            filled_at: Instant::now(),
        }
    }

    /// Takes a token, if there is one.
    pub(crate) fn take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.filled_at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.filled_at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_bucket_allows_a_burst_then_its_rate() {
        let mut bucket = Bucket::new(10, 5);

        assert!((0..5).all(|_| bucket.take()));
        assert!(!bucket.take());

        tokio::time::advance(Duration::from_millis(100)).await;
        assert!(bucket.take());
        assert!(!bucket.take());

        tokio::time::advance(Duration::from_secs(60)).await;
        assert!((0..5).all(|_| bucket.take()));
        assert!(!bucket.take());
    }
}
