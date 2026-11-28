//! Accounts and how they sign in: registration, sessions, API tokens, invites, requests to
//! join, verification and reset links, the admin's work on accounts, and the cleanup that
//! removes what expired.
//!
//! Every public use case of [`AuthService`] runs in a span named `auth <operation>` and is
//! measured as RED metrics of [`SUBSYSTEM`]. An expected error, such as a wrong password,
//! is recorded on the span without failing it and counts as `ok`, like a 4xx answer does
//! for HTTP; only failures of the server count as `error`.
//!
//! The security properties follow from a few rules kept throughout:
//!
//! - Secrets that prove who someone is, session and API tokens, invite codes, mailed
//!   links, are random 256-bit values kept only as their SHA-256.
//! - Passwords are argon2id, checked off the async runtime, as many at once as the machine
//!   has processors: each takes its memory, 19 MiB by default, while it runs. One that
//!   gets no turn within [`HASH_WAIT`] is refused as [`AuthError::Busy`]. An unknown
//!   address is checked against a dummy hash, so it takes as long as a wrong password and
//!   waits for a turn the same way, and a disabled or unverified account is only revealed
//!   after the right password.
//! - The forms anyone can send, asking for a reset link or to join, answer the same
//!   whatever comes of them, and do the work after the answer.
//! - Mail is queued on the transaction of the change it tells of, sealed, so a link exists
//!   in the clear only in the mail itself.
//! - No password, token or code reaches a log line, a span or an error message.

mod accounts;
mod admin;
mod cleanup;
mod error;
mod invites;
mod requests;
mod reset;
mod sessions;
mod tokens;
mod verification;

use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use neon_domain::auth::{
    PasswordPolicy, Principal, RegistrationMode, SessionPolicy,
    password::DEFAULT_MIN_PASSWORD_LENGTH,
};
use neon_domain::network::client_source;
use neon_domain::settings::{Secret, Snapshot};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use sqlx::{PgConnection, PgPool};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::task::TaskTracker;
use tracing::{Instrument as _, info_span, warn};

use crate::clock::Clock;
use crate::crypto::{Argon2Params, Envelope, PasswordHasher, Verified};
use crate::mail::{Mail, MailQueue, MailStatus, QueueError};
use crate::pages::{AccessChange, AccessChanges, NoListener};
use crate::ratelimit::{Bucket, MemoryLimiter, RateLimiter};
use crate::settings::{SettingsService, auth as keys};

pub use accounts::{NewAccount, ProfileUpdate, RegisterInput, Registration};
pub use admin::{UserPage, UserUpdate};
pub use cleanup::{CLEANUP_INTERVAL, Cleanup, CleanupReport};
pub use error::AuthError;
pub use invites::{CreatedInvite, InviteEntry, InviteSpec};
pub use sessions::{LoginInput, LoginOutcome};
pub(crate) use tokens::{TokenSpec, mint};

/// RED subsystem of the auth use cases.
pub const SUBSYSTEM: &str = "auth";

/// How long work done after an answer, such as mailing a reset link, may take.
const BACKGROUND_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a password waits for a turn to be hashed or checked before it is refused.
pub const HASH_WAIT: Duration = Duration::from_secs(2);

/// The password the dummy hash is made of. Nobody signs in with it: no account has it.
const DUMMY_PASSWORD: &str = "neon-dummy-password";

/// What the sign-in and sign-up pages need to know: which forms to show and what a
/// password needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthConfig {
    pub registration: RegistrationMode,
    pub min_password_length: usize,
    /// Self sign-ups verify their address before they sign in.
    pub verify_email: bool,
    /// Forgotten passwords can be reset by mail.
    pub password_reset: bool,
    /// People without an invite can ask for one.
    pub requests: bool,
    /// Admins can mail invites.
    pub invite_email: bool,
}

/// The auth use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct AuthService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    hasher: PasswordHasher,
    /// The turns to hash or check a password, one per hash running.
    hashing: Arc<Semaphore>,
    /// Checked against when an address is unknown, so a miss costs a full hash too.
    dummy_hash: String,
    limiter: Arc<dyn RateLimiter>,
    mail: MailQueue,
    envelope: Envelope,
    clock: Clock,
    access: Arc<dyn AccessChanges>,
    background: TaskTracker,
    red: Red,
}

/// Puts an [`AuthService`] together. Without being told otherwise it limits rates in
/// memory, sends no mail, hashes with [`Argon2Params::DEFAULT`], as many passwords at once
/// as the machine has processors, seals nothing, tells nobody of sessions and tokens that
/// end, and reads the system clock.
pub struct AuthServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    params: Argon2Params,
    hash_turns: Option<Arc<Semaphore>>,
    limiter: Option<Arc<dyn RateLimiter>>,
    mail: Option<MailQueue>,
    envelope: Envelope,
    clock: Clock,
    access: Arc<dyn AccessChanges>,
}

impl AuthServiceBuilder {
    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Where mail is queued. Without one, no mail can go: it has no public URL to link to.
    pub fn mail(mut self, mail: MailQueue) -> Self {
        self.mail = Some(mail);
        self
    }

    /// Seals invite codes, so admins can copy them again.
    pub fn envelope(mut self, envelope: Envelope) -> Self {
        self.envelope = envelope;
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Who hears that someone's sessions or tokens ended, or their account changed, as
    /// the page and workspace services' changes tell it too: what holds pages open for
    /// them looks again.
    pub fn access_changes(mut self, access: Arc<dyn AccessChanges>) -> Self {
        self.access = access;
        self
    }

    /// The cost of new password hashes. Hashes made with other parameters are made again
    /// with these as their users sign in.
    pub fn password_params(mut self, params: Argon2Params) -> Self {
        self.params = params;
        self
    }

    /// The turns passwords are hashed and checked in, one per hash running, which the
    /// service may share with whoever made them.
    pub fn hash_turns(mut self, turns: Arc<Semaphore>) -> Self {
        self.hash_turns = Some(turns);
        self
    }

    /// Makes the dummy hash, which takes as long as any password hash. Call it after
    /// telemetry is installed, as it creates the RED metrics.
    pub async fn build(self) -> Result<AuthService, AuthError> {
        let hasher = PasswordHasher::new(self.params)
            .map_err(|error| AuthError::Internal(error.to_string()))?;
        let dummy_hash = blocking({
            let hasher = hasher.clone();
            move || hasher.hash(DUMMY_PASSWORD)
        })
        .await?;
        let mail = self.mail.unwrap_or_else(|| {
            MailQueue::new(
                Arc::clone(&self.settings),
                self.envelope.clone(),
                None,
                self.clock.clone(),
            )
        });
        Ok(AuthService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                hasher,
                hashing: self.hash_turns.unwrap_or_else(|| {
                    let processors =
                        std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
                    Arc::new(Semaphore::new(processors))
                }),
                dummy_hash,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                mail,
                envelope: self.envelope,
                clock: self.clock,
                access: self.access,
                background: TaskTracker::new(),
                red: Red::new(SUBSYSTEM),
            }),
        })
    }
}

impl AuthService {
    pub fn builder(pool: PgPool, settings: Arc<SettingsService>) -> AuthServiceBuilder {
        AuthServiceBuilder {
            pool,
            settings,
            params: Argon2Params::DEFAULT,
            hash_turns: None,
            limiter: None,
            mail: None,
            envelope: Envelope::default(),
            clock: Clock::system(),
            access: Arc::new(NoListener),
        }
    }

    /// Which forms are on, as the settings and the mail set-up say now.
    pub fn config(&self) -> AuthConfig {
        let settings = self.settings();
        AuthConfig {
            registration: registration(&settings),
            min_password_length: self.policy().min_length,
            verify_email: self.verifying(),
            password_reset: self.password_reset_available(),
            requests: self.requests_open(),
            invite_email: self.invite_email_available(),
        }
    }

    /// Waits for the work started after earlier answers, such as mail sent after a request
    /// to join, to finish. Shutting down calls it, and so do tests.
    pub async fn settle(&self) {
        let background = &self.inner.background;
        background.close();
        background.wait().await;
        background.reopen();
    }

    fn settings(&self) -> Arc<Snapshot> {
        self.inner.settings.current()
    }

    fn now(&self) -> time::OffsetDateTime {
        self.inner.clock.now()
    }

    fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    /// Tells the port that a session or token of `user_id` ended, or their account
    /// changed, once the change committed.
    fn ended(&self, user_id: uuid::Uuid) {
        self.inner
            .access
            .access_changed(AccessChange::Account { user_id });
    }

    fn policy(&self) -> PasswordPolicy {
        let least = *self.settings().get(keys::MIN_PASSWORD_LENGTH);
        let fallback = usize::try_from(DEFAULT_MIN_PASSWORD_LENGTH).unwrap_or(12);
        PasswordPolicy {
            min_length: usize::try_from(least).unwrap_or(fallback),
        }
    }

    fn session_policy(&self) -> SessionPolicy {
        let settings = self.settings();
        SessionPolicy {
            idle: *settings.get(keys::SESSION_IDLE_TTL),
            absolute: *settings.get(keys::SESSION_ABSOLUTE_TTL),
        }
    }

    fn invite_ttl(&self) -> Duration {
        *self.settings().get(keys::INVITE_TTL)
    }

    fn mail_configured(&self) -> bool {
        self.inner.mail.configured()
    }

    /// Whether mail can go now, why not, and how the last deliveries went.
    pub fn mail_status(&self) -> MailStatus {
        self.inner.mail.status()
    }

    /// Self sign-ups verify their address now: switched on, and mail set up.
    fn verifying(&self) -> bool {
        *self.settings().get(keys::VERIFY_EMAIL) && self.mail_configured()
    }

    /// Passwords can be reset by mail now: switched on, and mail set up.
    pub fn password_reset_available(&self) -> bool {
        *self.settings().get(keys::PASSWORD_RESET) && self.mail_configured()
    }

    /// Invites can be mailed now: switched on, and mail set up.
    pub fn invite_email_available(&self) -> bool {
        *self.settings().get(keys::INVITE_EMAIL) && self.mail_configured()
    }

    /// People can ask to join now: switched on, registration by invite, and mail set up,
    /// which carries the confirmation and the invite.
    pub fn requests_open(&self) -> bool {
        let settings = self.settings();
        *settings.get(keys::REQUESTS)
            && registration(&settings) == RegistrationMode::Invite
            && self.mail_configured()
    }

    /// Takes an attempt from the bucket of the client at `ip`, counted by its
    /// [`client_source`]: an IPv4 address, or the /64 an IPv6 address is in.
    async fn limit_source(&self, bucket: Bucket, ip: &str) -> Result<(), AuthError> {
        self.limit(bucket, &client_source(ip)).await
    }

    async fn limit(&self, bucket: Bucket, key: &str) -> Result<(), AuthError> {
        self.inner
            .limiter
            .check(bucket, key)
            .await
            .map_err(|limited| {
                warn!(bucket = bucket.prefix(), "rate limited");
                AuthError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    async fn hash_password(&self, password: &Secret) -> Result<String, AuthError> {
        let turn = self.hash_turn().await?;
        let hasher = self.inner.hasher.clone();
        let password = password.clone();
        blocking(move || {
            let _turn = turn;
            hasher.hash(password.expose())
        })
        .await
    }

    async fn verify_password(
        &self,
        password: &Secret,
        hash: String,
    ) -> Result<Verified, AuthError> {
        let turn = self.hash_turn().await?;
        let hasher = self.inner.hasher.clone();
        let password = password.clone();
        blocking(move || {
            let _turn = turn;
            hasher.verify(password.expose(), &hash)
        })
        .await
    }

    /// A turn to hash or check a password, which the work holds until it ends, even when
    /// its caller is gone; [`AuthError::Busy`] when none comes within [`HASH_WAIT`].
    async fn hash_turn(&self) -> Result<OwnedSemaphorePermit, AuthError> {
        let hashing = Arc::clone(&self.inner.hashing);
        match tokio::time::timeout(HASH_WAIT, hashing.acquire_owned()).await {
            Ok(Ok(turn)) => Ok(turn),
            // The semaphore is never closed; a password waiting too long is refused.
            Ok(Err(_)) | Err(_) => {
                warn!("too many passwords being checked at once");
                Err(AuthError::Busy)
            }
        }
    }

    /// Costs what checking a real password does, waits for a turn as one does, and finds
    /// nothing.
    async fn verify_dummy(&self, password: &Secret) -> Result<(), AuthError> {
        let dummy = self.inner.dummy_hash.clone();
        match self.verify_password(password, dummy).await {
            Ok(_) => Ok(()),
            Err(AuthError::Busy) => Err(AuthError::Busy),
            Err(error) => {
                warn!(%error, "checking against the dummy hash failed");
                Ok(())
            }
        }
    }

    /// Queues a mail on `conn`, the transaction of the change it tells of.
    async fn queue(&self, conn: &mut PgConnection, mail: &Mail) -> Result<(), AuthError> {
        match self.inner.mail.queue(conn, mail).await {
            Ok(_) => Ok(()),
            Err(QueueError::Database(error)) => Err(AuthError::Database(error)),
            Err(error @ QueueError::Seal(_)) => Err(AuthError::Internal(error.to_string())),
        }
    }

    /// Runs `work` after the answer, in a span of its own under the current one, for at
    /// most [`BACKGROUND_TIMEOUT`].
    fn background(&self, operation: &'static str, work: impl Future<Output = ()> + Send + 'static) {
        let span = info_span!(
            "auth background",
            otel.name = %format!("auth {operation}"),
            auth.operation = operation,
        );
        self.inner.background.spawn(
            async move {
                if tokio::time::timeout(BACKGROUND_TIMEOUT, work)
                    .await
                    .is_err()
                {
                    warn!("background work did not finish in time and was dropped");
                }
            }
            .instrument(span),
        );
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, AuthError>>,
    ) -> Result<T, AuthError> {
        let span = info_span!(
            "auth",
            otel.name = %format!("auth {operation}"),
            auth.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }
}

fn registration(settings: &Snapshot) -> RegistrationMode {
    RegistrationMode::parse(settings.get(keys::REGISTRATION)).unwrap_or(RegistrationMode::DEFAULT)
}

fn require_admin(principal: &Principal) -> Result<(), AuthError> {
    if principal.is_admin() {
        Ok(())
    } else {
        Err(AuthError::Forbidden("only an admin may do this"))
    }
}

fn require_session(principal: &Principal) -> Result<(), AuthError> {
    if principal.via_session() {
        Ok(())
    } else {
        Err(AuthError::Forbidden(
            "this needs a browser session: API tokens cannot manage passwords, sessions or tokens",
        ))
    }
}

/// Runs CPU-heavy work, such as hashing, where blocking is allowed.
async fn blocking<T, E>(
    work: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, AuthError>
where
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AuthError::Internal(format!("password hashing stopped: {error}")))?
        .map_err(|error| AuthError::Internal(error.to_string()))
}
