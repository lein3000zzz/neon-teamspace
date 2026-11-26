use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_domain::settings::{Secret, Snapshot};
use neon_store::jobs::{self, Job, NewJob};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::debug;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::compose::{self, Sender};
use super::{Mail, Message, Notice, Outgoing, Recipient};
use crate::clock::Clock;
use crate::crypto::{Envelope, SealError};
use crate::settings::{INSTANCE_NAME, SettingsService, mail as keys};

/// The job kind of mail.
pub const KIND: &str = "mail.send";

/// The field the sealed mail is kept under in a job's payload: `{"sealed": "<base64>"}`.
const SEALED: &str = "sealed";

/// Queues mail, and says whether mail can go. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct MailQueue {
    inner: Arc<Inner>,
}

struct Inner {
    settings: Arc<SettingsService>,
    envelope: Envelope,
    /// The public URL's origin, without a trailing slash.
    origin: Option<String>,
    clock: Clock,
    deliveries: Mutex<Deliveries>,
}

/// Whether mail can go now, and why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailStatus {
    pub configured: bool,
    /// What is missing, while it cannot.
    pub problem: Option<String>,
    pub deliveries: Deliveries,
}

/// How the last deliveries went, since the server started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deliveries {
    /// When the mail server last took a mail.
    pub last_sent_at: Option<OffsetDateTime>,
    pub last_failure: Option<Failure>,
}

/// A delivery that failed, and what the mail server said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub at: OffsetDateTime,
    /// The kind of mail, as [`Message::kind`] names it.
    pub kind: &'static str,
    pub error: String,
    /// It will not be tried again.
    pub permanent: bool,
}

/// A mail as it waits in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub mail: Mail,
    /// By the queue's clock, to the second.
    pub queued_at: OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("mail cannot be queued: {0}")]
    Seal(#[from] SealError),
    #[error("queueing mail: {0}")]
    Database(#[from] sqlx::Error),
}

impl MailQueue {
    /// Mail sealed with `envelope`, its links pointing to `public_url`, which mail needs,
    /// with the time read from `clock`.
    pub fn new(
        settings: Arc<SettingsService>,
        envelope: Envelope,
        public_url: Option<&str>,
        clock: Clock,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                settings,
                envelope,
                origin: public_url.map(|url| url.trim_end_matches('/').to_owned()),
                clock,
                deliveries: Mutex::new(Deliveries::default()),
            }),
        }
    }

    /// Whether mail can go now. Read on every use, so a change to the mail settings
    /// applies at once.
    pub fn configured(&self) -> bool {
        self.problem().is_none()
    }

    pub fn status(&self) -> MailStatus {
        let problem = self.problem();
        MailStatus {
            configured: problem.is_none(),
            problem,
            deliveries: self.deliveries().clone(),
        }
    }

    fn problem(&self) -> Option<String> {
        let settings = self.inner.settings.current();
        let problem = if !*settings.get(keys::ENABLED) {
            "mail.enabled is off"
        } else if settings.get(keys::HOST).is_empty() {
            "mail.host is not set"
        } else if settings.get(keys::FROM).is_empty() {
            "mail.from is not set"
        } else if !self.inner.envelope.has_key() {
            "secrets.keys is not set: mail waits in the queue encrypted, as it carries \
             one-time links"
        } else if self.inner.origin.is_none() {
            "server.public_url is not set: the links in mail point there"
        } else {
            return None;
        };
        Some(problem.to_owned())
    }

    /// Queues `mail` on the caller's transaction, so it goes only if the change it tells
    /// of commits. The payload is sealed and bound to the job's id. Returns that id.
    pub async fn queue(&self, conn: &mut PgConnection, mail: &Mail) -> Result<Uuid, QueueError> {
        let id = Uuid::now_v7();
        let payload = Payload::new(mail, self.now());
        let plain = Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| SealError::Encrypt)?);
        let sealed = self.inner.envelope.seal(&plain, &aad(id))?;
        let payload = json!({ SEALED: STANDARD.encode(sealed) });
        jobs::enqueue(&mut *conn, &NewJob::new(KIND, payload).id(id)).await?;
        debug!(kind = mail.message.kind(), job_id = %id, "mail queued");
        Ok(id)
    }

    /// The mail a job carries. The messages say why it does not open, never what it holds.
    pub fn open(&self, job: &Job) -> Result<Queued, String> {
        let sealed = job
            .payload
            .get(SEALED)
            .and_then(Json::as_str)
            .and_then(|text| STANDARD.decode(text).ok())
            .ok_or_else(|| "the payload is not sealed mail".to_owned())?;
        let plain = self
            .inner
            .envelope
            .open(&sealed, &aad(job.id))
            .map_err(|error| format!("the payload does not open: {error}"))?;
        let payload: Payload = serde_json::from_slice(&plain)
            .map_err(|_| "the payload is not a mail this server knows".to_owned())?;
        Ok(payload.into())
    }

    /// `mail` written out with the settings of the moment. `None` while the public URL
    /// its links need is unset.
    pub fn write(&self, mail: &Mail) -> Option<Outgoing> {
        let origin = self.inner.origin.as_deref()?;
        let settings: Arc<Snapshot> = self.inner.settings.current();
        let sender = Sender {
            name: settings.get(INSTANCE_NAME),
            origin,
        };
        let letter = compose::write(&mail.message, &sender);
        Some(Outgoing {
            to: mail.to.clone(),
            text: letter.text(&sender),
            html: letter.html(&sender),
            unsubscribe: compose::unsubscribe_api(&mail.message, &sender),
            subject: letter.subject,
        })
    }

    pub(super) fn now(&self) -> OffsetDateTime {
        self.inner.clock.now()
    }

    pub(super) fn sent(&self) {
        self.deliveries().last_sent_at = Some(self.now());
    }

    pub(super) fn failed(&self, kind: &'static str, error: &str, permanent: bool) {
        self.deliveries().last_failure = Some(Failure {
            at: self.now(),
            kind,
            error: error.to_owned(),
            permanent,
        });
    }

    fn deliveries(&self) -> MutexGuard<'_, Deliveries> {
        self.inner
            .deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Binds sealed mail to its job, so it does not open as another's.
fn aad(id: Uuid) -> Vec<u8> {
    format!("job:{id}").into_bytes()
}

/// A mail as it is sealed into a job. Times are Unix seconds.
#[derive(Serialize, Deserialize)]
struct Payload {
    to: String,
    name: Option<String>,
    queued_at: i64,
    message: Body,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Body {
    Verification {
        token: String,
        lasts_secs: u64,
    },
    PasswordReset {
        token: String,
        lasts_secs: u64,
    },
    PasswordChanged {
        at: i64,
    },
    Invite {
        inviter_name: String,
        inviter_email: String,
        code: String,
        expires_at: i64,
    },
    JoinConfirmation {
        token: String,
        lasts_secs: u64,
    },
    JoinDenied {
        ask_again_after_secs: u64,
    },
    JoinRequested {
        requester: String,
        note: String,
    },
    Test {
        sent_by: String,
    },
    Notification {
        notice: SealedNotice,
        unsubscribe: String,
    },
    Digest {
        notices: Vec<SealedNotice>,
        more: usize,
        path: String,
        unsubscribe: String,
    },
}

/// A notice as it is sealed into a job.
#[derive(Serialize, Deserialize)]
struct SealedNotice {
    summary: String,
    excerpt: Option<String>,
    path: String,
}

impl From<&Notice> for SealedNotice {
    fn from(notice: &Notice) -> Self {
        Self {
            summary: notice.summary.clone(),
            excerpt: notice.excerpt.clone(),
            path: notice.path.clone(),
        }
    }
}

impl From<SealedNotice> for Notice {
    fn from(notice: SealedNotice) -> Self {
        Self {
            summary: notice.summary,
            excerpt: notice.excerpt,
            path: notice.path,
        }
    }
}

impl Payload {
    fn new(mail: &Mail, queued_at: OffsetDateTime) -> Self {
        let token = |secret: &Secret| secret.expose().to_owned();
        let message = match &mail.message {
            Message::Verification { token: t, lasts } => Body::Verification {
                token: token(t),
                lasts_secs: lasts.as_secs(),
            },
            Message::PasswordReset { token: t, lasts } => Body::PasswordReset {
                token: token(t),
                lasts_secs: lasts.as_secs(),
            },
            Message::PasswordChanged { at } => Body::PasswordChanged {
                at: at.unix_timestamp(),
            },
            Message::Invite {
                inviter_name,
                inviter_email,
                code,
                expires_at,
            } => Body::Invite {
                inviter_name: inviter_name.clone(),
                inviter_email: inviter_email.clone(),
                code: token(code),
                expires_at: expires_at.unix_timestamp(),
            },
            Message::JoinConfirmation { token: t, lasts } => Body::JoinConfirmation {
                token: token(t),
                lasts_secs: lasts.as_secs(),
            },
            Message::JoinDenied { ask_again_after } => Body::JoinDenied {
                ask_again_after_secs: ask_again_after.as_secs(),
            },
            Message::JoinRequested { requester, note } => Body::JoinRequested {
                requester: requester.clone(),
                note: note.clone(),
            },
            Message::Test { sent_by } => Body::Test {
                sent_by: sent_by.clone(),
            },
            Message::Notification {
                notice,
                unsubscribe,
            } => Body::Notification {
                notice: notice.into(),
                unsubscribe: token(unsubscribe),
            },
            Message::Digest {
                notices,
                more,
                path,
                unsubscribe,
            } => Body::Digest {
                notices: notices.iter().map(SealedNotice::from).collect(),
                more: *more,
                path: path.clone(),
                unsubscribe: token(unsubscribe),
            },
        };
        Self {
            to: mail.to.address.clone(),
            name: mail.to.name.clone(),
            queued_at: queued_at.unix_timestamp(),
            message,
        }
    }
}

impl From<Payload> for Queued {
    fn from(payload: Payload) -> Self {
        let at = |secs: i64| {
            OffsetDateTime::from_unix_timestamp(secs).unwrap_or(OffsetDateTime::UNIX_EPOCH)
        };
        let message = match payload.message {
            Body::Verification { token, lasts_secs } => Message::Verification {
                token: Secret::new(token),
                lasts: Duration::from_secs(lasts_secs),
            },
            Body::PasswordReset { token, lasts_secs } => Message::PasswordReset {
                token: Secret::new(token),
                lasts: Duration::from_secs(lasts_secs),
            },
            Body::PasswordChanged { at: secs } => Message::PasswordChanged { at: at(secs) },
            Body::Invite {
                inviter_name,
                inviter_email,
                code,
                expires_at,
            } => Message::Invite {
                inviter_name,
                inviter_email,
                code: Secret::new(code),
                expires_at: at(expires_at),
            },
            Body::JoinConfirmation { token, lasts_secs } => Message::JoinConfirmation {
                token: Secret::new(token),
                lasts: Duration::from_secs(lasts_secs),
            },
            Body::JoinDenied {
                ask_again_after_secs,
            } => Message::JoinDenied {
                ask_again_after: Duration::from_secs(ask_again_after_secs),
            },
            Body::JoinRequested { requester, note } => Message::JoinRequested { requester, note },
            Body::Test { sent_by } => Message::Test { sent_by },
            Body::Notification {
                notice,
                unsubscribe,
            } => Message::Notification {
                notice: notice.into(),
                unsubscribe: Secret::new(unsubscribe),
            },
            Body::Digest {
                notices,
                more,
                path,
                unsubscribe,
            } => Message::Digest {
                notices: notices.into_iter().map(Notice::from).collect(),
                more,
                path,
                unsubscribe: Secret::new(unsubscribe),
            },
        };
        Self {
            mail: Mail {
                to: Recipient::new(payload.to, payload.name),
                message,
            },
            queued_at: at(payload.queued_at),
        }
    }
}
