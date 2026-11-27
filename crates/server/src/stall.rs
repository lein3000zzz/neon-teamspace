//! A connection that gives up on a client that stops taking what it is sent.
//!
//! An answer the client does not read stays in the server's memory, the whole of it, for as
//! long as the connection is open: a large export, a page's markdown, a WebSocket's messages.
//! Its writes wait on the client then, and [`WriteStall`] fails the one that waits longer
//! than its limit, which closes the connection and drops what it held. A client that reads,
//! however slowly, keeps every write moving and is never cut off.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Sleep, sleep};

/// How long a write may wait for the client to take what was sent before it fails.
pub const WRITE_STALL_LIMIT: Duration = Duration::from_secs(30);

/// A stream whose writes fail once one has waited `limit` for the other side to take any
/// of it.
pub struct WriteStall<S> {
    inner: S,
    limit: Duration,
    /// Since when the current write has waited, while it does.
    stalled: Option<Pin<Box<Sleep>>>,
}

impl<S> WriteStall<S> {
    pub fn new(inner: S, limit: Duration) -> Self {
        Self {
            inner,
            limit,
            stalled: None,
        }
    }

    /// What a write that waits answers: pending, until it has waited `limit`, then an error.
    fn waiting<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        let limit = self.limit;
        let stalled = self.stalled.get_or_insert_with(|| Box::pin(sleep(limit)));
        ready!(stalled.as_mut().poll(cx));
        self.stalled = None;
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the client took nothing of what was sent for too long",
        )))
    }

    /// What a write that went through answers: the wait, if there was one, is over.
    fn done<T>(&mut self, done: io::Result<T>) -> Poll<io::Result<T>> {
        self.stalled = None;
        Poll::Ready(done)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for WriteStall<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for WriteStall<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(done) => this.done(done),
            Poll::Pending => this.waiting(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write_vectored(cx, bufs) {
            Poll::Ready(done) => this.done(done),
            Poll::Pending => this.waiting(cx),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(done) => this.done(done),
            Poll::Pending => this.waiting(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_shutdown(cx) {
            Poll::Ready(done) => this.done(done),
            Poll::Pending => this.waiting(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_write_the_other_side_takes_nothing_of_fails_after_the_limit() {
        let (near, mut far) = duplex(16);
        let mut stream = WriteStall::new(near, Duration::from_secs(30));
        stream.write_all(&[0; 16]).await.expect("fits the buffer");
        let stalled = stream.write_all(&[0; 16]).await;
        assert_eq!(stalled.expect_err("fails").kind(), io::ErrorKind::TimedOut);
        let mut taken = [0; 16];
        far.read_exact(&mut taken).await.expect("what went through");
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_reader_keeps_the_writes_going() {
        let (near, mut far) = duplex(16);
        let mut stream = WriteStall::new(near, Duration::from_secs(30));
        let reader = tokio::spawn(async move {
            let mut taken = Vec::new();
            let mut buf = [0; 16];
            while taken.len() < 64 {
                // Each read comes well within the limit, though four of them take longer.
                sleep(Duration::from_secs(20)).await;
                let n = far.read(&mut buf).await.expect("reads");
                taken.extend_from_slice(&buf[..n]);
            }
            taken.len()
        });
        stream.write_all(&[0; 64]).await.expect("keeps going");
        assert_eq!(reader.await.expect("read"), 64);
    }
}
