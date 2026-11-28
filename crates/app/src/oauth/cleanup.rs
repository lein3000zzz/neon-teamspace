//! The hourly cleanup of OAuth's leftovers: expired codes and tokens, grants nothing can
//! use any more, and clients nobody connected.
//!
//! It is a [`Periodic`] task rather than a job.

use std::time::Duration;

use neon_store::oauth as repo;
use tracing::info;

use super::{OAuthError, OAuthService};
use crate::periodic::Periodic;
use crate::settings::oauth as keys;

/// How long after its last consent a grant without a code or token that works is kept, so
/// that the cleanup never races a person consenting again.
const SPENT_GRANT_GRACE: Duration = Duration::from_secs(3600);

/// What one run removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub codes: u64,
    pub tokens: u64,
    pub grants: u64,
    pub clients: u64,
}

impl OAuthService {
    /// Removes, as of now:
    ///
    /// - codes and tokens that expired, used refresh tokens among them, which reuse
    ///   detection no longer needs once they would not work anyway,
    /// - grants none of whose codes and tokens works any more, an hour after they were
    ///   last consented to: the client has to be connected again either way,
    /// - clients no grant uses that were neither registered nor used within
    ///   `oauth.unused_client_ttl`.
    pub async fn cleanup(&self) -> Result<CleanupReport, OAuthError> {
        self.observe("cleanup", async {
            let now = self.now();
            let pool = self.pool();
            let unused = *self.settings().get(keys::UNUSED_CLIENT_TTL);
            let report = CleanupReport {
                codes: repo::delete_expired_codes(pool, now).await?,
                tokens: repo::delete_expired_tokens(pool, now).await?,
                grants: repo::delete_spent_grants(pool, now, now - SPENT_GRANT_GRACE).await?,
                clients: repo::delete_unused_clients(pool, now - unused).await?,
            };
            if report != CleanupReport::default() {
                info!(
                    codes = report.codes,
                    tokens = report.tokens,
                    grants = report.grants,
                    clients = report.clients,
                    "expired OAuth records removed"
                );
            }
            Ok(report)
        })
        .await
    }
}

/// The running cleanup: once at start, then every interval until stopped.
#[must_use = "dropping the handle leaves the task running with no way to stop it"]
pub struct Cleanup(Periodic);

impl Cleanup {
    pub fn spawn(oauth: OAuthService, interval: Duration) -> Self {
        Self(Periodic::spawn("oauth cleanup", interval, move || {
            let oauth = oauth.clone();
            async move { oauth.cleanup().await.map(drop) }
        }))
    }

    /// Stops, waiting for a run in progress, so the pool can close after it.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}
