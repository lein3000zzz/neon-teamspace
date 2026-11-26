//! The port outgoing webhooks are sent through, and what it answers.
//!
//! An implementation resolves the URL's host once per call, refuses it when any address it
//! resolves to is one [`check_ip`](neon_domain::webhooks::check_ip) refuses, then connects
//! to an address it checked, never resolving again, so a name that changes its answer in
//! between cannot swap in another. It follows no redirect, bounds the time to connect and
//! the time of the whole exchange, reads at most a small part of the answer, and sends a
//! fixed `User-Agent`.

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use neon_domain::webhooks::Blocked;
use url::Url;

/// The `User-Agent` of every webhook request.
pub const USER_AGENT: &str = "NeonTeamspace-Webhooks/1";

/// How long connecting may take, TLS included.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a whole delivery may take, from resolving the host to the end of what is read
/// of the answer.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);

/// The most of an answer's body that is read; the rest is dropped with the connection.
pub const MAX_ANSWER: usize = 64 * 1024;

/// The most of an answer's body a delivery log keeps, in bytes.
pub const MAX_EXCERPT: usize = 256;

/// A request to send: a `POST` of `body` to `url` with `headers`.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub url: Url,
    pub headers: Vec<(&'static str, String)>,
    pub body: Bytes,
}

/// What a receiver answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    /// The start of its body, as [`excerpt`] makes it.
    pub excerpt: Option<String>,
}

/// Why a request was not answered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    #[error("the URL's host does not resolve")]
    Unresolvable,
    #[error("the URL's host is {0}, which webhooks may not reach")]
    Refused(Blocked),
    #[error("the connection failed")]
    Connect,
    #[error("the TLS handshake failed")]
    Tls,
    #[error("the receiver did not answer in time")]
    Timeout,
    #[error("the receiver's answer is not HTTP")]
    Protocol,
}

impl SendError {
    /// A short, stable name, as delivery logs show it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unresolvable => "unresolvable",
            Self::Refused(_) => "address_refused",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Timeout => "timeout",
            Self::Protocol => "protocol",
        }
    }
}

/// Sends outgoing webhooks. `allow_private` is the setting `webhooks.allow_private` as it is
/// when the call is made.
#[async_trait]
pub trait WebhookTransport: Send + Sync + 'static {
    /// Resolves the URL's host and checks every address, as a delivery would, without
    /// connecting.
    async fn check(&self, url: &Url, allow_private: bool) -> Result<(), SendError>;

    /// Sends a request, as the module docs say.
    async fn send(&self, request: Outgoing, allow_private: bool) -> Result<Answer, SendError>;
}

/// The start of an answer's body for a delivery log: at most [`MAX_EXCERPT`] bytes, cut at a
/// character, read as UTF-8 with what does not read replaced, control characters but line
/// breaks and tabs replaced too. `None` for an empty body.
pub fn excerpt(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&body[..body.len().min(MAX_EXCERPT * 2)]);
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_control() && c != '\n' && c != '\t' {
            '\u{fffd}'
        } else {
            c
        };
        if out.len() + c.len_utf8() > MAX_EXCERPT {
            break;
        }
        out.push(c);
    }
    let trimmed = out.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts_are_short_readable_text() {
        assert_eq!(excerpt(b""), None);
        assert_eq!(excerpt(b"  \n"), None);
        assert_eq!(excerpt(b"ok\n"), Some("ok".to_owned()));
        assert_eq!(
            excerpt(b"a\x00b\x1bc"),
            Some("a\u{fffd}b\u{fffd}c".to_owned())
        );
        assert_eq!(excerpt(&[0xff, b'x']), Some("\u{fffd}x".to_owned()));
        let long = "é".repeat(MAX_EXCERPT);
        let cut = excerpt(long.as_bytes()).expect("text");
        assert!(cut.len() <= MAX_EXCERPT && cut.len() >= MAX_EXCERPT - 1);
        assert!(cut.chars().all(|c| c == 'é'));
    }

    #[test]
    fn kinds_are_stable_names() {
        assert_eq!(
            SendError::Refused(Blocked::Loopback).kind(),
            "address_refused"
        );
        assert_eq!(
            SendError::Refused(Blocked::Loopback).to_string(),
            "the URL's host is a loopback address, which webhooks may not reach"
        );
        assert_eq!(SendError::Timeout.kind(), "timeout");
    }
}
