//! The hourly cleanup of invitations and invite links past their retention.

use std::time::Duration;

use neon_domain::workspaces::RETENTION;
use neon_store::workspace_invites;
use tracing::info;

use super::{WorkspaceError, WorkspaceService};
use crate::periodic::Periodic;

/// What one run removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub invitations: u64,
    pub invite_links: u64,
}

impl WorkspaceService {
    /// Removes, as of now, the invitations answered, revoked or expired more than 30 days
    /// ago, and the invite links expired or revoked more than 30 days ago.
    pub async fn cleanup(&self) -> Result<CleanupReport, WorkspaceError> {
        self.observe("cleanup", async {
            let before = self.now() - RETENTION;
            let report = CleanupReport {
                invitations: workspace_invites::delete_ended(self.pool(), before).await?,
                invite_links: workspace_invites::delete_ended_links(self.pool(), before).await?,
            };
            if report != CleanupReport::default() {
                info!(
                    invitations = report.invitations,
                    invite_links = report.invite_links,
                    "ended workspace invitations removed"
                );
            }
            Ok(report)
        })
        .await
    }
}

/// The running cleanup: once at start, then every interval until stopped.
pub struct Cleanup(Periodic);

impl Cleanup {
    pub fn spawn(workspaces: WorkspaceService, interval: Duration) -> Self {
        Self(Periodic::spawn("workspace cleanup", interval, move || {
            let workspaces = workspaces.clone();
            async move { workspaces.cleanup().await.map(drop) }
        }))
    }

    /// Stops, waiting for a run in progress, so the pool can close after it.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}
