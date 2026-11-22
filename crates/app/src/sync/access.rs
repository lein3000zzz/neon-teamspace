//! Hearing of access changes, and looking again at the connections they may concern.
//!
//! The page and workspace services tell an [`AccessListener`] whose access may have
//! changed, once each change commits. It hands the change to the sync service's watcher
//! and returns at once. The watcher asks the rules again, for every open connection the
//! change may concern, what its person may do with the page now, and the connection's
//! document closes it, makes it read-only, or lets it edit again.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::Inner;
use crate::pages::{AccessChange, AccessChanges};

/// Tells the sync service of access changes. Give it to the page and workspace services.
pub struct AccessListener {
    changes: mpsc::UnboundedSender<AccessChange>,
    epoch: Arc<AtomicU64>,
}

/// What an [`AccessListener`] hears, for the sync service to watch.
pub struct AccessFeed {
    pub(crate) changes: mpsc::UnboundedReceiver<AccessChange>,
    /// How many changes were heard. A connection admitted before a change and joined after
    /// it looks again at its access, as the watcher may have missed it.
    pub(crate) epoch: Arc<AtomicU64>,
}

/// A listener for the services that change access, and the feed for the sync service.
pub fn access_feed() -> (Arc<AccessListener>, AccessFeed) {
    let (changes, received) = mpsc::unbounded_channel();
    let epoch = Arc::new(AtomicU64::new(0));
    (
        Arc::new(AccessListener {
            changes,
            epoch: Arc::clone(&epoch),
        }),
        AccessFeed {
            changes: received,
            epoch,
        },
    )
}

impl AccessChanges for AccessListener {
    fn access_changed(&self, change: AccessChange) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        // Once the sync service stops, nobody needs to hear.
        let _ = self.changes.send(change);
    }
}

impl AccessChange {
    /// Whether the change may concern `user_id`'s access in `workspace_id`. Which pages a
    /// change to a teamspace or a page's subtree reaches is the rules' to say again, so
    /// every page of the workspace counts.
    pub(crate) fn concerns(&self, workspace_id: uuid::Uuid, user_id: uuid::Uuid) -> bool {
        let (workspace, user) = match *self {
            Self::Pages {
                workspace_id,
                user_id,
                ..
            }
            | Self::Teamspace {
                workspace_id,
                user_id,
                ..
            }
            | Self::Workspace {
                workspace_id,
                user_id,
            } => (workspace_id, user_id),
        };
        workspace == workspace_id && user.is_none_or(|user| user == user_id)
    }
}

/// Looks again at the connections each change concerns, until the service stops. Changes
/// that came together are looked at together, each connection once.
pub(crate) async fn watch(inner: Arc<Inner>, mut changes: mpsc::UnboundedReceiver<AccessChange>) {
    loop {
        let first = tokio::select! {
            () = inner.stop.cancelled() => return,
            change = changes.recv() => match change {
                Some(change) => change,
                None => return,
            },
        };
        let mut batch = vec![first];
        while let Ok(change) = changes.try_recv() {
            batch.push(change);
        }
        let concerned = inner.concerned(&batch);
        debug!(
            changes = batch.len(),
            connections = concerned.len(),
            "access changed; looking again"
        );
        for connection in concerned {
            if let Err(error) = inner.recheck(connection).await {
                warn!(%error, "cannot look again at a connection's access; it keeps it for now");
            }
        }
    }
}
