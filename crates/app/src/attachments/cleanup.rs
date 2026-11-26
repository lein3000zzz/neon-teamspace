//! The hourly cleanup of attachments: files no page shows any more, past their grace, and
//! the blobs of files deleted.
//!
//! A page's last snapshot names the files its document shows, and a cover the file it
//! shows. A document with updates logged since its snapshot may show more, and so may one
//! open now, so the cleanup reads each such document of the workspaces it looks at, as it
//! is now. A file shown
//! nowhere is marked, then deleted once it has stayed so for `attachments.orphan_grace`; a
//! file shown again is unmarked, and starts over the next time it is shown nowhere. When a
//! document cannot be read, its workspace's files wait for the next run.

use std::collections::{BTreeSet, HashSet};

use neon_store::files::{self, Unreferenced};
use tracing::{info, warn};
use uuid::Uuid;

use super::{AttachmentError, AttachmentService, shown_files};
use crate::settings::attachments as keys;

/// How many files, and how many blobs, one run looks at, at most. The next run takes the
/// rest.
const BATCH: i64 = 1000;

/// What one run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// Files found shown nowhere, from now.
    pub marked: u64,
    /// Files shown again.
    pub unmarked: u64,
    /// Files deleted, shown nowhere for their grace.
    pub deleted: u64,
    /// Blobs deleted from the store.
    pub blobs: u64,
}

impl AttachmentService {
    /// Marks, unmarks and deletes files as the module docs describe, then deletes the blobs
    /// of files deleted. The hourly cleanup runs it.
    pub async fn cleanup(&self) -> Result<CleanupReport, AttachmentError> {
        self.observe("cleanup", async {
            let now = self.now();
            let grace = *self.settings().get(keys::ORPHAN_GRACE);
            let before = now - grace;
            let mut candidates = files::unreferenced(self.pool(), None, BATCH).await?;
            candidates.extend(files::unreferenced(self.pool(), Some(before), BATCH).await?);
            let (shown, unsure) = self.shown_since_snapshots(&candidates).await?;
            let shown_ids: Vec<Uuid> = shown.iter().copied().collect();
            let mut report = CleanupReport {
                unmarked: files::unmark_referenced(self.pool(), &shown_ids).await?,
                ..CleanupReport::default()
            };
            let orphans: Vec<&Unreferenced> = candidates
                .iter()
                .filter(|file| !shown.contains(&file.id) && !unsure.contains(&file.workspace_id))
                .collect();
            let fresh: Vec<Uuid> = orphans
                .iter()
                .filter(|file| file.since.is_none())
                .map(|file| file.id)
                .collect();
            report.marked = files::mark_unreferenced(self.pool(), &fresh, now).await?;
            let due: Vec<Uuid> = orphans
                .iter()
                .filter(|file| file.since.is_some_and(|since| since < before))
                .map(|file| file.id)
                .collect();
            report.deleted = files::delete_unreferenced(self.pool(), &due, before).await?;
            report.blobs = self.sweep(now).await?;
            if report != CleanupReport::default() {
                info!(
                    marked = report.marked,
                    unmarked = report.unmarked,
                    deleted = report.deleted,
                    blobs = report.blobs,
                    "attachments cleaned up"
                );
            }
            Ok(report)
        })
        .await
    }

    /// The files the documents of the candidates' workspaces show that their last
    /// snapshots may not, and the workspaces one of whose documents did not read.
    async fn shown_since_snapshots(
        &self,
        candidates: &[Unreferenced],
    ) -> Result<(HashSet<Uuid>, HashSet<Uuid>), AttachmentError> {
        let workspaces: Vec<Uuid> = candidates
            .iter()
            .map(|file| file.workspace_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let (mut shown, mut unsure) = (HashSet::new(), HashSet::new());
        if workspaces.is_empty() {
            return Ok((shown, unsure));
        }
        let open = self.inner.sync.open_pages();
        for (page_id, workspace_id) in files::pages_to_read(self.pool(), &workspaces, &open).await?
        {
            match self.inner.sync.content(page_id).await {
                Ok(document) => shown.extend(shown_files(&document)),
                Err(error) => {
                    warn!(%error, %page_id, "a document did not read; its workspace's files wait");
                    unsure.insert(workspace_id);
                }
            }
        }
        Ok((shown, unsure))
    }

    /// Deletes the blobs due to be, a batch of them, and says how many went. One the store
    /// does not delete stays for the next run.
    async fn sweep(&self, now: time::OffsetDateTime) -> Result<u64, AttachmentError> {
        let keys = files::due_blobs(self.pool(), now, BATCH).await?;
        if keys.is_empty() {
            return Ok(0);
        }
        let store = self.blobs()?;
        let mut deleted = 0;
        for key in keys {
            match store.delete(&key).await {
                Ok(()) => {
                    files::blob_deleted(self.pool(), &key).await?;
                    deleted += 1;
                }
                Err(error) => warn!(%error, "a blob stays to delete at the next cleanup"),
            }
        }
        Ok(deleted)
    }
}
