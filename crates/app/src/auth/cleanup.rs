//! The hourly cleanup of what expired: sessions, mailed links, accounts that never
//! verified, invites and requests to join past their retention.
//!
//! It is a periodic task of its own rather than a job: it needs no retries, since the next
//! run catches up, and every server process may run it, as its deletes do not conflict.

use std::time::Duration;

use neon_domain::auth::{email_token, invite, join_request};
use neon_store::{email_tokens, invites, join_requests, sessions, users};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::{AuthError, AuthService};

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
pub struct Cleanup {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Cleanup {
    pub fn spawn(auth: AuthService, interval: Duration) -> Self {
        let stop = CancellationToken::new();
        let task = tokio::spawn(run(auth, interval, stop.clone()));
        Self { stop, task }
    }

    /// Stops, waiting for a run in progress, so the pool can close after it.
    pub async fn stop(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            error!(%error, "the auth cleanup task failed");
        }
    }
}

async fn run(auth: AuthService, interval: Duration, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = ticks.tick() => {
                if let Err(error) = auth.cleanup().await {
                    warn!(%error, "the auth cleanup failed; the next run tries again");
                }
            }
        }
    }
}
