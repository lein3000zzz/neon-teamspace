//! The OAuth 2.1 authorization server that MCP clients and other programs connect through:
//! registering clients, the consent of a person signed in at a browser, the token endpoint,
//! revocation, the connected apps people and workspace admins see, and the hourly cleanup.
//!
//! - **Clients** register themselves and are public: PKCE with S256 stands in for a
//!   secret. Registering is rate limited by source, an IPv4 address or an IPv6 /64, and
//!   the clients nobody connected are bounded in number, for each source and for all, and
//!   removed after a while.
//! - **Consent** happens on the web app's page, from a browser session alone: the person
//!   chooses a workspace, whom the client acts as, themselves or one of their agents that
//!   works, as `mcp.identities` and the workspace's agent switch allow, and its scopes.
//!   Errors in a request are shown to the person, never sent to the client's redirect URI
//!   on their own, as anyone may register one; only approving and denying send the browser
//!   back.
//! - **Codes** work once, for a minute, and only with the PKCE verifier, the redirect URI
//!   and the client they were given for. A code used twice ends its grant, and every token
//!   issued from it.
//! - **Tokens**: an access token for `oauth.access_token_ttl`, an hour by default, and a
//!   refresh token for `oauth.refresh_token_ttl`, 30 days, which works once and is replaced
//!   each time. A refresh token used twice ends its grant. Refreshing never widens the
//!   scopes. Each grant exchanges a few at a time, as each refresh token spent is kept
//!   until it expires, to catch a second use.
//! - **Every use** reads the grant again: its tokens stop at once when it is revoked, the
//!   person is disabled or out of its workspace, the agent it acts as is suspended, deleted,
//!   handed to someone else or switched off, or `mcp.identities` drops its mode. The bearer
//!   authentication of [`AuthService`](crate::auth::AuthService) resolves an access token
//!   to the same principal as an API token bound to the workspace, acting as the person or
//!   the agent, so every rule applies to it alike.
//!
//! Every public use case runs in a span named `oauth <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`]: a refusal is `ok`, only failures of the server are `error`.
//! No code, token, verifier or secret reaches a log line, a span or an error message.

pub(crate) mod access;
mod apps;
mod authorize;
mod cleanup;
mod error;
mod tokens;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neon_domain::settings::Snapshot;
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use sqlx::PgPool;
use tracing::{Instrument as _, info_span, warn};
use uuid::Uuid;

pub use apps::ConnectedApp;
pub use authorize::{Approval, Consent, ConsentWorkspace};
pub use cleanup::{Cleanup, CleanupReport};
pub use error::OAuthError;
pub use tokens::{IssuedTokens, TokenRequest};

use crate::clock::Clock;
use crate::pages::{AccessChange, AccessChanges, NoListener};
use crate::ratelimit::{Bucket, MemoryLimiter, RateLimiter};
use crate::settings::{SettingsService, oauth as keys};

/// RED subsystem of the OAuth use cases.
pub const SUBSYSTEM: &str = "oauth";

/// The OAuth use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct OAuthService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    limiter: Arc<dyn RateLimiter>,
    clock: Clock,
    access: Arc<dyn AccessChanges>,
    red: Red,
}

/// Puts an [`OAuthService`] together. Without being told otherwise it limits rates in
/// memory, tells nobody of grants that end, and reads the system clock.
pub struct OAuthServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    limiter: Option<Arc<dyn RateLimiter>>,
    clock: Clock,
    access: Arc<dyn AccessChanges>,
}

impl OAuthServiceBuilder {
    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Who hears that a grant ended, as the auth service tells it of tokens that end:
    /// what its tokens hold open looks again, and closes.
    pub fn access_changes(mut self, access: Arc<dyn AccessChanges>) -> Self {
        self.access = access;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> OAuthService {
        OAuthService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                clock: self.clock,
                access: self.access,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl OAuthService {
    pub fn builder(pool: PgPool, settings: Arc<SettingsService>) -> OAuthServiceBuilder {
        OAuthServiceBuilder {
            pool,
            settings,
            limiter: None,
            clock: Clock::system(),
            access: Arc::new(NoListener),
        }
    }

    /// Whether clients may register themselves now, as `oauth.registration` says: the
    /// metadata names the registration endpoint only then.
    pub fn registration_open(&self) -> bool {
        *self.settings().get(keys::REGISTRATION)
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

    fn access_ttl(&self) -> Duration {
        *self.settings().get(keys::ACCESS_TOKEN_TTL)
    }

    fn refresh_ttl(&self) -> Duration {
        *self.settings().get(keys::REFRESH_TOKEN_TTL)
    }

    /// Tells the port that the tokens of a grant acting as `user_id` ended, once that
    /// committed: the connections they opened look again, and close.
    fn ended(&self, user_id: Uuid) {
        self.inner
            .access
            .access_changed(AccessChange::Account { user_id });
    }

    async fn limit(&self, bucket: Bucket, key: &str) -> Result<(), OAuthError> {
        self.inner
            .limiter
            .check(bucket, key)
            .await
            .map_err(|limited| {
                warn!(bucket = bucket.prefix(), "rate limited");
                OAuthError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, OAuthError>>,
    ) -> Result<T, OAuthError> {
        let span = info_span!(
            "oauth",
            otel.name = %format!("oauth {operation}"),
            oauth.operation = operation,
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
