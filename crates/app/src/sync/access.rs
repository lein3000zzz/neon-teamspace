//! Hearing of access changes, and looking again at the connections they may concern.
//!
//! The page, workspace and auth services tell an [`AccessListener`] whose access may have
//! changed, once each change commits. It hands the change to the sync service's watcher
//! and returns at once. The watcher finds the open connections the change may concern: of
//! the person it names and of their agents and integrations, whose access theirs bounds,
//! or everyone's, in its workspace, and for a change to a page, on
//! that page, a page under it, or a page that is gone. For each, at most
//! [`RECHECKS_AT_ONCE`] at a time, it checks again that the session or token holds and
//! asks the rules what its person may do with the page now, and the connection's document
//! closes it, makes it read-only, or lets it edit again. Every
//! [`Limits::revalidate_interval`](super::Limits::revalidate_interval) it looks so at every
//! connection.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures_util::StreamExt as _;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uuid::Uuid;

use neon_domain::auth::Principal;

use super::broadcast::ClientId;
use super::{Inner, Recheck};
use crate::pages::{AccessChange, AccessChanges};
use crate::standing::owner_of;

/// How many connections the watcher looks at again at once.
pub(crate) const RECHECKS_AT_ONCE: usize = 8;

/// Tells the sync service of access changes. Give it to the page, workspace and auth
/// services.
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
    /// Whether the change may concern what `principal` may do in `workspace_id`: a change
    /// for them, or for the owner of an agent or integration, whose access bounds its own.
    /// Which pages a change to a teamspace reaches is the rules' to say again, so every
    /// page of the workspace counts; a change to a page is narrowed further by
    /// [`subtree`](Self::subtree).
    pub fn concerns(&self, workspace_id: Uuid, principal: &Principal) -> bool {
        let (workspace, user) = match *self {
            Self::Agents => return owner_of(principal).is_some(),
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
            } => (Some(workspace_id), user_id),
            Self::Account { user_id } => (None, Some(user_id)),
        };
        workspace.is_none_or(|workspace| workspace == workspace_id)
            && user
                .is_none_or(|user| user == principal.user.id || owner_of(principal) == Some(user))
    }

    /// The page whose subtree the change is to, if it is to one.
    pub(crate) fn subtree(&self) -> Option<Uuid> {
        match *self {
            Self::Pages { page_id, .. } => Some(page_id),
            Self::Teamspace { .. }
            | Self::Workspace { .. }
            | Self::Account { .. }
            | Self::Agents => None,
        }
    }
}

/// Looks again at the connections each change concerns, and at every connection every
/// `every`, until the service stops or is gone. Changes that came together are looked at
/// together, each connection once. It holds the service only while it looks.
pub(crate) async fn watch(
    inner: Weak<Inner>,
    stop: CancellationToken,
    every: Duration,
    mut changes: mpsc::UnboundedReceiver<AccessChange>,
) {
    let mut revalidate = interval_at(Instant::now() + every, every);
    revalidate.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let batch = tokio::select! {
            () = stop.cancelled() => return,
            change = changes.recv() => match change {
                Some(change) => {
                    let mut batch = vec![change];
                    while let Ok(change) = changes.try_recv() {
                        batch.push(change);
                    }
                    Some(batch)
                }
                None => return,
            },
            _ = revalidate.tick() => None,
        };
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let connections = match &batch {
            Some(batch) => inner.concerned(batch).await,
            None => inner.connections(|_, _| true),
        };
        debug!(
            changes = batch.as_ref().map_or(0, Vec::len),
            connections = connections.len(),
            "looking again at connections' access"
        );
        recheck_all(&inner, connections).await;
    }
}

/// Looks again at each connection, a few at a time.
async fn recheck_all(inner: &Inner, connections: Vec<Recheck>) {
    futures_util::stream::iter(connections)
        .for_each_concurrent(RECHECKS_AT_ONCE, |connection| async move {
            if let Err(error) = inner.recheck(connection).await {
                warn!(%error, "cannot look again at a connection's access; it keeps it for now");
            }
        })
        .await;
}

impl Inner {
    /// The open connections any of `changes` may concern, each once.
    async fn concerned(&self, changes: &[AccessChange]) -> Vec<Recheck> {
        let mut found: HashMap<ClientId, Recheck> = HashMap::new();
        for change in changes {
            let candidates =
                self.connections(|workspace, principal| change.concerns(workspace, principal));
            let candidates = match change.subtree() {
                Some(root) if !candidates.is_empty() => self.within(root, candidates).await,
                _ => candidates,
            };
            for connection in candidates {
                found.entry(connection.client).or_insert(connection);
            }
        }
        found.into_values().collect()
    }

    /// The connections on `root`, a page under it, or a page that is gone. When the pages
    /// cannot be read, all of them, to be safe.
    async fn within(&self, root: Uuid, connections: Vec<Recheck>) -> Vec<Recheck> {
        let pages: Vec<Uuid> = connections
            .iter()
            .map(|connection| connection.page_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        match neon_store::pages::within(&self.pool, root, &pages).await {
            Ok(under) => {
                let under: HashSet<Uuid> = under.into_iter().collect();
                connections
                    .into_iter()
                    .filter(|connection| under.contains(&connection.page_id))
                    .collect()
            }
            Err(error) => {
                warn!(%error, "cannot tell which open pages a change reaches; looking at all");
                connections
            }
        }
    }
}
