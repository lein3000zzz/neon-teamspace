//! The port that hears when someone's access to pages may have changed, so that what
//! holds pages open, such as live editors, can look again at what each person may do. The
//! page and workspace services tell it of changes to the rules' facts, and the auth service
//! of sessions and tokens that end.

use std::sync::Arc;

use uuid::Uuid;

/// Whose access to which pages may have changed. Each names where to look again: whoever
/// has those pages open checks what they may do now, and closes them or stops editing when
/// the rules say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessChange {
    /// A page and every page under it. For everyone when it was moved, trashed, restored
    /// or deleted; for `user_id` when a share on it was added, changed or removed.
    Pages {
        workspace_id: Uuid,
        page_id: Uuid,
        user_id: Option<Uuid>,
    },
    /// A teamspace's pages. For `user_id` when they joined or left it, or were added,
    /// removed or given another role there; for everyone when its access changed or it
    /// was deleted.
    Teamspace {
        workspace_id: Uuid,
        teamspace_id: Uuid,
        user_id: Option<Uuid>,
    },
    /// Every page of a workspace. For `user_id` when their role changed, or they left or
    /// were removed; for everyone when the workspace was deleted.
    Workspace {
        workspace_id: Uuid,
        user_id: Option<Uuid>,
    },
    /// Every page, for `user_id`, whose way in changed: a session or API token of theirs
    /// ended, or their account was disabled or changed role. Whoever holds pages open for
    /// them checks that the session or token they opened them with still holds.
    Account { user_id: Uuid },
}

/// Hears of access changes, once each has committed. It runs on the use case's task, so it
/// must not block: an implementation hands the change on, to a channel say, and returns.
pub trait AccessChanges: Send + Sync {
    fn access_changed(&self, change: AccessChange);
}

/// What the services tell while nothing listens: nobody.
pub struct NoListener;

impl AccessChanges for NoListener {
    fn access_changed(&self, _: AccessChange) {}
}

/// Tells each of several listeners of every change, in turn: the live documents and the
/// database feeds both hear them.
pub struct Fanout(Vec<Arc<dyn AccessChanges>>);

impl Fanout {
    pub fn new(listeners: Vec<Arc<dyn AccessChanges>>) -> Self {
        Self(listeners)
    }
}

impl AccessChanges for Fanout {
    fn access_changed(&self, change: AccessChange) {
        for listener in &self.0 {
            listener.access_changed(change);
        }
    }
}
