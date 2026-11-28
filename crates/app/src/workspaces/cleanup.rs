//! The hourly cleanup of invitations and invite links past their retention, of webhooks'
//! secrets rotated out once they stop holding, of pages in the trash past theirs, and of
//! the versions of pages the history no longer keeps, and of attachments no page or version
//! shows past their grace, with the blobs of files deleted.

use std::time::Duration;

use neon_domain::workspaces::RETENTION;
use neon_store::{incoming_webhooks, webhooks, workspace_invites};
use tracing::info;

use super::{WorkspaceError, WorkspaceService};
use crate::attachments::AttachmentService;
use crate::history::HistoryService;
use crate::pages::PageService;
use crate::periodic::Periodic;

/// What one run removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub invitations: u64,
    pub invite_links: u64,
    /// Webhooks' secrets, outgoing and incoming, rotated out and no longer holding.
    pub rotated_secrets: u64,
}

impl WorkspaceService {
    /// Removes, as of now, the invitations answered, revoked or expired more than 30 days
    /// ago, the invite links expired or revoked more than 30 days ago, and the webhooks'
    /// secrets rotated out whose overlap has ended.
    pub async fn cleanup(&self) -> Result<CleanupReport, WorkspaceError> {
        self.observe("cleanup", async {
            let now = self.now();
            let before = now - RETENTION;
            let report = CleanupReport {
                invitations: workspace_invites::delete_ended(self.pool(), before).await?,
                invite_links: workspace_invites::delete_ended_links(self.pool(), before).await?,
                rotated_secrets: webhooks::forget_rotated(self.pool(), now).await?
                    + incoming_webhooks::forget_rotated(self.pool(), now).await?,
            };
            if report != CleanupReport::default() {
                info!(
                    invitations = report.invitations,
                    invite_links = report.invite_links,
                    rotated_secrets = report.rotated_secrets,
                    "ended workspace invitations and rotated webhook secrets removed"
                );
            }
            Ok(report)
        })
        .await
    }
}

/// The running cleanup: once at start, then every interval until stopped. Each run removes
/// ended invitations and links, purges the pages' trash, thins out pages' versions, then
/// cleans up attachments, after the trash and the versions so the files of pages purged and
/// of versions thinned out go in the same run; one failing does not keep the others from
/// running, and the run's failure names every step that failed.
#[must_use = "dropping the handle leaves the task running with no way to stop it"]
pub struct Cleanup(Periodic);

impl Cleanup {
    pub fn spawn(
        workspaces: WorkspaceService,
        pages: PageService,
        history: HistoryService,
        attachments: AttachmentService,
        interval: Duration,
    ) -> Self {
        Self(Periodic::spawn("workspace cleanup", interval, move || {
            let workspaces = workspaces.clone();
            let pages = pages.clone();
            let history = history.clone();
            let attachments = attachments.clone();
            async move {
                let invitations = workspaces.cleanup().await.map_err(|e| e.to_string());
                let trash = pages.purge_trash().await.map_err(|e| e.to_string());
                let versions = history.thin().await.map_err(|e| e.to_string());
                let files = attachments.cleanup().await.map_err(|e| e.to_string());
                outcome([
                    ("invitations", invitations.err()),
                    ("trash", trash.err()),
                    ("versions", versions.err()),
                    ("files", files.err()),
                ])
            }
        }))
    }

    /// Stops, waiting for a run in progress, so the pool can close after it.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// A run's outcome from its steps, each named with its error if it failed: every failure, in
/// the order the steps ran, or none.
fn outcome(steps: impl IntoIterator<Item = (&'static str, Option<String>)>) -> Result<(), String> {
    let failures: Vec<String> = steps
        .into_iter()
        .filter_map(|(step, error)| error.map(|error| format!("{step}: {error}")))
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_fails_with_every_step_that_failed() {
        assert_eq!(outcome([("invitations", None), ("trash", None)]), Ok(()));
        assert_eq!(
            outcome([
                ("invitations", None),
                ("trash", Some("database: gone".to_owned())),
                ("versions", None),
                ("files", Some("the blob store is down".to_owned())),
            ]),
            Err("trash: database: gone; files: the blob store is down".to_owned()),
        );
    }
}
