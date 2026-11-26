//! Making notifications, in the transaction of what they tell of, and telling the inboxes
//! open that they changed once it commits.

use std::collections::BTreeSet;
use std::sync::Arc;

use bytes::Bytes;
use neon_domain::agents::IdentityMode;
use neon_domain::auth::{Credential, Principal, Scope};
use neon_domain::notifications::DEFAULT_MAIL;
use neon_domain::workspaces::Action;
use neon_store::jobs::{self, NewJob};
use neon_store::notifications::{self as repo, Notification};
use neon_store::users;
use serde_json::json;
use sqlx::PgConnection;
use tracing::debug;
use uuid::Uuid;

use super::MAIL_KIND;
use crate::pages::{PageError, PageService};
use crate::settings::{SettingsService, mcp, notifications as keys};
use crate::sync::{Broadcaster, Room};

/// The message the inbox's room hears when one of its notifications was made or changed:
/// read it again.
pub(crate) fn changed_message() -> Bytes {
    Bytes::from_static(br#"{"type":"inbox"}"#)
}

/// Makes notifications on a transaction and, once it commits, tells the inboxes it changed.
pub(crate) struct Notifier {
    settings: Arc<SettingsService>,
    broadcaster: Arc<dyn Broadcaster>,
    told: BTreeSet<Uuid>,
}

impl Notifier {
    pub(crate) fn new(settings: &Arc<SettingsService>, broadcaster: &Arc<dyn Broadcaster>) -> Self {
        Self {
            settings: Arc::clone(settings),
            broadcaster: Arc::clone(broadcaster),
            told: BTreeSet::new(),
        }
    }

    /// One for the page services' use cases, which share their settings and broadcaster.
    pub(crate) fn of(pages: &PageService) -> Self {
        Self::new(pages.settings(), pages.broadcaster())
    }

    /// Saves a notification on the caller's transaction, unless its person has one of the
    /// same comment, and queues the mail of it for those who want a mail for each, while
    /// the instance sends them. Says whether it was made.
    pub(crate) async fn record(
        &mut self,
        conn: &mut PgConnection,
        notification: &Notification,
    ) -> sqlx::Result<bool> {
        if !repo::insert(&mut *conn, notification).await? {
            return Ok(false);
        }
        let settings = self.settings.current();
        if *settings.get(keys::MAIL)
            && repo::wants_each_mail(&mut *conn, notification.user_id, DEFAULT_MAIL).await?
        {
            let due = notification.created_at + *settings.get(keys::MAIL_DELAY);
            let job =
                NewJob::new(MAIL_KIND, json!({ "notification_id": notification.id })).run_at(due);
            jobs::enqueue(&mut *conn, &job).await?;
        }
        debug!(
            notification_id = %notification.id,
            user_id = %notification.user_id,
            kind = notification.kind.as_str(),
            "notification made"
        );
        self.told.insert(notification.user_id);
        Ok(true)
    }

    /// Notes an inbox that changed without a new notification, to tell it too.
    pub(crate) fn touched(&mut self, user_id: Uuid) {
        self.told.insert(user_id);
    }

    /// Tells each inbox that changed, once the transaction committed.
    pub(crate) fn tell(self) {
        for user in &self.told {
            self.broadcaster
                .broadcast(Room::Inbox(*user), changed_message(), None);
        }
    }
}

/// Someone as they stand in a workspace, reading: what notifications about it are asked
/// as, their own access with an agent's owner's. `None` for an account that is gone or
/// disabled, and for an agent or integration that does not work now.
pub(crate) async fn reader(
    conn: &mut PgConnection,
    settings: &SettingsService,
    user_id: Uuid,
    workspace_id: Uuid,
) -> sqlx::Result<Option<Principal>> {
    let Some((user, standing)) = users::identity(&mut *conn, user_id).await? else {
        return Ok(None);
    };
    let agents_on = IdentityMode::Agent.among(settings.current().get(mcp::IDENTITIES));
    let works = if user.kind.is_human() {
        !user.is_disabled()
    } else {
        user.workspace_id == Some(workspace_id) && standing.status(user.kind, agents_on).works()
    };
    Ok(works.then(|| Principal {
        user,
        credential: Credential::Token {
            id: Uuid::nil(),
            name: "inbox".to_owned(),
            scopes: vec![Scope::Read],
            workspace_id: Some(workspace_id),
        },
    }))
}

/// Whether someone may see a page now, as the rules say for them in its workspace.
pub(crate) async fn sees(
    pages: &PageService,
    conn: &mut PgConnection,
    user_id: Uuid,
    workspace_id: Uuid,
    page_id: Uuid,
) -> Result<bool, PageError> {
    let Some(reader) = reader(&mut *conn, pages.settings(), user_id, workspace_id).await? else {
        return Ok(false);
    };
    match pages.context(&mut *conn, &reader, page_id).await {
        Ok(context) => {
            Ok(context.page().workspace_id == workspace_id
                && context.check(Action::ViewPage).is_ok())
        }
        Err(PageError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}
