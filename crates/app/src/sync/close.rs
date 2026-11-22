//! Why the server closes a connection: a WebSocket close code and a reason to show.

use std::fmt;

/// Why the server ends a connection to a live document. Each has a WebSocket close code:
/// the standard ones where one fits, else one of the 4000s, numbered after the HTTP status
/// that means the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseReason {
    /// The server is stopping. Reconnect.
    Restarting,
    /// A text message: the protocol is binary.
    UnsupportedData,
    /// A message that is not a y-sync or awareness message.
    InvalidMessage,
    /// A message larger than the server takes.
    MessageTooBig,
    /// The server could not load or save the document.
    ServerError,
    /// A change from a connection that may only view the page.
    ReadOnly,
    /// The person may no longer see the page: it was trashed or deleted, a share was
    /// removed, or they left its teamspace or workspace.
    AccessLost,
    /// The client did not read what the server sent fast enough.
    TooSlow,
    /// The change would make the document larger than the server keeps.
    DocumentTooLarge,
    /// More messages than the server takes in a while.
    TooManyMessages,
    /// The person, or the page, has as many connections open as the server takes.
    TooManyConnections,
    /// The session or API token the connection opened with ended, or its account was
    /// disabled.
    SignedOut,
    /// An update that needs changes the server does not have: y-websocket clients send
    /// theirs in order, and one that reconnects syncs what is missing.
    OutOfOrder,
}

impl CloseReason {
    /// Every reason, for docs and tests.
    pub const ALL: [Self; 13] = [
        Self::Restarting,
        Self::UnsupportedData,
        Self::InvalidMessage,
        Self::MessageTooBig,
        Self::ServerError,
        Self::ReadOnly,
        Self::AccessLost,
        Self::TooSlow,
        Self::DocumentTooLarge,
        Self::TooManyMessages,
        Self::TooManyConnections,
        Self::SignedOut,
        Self::OutOfOrder,
    ];

    /// The WebSocket close code.
    pub fn code(self) -> u16 {
        match self {
            Self::UnsupportedData => 1003,
            Self::InvalidMessage => 1007,
            Self::MessageTooBig => 1009,
            Self::ServerError => 1011,
            Self::Restarting => 1012,
            Self::TooManyConnections => 1013,
            Self::SignedOut => 4401,
            Self::ReadOnly => 4403,
            Self::AccessLost => 4404,
            Self::TooSlow => 4408,
            Self::DocumentTooLarge => 4413,
            Self::OutOfOrder => 4422,
            Self::TooManyMessages => 4429,
        }
    }

    /// The reason sent with the code. Close frames hold at most 123 bytes of it.
    pub fn message(self) -> &'static str {
        match self {
            Self::Restarting => "the server is restarting: reconnect",
            Self::UnsupportedData => "only binary messages are understood",
            Self::InvalidMessage => "not a y-sync or awareness message",
            Self::MessageTooBig => "the message is too large",
            Self::ServerError => "the server could not load or save the document",
            Self::ReadOnly => "you may only view this page",
            Self::AccessLost => "you may no longer see this page",
            Self::TooSlow => "the connection fell too far behind",
            Self::DocumentTooLarge => "the page would grow larger than the server keeps",
            Self::TooManyMessages => "too many messages: slow down",
            Self::TooManyConnections => {
                "too many connections open: close some pages, then reconnect"
            }
            Self::SignedOut => "your session or token ended: sign in again",
            Self::OutOfOrder => "the update needs changes the server does not have: reconnect",
        }
    }

    /// A short name, for logs and metric labels.
    pub fn name(self) -> &'static str {
        match self {
            Self::Restarting => "restarting",
            Self::UnsupportedData => "unsupported_data",
            Self::InvalidMessage => "invalid_message",
            Self::MessageTooBig => "message_too_big",
            Self::ServerError => "server_error",
            Self::ReadOnly => "read_only",
            Self::AccessLost => "access_lost",
            Self::TooSlow => "too_slow",
            Self::DocumentTooLarge => "document_too_large",
            Self::TooManyMessages => "too_many_messages",
            Self::TooManyConnections => "too_many_connections",
            Self::SignedOut => "signed_out",
            Self::OutOfOrder => "out_of_order",
        }
    }
}

impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message(), self.code())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn each_reason_has_its_own_code_and_a_short_message() {
        let codes: HashSet<u16> = CloseReason::ALL.iter().map(|r| r.code()).collect();
        let names: HashSet<&str> = CloseReason::ALL.iter().map(|r| r.name()).collect();

        assert_eq!(codes.len(), CloseReason::ALL.len());
        assert_eq!(names.len(), CloseReason::ALL.len());
        for reason in CloseReason::ALL {
            assert!(reason.message().len() <= 123, "{reason:?}");
            let code = reason.code();
            assert!(
                matches!(code, 1003 | 1007 | 1009 | 1011..=1013 | 4000..=4999),
                "{code}"
            );
        }
        assert_eq!(
            CloseReason::ReadOnly.to_string(),
            "you may only view this page (4403)"
        );
    }
}
