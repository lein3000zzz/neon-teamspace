//! The mail the server sends: what each mail says, how it is written, and how it gets to
//! a mail server through the job queue.
//!
//! A use case says what to send as a typed [`Mail`] and queues it with
//! [`MailQueue::queue`] on the transaction of the change it tells of, so a change that rolls
//! back sends nothing. The job's payload is sealed with the [`Envelope`](crate::crypto::Envelope),
//! as a mail can carry a one-time link, and bound to the job's row; finished mail jobs are
//! deleted rather than kept. [`MailJob`] runs the jobs: it opens the payload, writes the
//! mail with the settings of the moment ([`compose`]), and hands it to the [`Mailer`] port,
//! behind which the SMTP adapter sits. A mail server that refuses the address ends the job
//! for good; any other failure is retried with backoff.
//!
//! Mailing needs four things, which [`MailQueue::status`] checks: mail switched on with a
//! host and a From address, a secret key to seal the queued mail, and the public URL the
//! links point to.

pub mod compose;

mod color;
mod job;
mod queue;

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use neon_domain::settings::Secret;
use time::OffsetDateTime;

pub use job::MailJob;
pub use queue::{Deliveries, Failure, KIND, MailQueue, MailStatus, QueueError, Queued};

/// Who a mail goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub address: String,
    /// Their display name, when they have an account.
    pub name: Option<String>,
}

impl Recipient {
    pub fn new(address: impl Into<String>, name: Option<String>) -> Self {
        Self {
            address: address.into(),
            name,
        }
    }
}

impl fmt::Display for Recipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name} <{}>", self.address),
            None => f.write_str(&self.address),
        }
    }
}

/// One mail to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: Recipient,
    pub message: Message,
}

/// What a mail says, by kind. Every mail the server sends is one of these. The secrets
/// their links need are [`Secret`]s, so `Debug` output of a mail never shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The link verifying an account's address, sent after a self sign-up while sign-ups
    /// verify. It opens the app's verification page with `token`.
    Verification { token: Secret, lasts: Duration },
    /// The link setting a new password, which the app's reset page takes with `token`.
    PasswordReset { token: Secret, lasts: Duration },
    /// Tells an account its password changed at `at`, by its owner or by a reset link, so
    /// a change they did not make does not go unnoticed.
    PasswordChanged { at: OffsetDateTime },
    /// An invite to sign up with `code`, naming the admin who sent it. Only the recipient's
    /// address signs up with it, once.
    Invite {
        inviter_name: String,
        inviter_email: String,
        code: Secret,
        expires_at: OffsetDateTime,
    },
    /// The link confirming a request to join, which the app's request page takes with
    /// `token`. Admins see the request only once it is confirmed.
    JoinConfirmation { token: Secret, lasts: Duration },
    /// Tells someone an admin denied their request to join, and when they may ask again.
    JoinDenied { ask_again_after: Duration },
    /// Tells an admin someone confirmed a request to join, waiting for them.
    JoinRequested { requester: String, note: String },
    /// A test, sent by an admin to see whether mail goes through.
    Test { sent_by: String },
}

impl Message {
    /// A stable name for the kind, for logs and metrics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Verification { .. } => "verification",
            Self::PasswordReset { .. } => "password_reset",
            Self::PasswordChanged { .. } => "password_changed",
            Self::Invite { .. } => "invite",
            Self::JoinConfirmation { .. } => "join_confirmation",
            Self::JoinDenied { .. } => "join_denied",
            Self::JoinRequested { .. } => "join_requested",
            Self::Test { .. } => "test",
        }
    }

    /// When the link in it stops working, for a mail queued at `queued_at`: past then,
    /// delivering it would only hand over a dead link.
    pub fn link_expires(&self, queued_at: OffsetDateTime) -> Option<OffsetDateTime> {
        match self {
            Self::Verification { lasts, .. }
            | Self::PasswordReset { lasts, .. }
            | Self::JoinConfirmation { lasts, .. } => Some(queued_at + *lasts),
            Self::Invite { expires_at, .. } => Some(*expires_at),
            Self::PasswordChanged { .. }
            | Self::JoinDenied { .. }
            | Self::JoinRequested { .. }
            | Self::Test { .. } => None,
        }
    }
}

/// A mail written out, ready for a mail server. Its text and HTML hold the mail's links, so
/// `Debug` shows only who it goes to and its subject.
#[derive(Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub to: Recipient,
    pub subject: String,
    pub text: String,
    pub html: String,
}

impl fmt::Debug for Outgoing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outgoing")
            .field("to", &self.to)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

/// Why a mail did not go. The messages are for logs and admins: what the mail server
/// answered, never the mail's links or the password used to sign in.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// Mail is not set up now. It may be later, so the mail is tried again.
    #[error("mail is not set up: {0}")]
    NotConfigured(String),
    /// The mail server refused the recipient, or the mail itself, for good: sending it
    /// again cannot help.
    #[error("the mail server refused it: {0}")]
    Rejected(String),
    /// Anything else, such as a server that cannot be reached or refuses the sign-in.
    #[error("{0}")]
    Failed(String),
}

/// Hands written mail to a mail server. The mail job calls it; use cases queue mail
/// instead, with [`MailQueue`].
#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    async fn send(&self, mail: &Outgoing) -> Result<(), SendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_secrets() {
        let mail = Mail {
            to: Recipient::new("ada@example.com", Some("Ada".into())),
            message: Message::Invite {
                inviter_name: "Grace".into(),
                inviter_email: "grace@example.com".into(),
                code: Secret::new("inv_hunter2"),
                expires_at: OffsetDateTime::UNIX_EPOCH,
            },
        };
        let outgoing = Outgoing {
            to: mail.to.clone(),
            subject: "Grace invited you".into(),
            text: "https://neon.example.com/register?invite=inv_hunter2".into(),
            html: "<a href=\"https://neon.example.com/register?invite=inv_hunter2\">".into(),
        };

        let shown = format!("{mail:?} {outgoing:?}");

        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("grace@example.com"), "{shown}");
        assert!(shown.contains("Grace invited you"), "{shown}");
        assert_eq!(mail.message.kind(), "invite");
        assert_eq!(mail.to.to_string(), "Ada <ada@example.com>");
    }

    #[test]
    fn links_expire_with_their_message() {
        let queued = OffsetDateTime::UNIX_EPOCH;
        let hour = Duration::from_secs(3600);
        let reset = Message::PasswordReset {
            token: Secret::new("t"),
            lasts: hour,
        };
        let invite = Message::Invite {
            inviter_name: "Grace".into(),
            inviter_email: "grace@example.com".into(),
            code: Secret::new("c"),
            expires_at: queued + 2 * hour,
        };

        assert_eq!(reset.link_expires(queued), Some(queued + hour));
        assert_eq!(invite.link_expires(queued), Some(queued + 2 * hour));
        assert_eq!(
            Message::Test {
                sent_by: "grace@example.com".into()
            }
            .link_expires(queued),
            None
        );
    }
}
