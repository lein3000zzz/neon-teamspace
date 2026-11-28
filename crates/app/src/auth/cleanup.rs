//! The hourly cleanup of what expired: sessions, mailed links, accounts that never
//! verified, invites and requests to join past their retention.
//!
//! It is a [`Periodic`] task rather than a job.

use std::time::Duration;

use neon_domain::auth::{email_token, invite, join_request};
use neon_store::{email_tokens, invites, join_requests, sessions, users};
use tracing::info;

use super::{AuthError, AuthService};
use crate::periodic::Periodic;

/// How often the cleanup runs.
pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

/// What one run removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub sessions: u64,
    pub email_tokens: u64,
    pub unverified_users: u64,
    pub invites: u64,
    pub join_requests: u64,
}

impl AuthService {
    /// Removes, as of now:
    ///
    /// - sessions past either expiry,
    /// - mailed links used or expired more than a day ago, once they no longer count
    ///   towards the links of the day,
    /// - accounts still unverified 7 days after they were made,
    /// - invites expired or revoked more than 30 days ago, with their sign-ups,
    /// - requests to join decided more than 30 days ago, which ends a denial's block, and
    ///   those never confirmed a day after their link stopped working.
    pub async fn cleanup(&self) -> Result<CleanupReport, AuthError> {
        self.observe("cleanup", async {
            let now = self.now();
            let pool = self.pool();
            let report = CleanupReport {
                sessions: sessions::delete_expired(pool, now).await?,
                email_tokens: email_tokens::delete_spent(pool, now - email_token::RETENTION)
                    .await?,
                unverified_users: users::prune_unverified(pool, now - email_token::UNVERIFIED_TTL)
                    .await?,
                invites: invites::delete_ended(pool, now - invite::RETENTION).await?,
                join_requests: join_requests::delete_decided(pool, now - join_request::RETENTION)
                    .await?
                    + join_requests::prune_unconfirmed(pool, now - join_request::LINK_TTL).await?,
            };
            if report != CleanupReport::default() {
                info!(
                    sessions = report.sessions,
                    email_tokens = report.email_tokens,
                    unverified_users = report.unverified_users,
                    invites = report.invites,
                    join_requests = report.join_requests,
                    "expired auth records removed"
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
    pub fn spawn(auth: AuthService, interval: Duration) -> Self {
        Self(Periodic::spawn("auth cleanup", interval, move || {
            let auth = auth.clone();
            async move { auth.cleanup().await.map(drop) }
        }))
    }

    /// Stops, waiting for a run in progress, so the pool can close after it.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}
