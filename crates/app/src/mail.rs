//! The mail the server sends, behind the [`Mailer`] port.
//!
//! Use cases say what to send as a typed [`Message`], with the recipient. Writing the mail
//! itself, its subject, paragraphs and links to the app, and getting it to a mail server,
//! is the implementation's job. A message carries the secrets its links need, such as a
//! verification token, as [`Secret`]s, so `Debug` output of a mail never shows them.
//!
//! Use cases call [`Mailer::send`] after the change it tells of is committed, and outside
//! any transaction. An implementation may send at once or queue the mail; either way it
//! must not log a message's secrets, and should return quickly. A failed send is reported
//! where the use case can tell the caller, such as an invite that was made but not mailed,
//! and logged as a warning where it cannot.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use neon_domain::settings::Secret;
use time::OffsetDateTime;
use tracing::info;

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

/// One mail to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: Recipient,
    pub message: Message,
}

/// What a mail says, by kind. Every mail the server sends is one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The link verifying an account's address, sent after a self sign-up while sign-ups
    /// verify. The link opens the app's verification page with `token`.
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
}

/// Why a mail did not go. The message is for logs and admins, and never holds the mail's
/// secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MailError {
    #[error("mail is not set up")]
    NotConfigured,
    #[error("sending mail failed: {0}")]
    Failed(String),
}

/// Sends mail.
#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    /// Whether mail can go out now. The flows that cannot work without it, verifying
    /// sign-ups, resetting passwords, mailing invites and requests to join, are off while
    /// it is false. Read on every use, so a change to the mail settings applies at once.
    fn configured(&self) -> bool;

    async fn send(&self, mail: Mail) -> Result<(), MailError>;
}

/// Sends nothing: it logs each mail's kind and says mail is not set up, so every flow
/// that needs mail stays off.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    fn configured(&self) -> bool {
        false
    }

    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        info!(
            kind = mail.message.kind(),
            "mail not sent: no mail transport is set up"
        );
        Err(MailError::NotConfigured)
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

        let shown = format!("{mail:?}");

        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("grace@example.com"), "{shown}");
        assert_eq!(mail.message.kind(), "invite");
        assert_eq!(mail.to.to_string(), "Ada <ada@example.com>");
    }

    #[tokio::test]
    async fn the_log_mailer_sends_nothing() {
        let mailer = LogMailer;
        let mail = Mail {
            to: Recipient::new("ada@example.com", None),
            message: Message::Test {
                sent_by: "grace@example.com".into(),
            },
        };

        assert!(!mailer.configured());
        assert_eq!(mailer.send(mail).await, Err(MailError::NotConfigured));
    }
}
