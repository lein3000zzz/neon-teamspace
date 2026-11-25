//! Subscriptions to resources: each a task that follows, through the [`Broadcaster`], what
//! changes the resource, and tells its session `notifications/resources/updated`, at most
//! once every `mcp.notification_interval`. Nothing is polled.
//!
//! | Resource | Follows |
//! |---|---|
//! | a page | its document's changes, in the page's room; for a row, its changes in its database's room too |
//! | a database | its schema and views changing |
//! | a view's rows | its database's rows, schema and that view changing |
//! | the caller's tasks | the rows of the workspace's databases that name them, in the workspace's room |
//!
//! The first change is told at once; those that come within the interval after are told
//! together once it ends. A subscription ends when its session does, when it is dropped,
//! and when the caller can no longer see its resource, which the session's watcher checks.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::select_all;
use neon_app::settings::mcp as keys;
use neon_app::sync::{Broadcaster, ClientId, Next, Outbox, OutboxReceiver, Room, changes_document};
use neon_domain::auth::Principal;
use neon_domain::pages::PageKind;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::{Peer, RoleServer};
use serde_json::Value as Json;
use tokio::task::AbortHandle;
use tokio::time::{Instant, sleep_until};
use tracing::{debug, warn};
use uuid::Uuid;

use super::Services;
use super::resources::Resource;

/// How many messages, and bytes of them, may wait for a subscription to read them. One that
/// falls behind joins its room again, and tells of a change.
const QUEUE: usize = 256;
const QUEUE_BYTES: usize = 32 * 1024 * 1024;

/// A resource subscribed to. Dropped, its task stops and leaves its rooms.
pub(crate) struct Watch {
    pub(crate) resource: Resource,
    task: AbortHandle,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts following a resource the caller sees, for its session's `peer`: its rooms are
/// joined before this returns, so no change made after is missed.
pub(crate) async fn spawn(
    services: Services,
    principal: Principal,
    resource: Resource,
    peer: Peer<RoleServer>,
) -> Watch {
    let joined = rooms(&services, &principal, resource).await;
    let task = tokio::spawn(follow(services, principal, resource, peer, joined));
    Watch {
        resource,
        task: task.abort_handle(),
    }
}

/// Which messages of a room change the resource.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Interest {
    /// A page's document changing.
    Document,
    /// A row changing, its values, title or place, trashed or restored.
    Row(Uuid),
    /// A database's schema or views changing.
    Schema,
    /// A view's rows: any row, the schema, or that view changing.
    View(Uuid),
    /// The rows that name `user`, or named them when last looked at.
    Tasks { user: String, rows: HashSet<Uuid> },
}

impl Interest {
    fn wants(&self, message: &[u8]) -> bool {
        if *self == Self::Document {
            return changes_document(message);
        }
        let Ok(event) = serde_json::from_slice::<Json>(message) else {
            return false;
        };
        let kind = event["type"].as_str().unwrap_or_default();
        let row = event["row_id"]
            .as_str()
            .or_else(|| event["row"]["id"].as_str())
            .and_then(|id| Uuid::try_parse(id).ok());
        match self {
            Self::Document => false,
            Self::Row(id) => kind.starts_with("row_") && row == Some(*id),
            Self::Schema => matches!(kind, "schema_changed" | "view_changed"),
            Self::View(view) => match kind {
                "view_changed" => event["view_id"].as_str() == Some(view.to_string().as_str()),
                other => other.starts_with("row_") || other == "schema_changed",
            },
            Self::Tasks { user, rows } => {
                let names = |values: &Json| {
                    values.as_object().is_some_and(|values| {
                        values.values().any(|value| {
                            value
                                .as_array()
                                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(user)))
                        })
                    })
                };
                kind == "schema_changed"
                    || row.is_some_and(|row| rows.contains(&row))
                    || names(&event["values"])
                    || names(&event["row"]["values"])
            }
        }
    }
}

/// A room joined, left when dropped.
struct Joined {
    broadcaster: Arc<dyn Broadcaster>,
    room: Room,
    client: ClientId,
    receiver: OutboxReceiver,
    interest: Interest,
}

impl Joined {
    fn new(broadcaster: &Arc<dyn Broadcaster>, room: Room, interest: Interest) -> Self {
        let client = ClientId::next();
        let (outbox, receiver) = Outbox::new(QUEUE, QUEUE_BYTES);
        broadcaster.join(room, client, outbox);
        Self {
            broadcaster: Arc::clone(broadcaster),
            room,
            client,
            receiver,
            interest,
        }
    }

    /// The room joined again with an empty queue, after falling behind.
    fn rejoin(&mut self) {
        self.broadcaster.leave(self.room, self.client);
        self.client = ClientId::next();
        let (outbox, receiver) = Outbox::new(QUEUE, QUEUE_BYTES);
        self.broadcaster.join(self.room, self.client, outbox);
        self.receiver = receiver;
    }
}

impl Drop for Joined {
    fn drop(&mut self) {
        self.broadcaster.leave(self.room, self.client);
    }
}

/// The rooms that tell of the resource's changes, with what to look for in each.
async fn rooms(s: &Services, principal: &Principal, resource: Resource) -> Vec<Joined> {
    let b = &s.broadcaster;
    let database = |id| Room::Database {
        id,
        members_hidden: true,
    };
    match resource {
        Resource::Page(id) => {
            let mut joined = vec![Joined::new(b, Room::Page(id), Interest::Document)];
            if let Ok(view) = s.pages.page(principal, id).await
                && view.page.kind == PageKind::Row
                && let Some(database_id) = view.page.parent_id
            {
                joined.push(Joined::new(b, database(database_id), Interest::Row(id)));
            }
            joined
        }
        Resource::Database(id) => vec![Joined::new(b, database(id), Interest::Schema)],
        Resource::View { database: id, view } => {
            vec![Joined::new(b, database(id), Interest::View(view))]
        }
        Resource::Tasks => match principal.workspace() {
            Some(workspace) => {
                let interest = Interest::Tasks {
                    user: principal.user.id.to_string(),
                    rows: tasks(s, principal, workspace).await,
                };
                vec![Joined::new(b, Room::Workspace(workspace), interest)]
            }
            None => Vec::new(),
        },
    }
}

/// The rows assigned to the caller now.
async fn tasks(s: &Services, principal: &Principal, workspace: Uuid) -> HashSet<Uuid> {
    match s.databases.tasks(principal, workspace).await {
        Ok(tasks) => tasks.into_iter().map(|task| task.row.id).collect(),
        Err(error) => {
            warn!(%error, "cannot read the tasks a subscription follows");
            HashSet::new()
        }
    }
}

/// What happened while a subscription waited.
enum Event {
    /// A message from the room at this index, or its closing.
    Heard(usize, Next),
    /// The time to tell of the changes heard.
    Due,
}

async fn follow(
    s: Services,
    principal: Principal,
    resource: Resource,
    peer: Peer<RoleServer>,
    mut joined: Vec<Joined>,
) {
    let uri = resource.uri();
    if joined.is_empty() {
        return;
    }
    let mut told: Option<Instant> = None;
    let mut due: Option<Instant> = None;
    loop {
        let event = {
            let heard =
                select_all(joined.iter_mut().enumerate().map(|(index, room)| {
                    Box::pin(async move { (index, room.receiver.next().await) })
                }));
            tokio::select! {
                ((index, next), _, _) = heard => Event::Heard(index, next),
                () = sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => Event::Due,
            }
        };
        match event {
            Event::Heard(index, Next::Send(message)) => {
                if joined[index].interest.wants(&message) {
                    schedule(&mut due, told, &s);
                }
            }
            Event::Heard(index, Next::Close(reason)) => {
                debug!(
                    reason = reason.name(),
                    "a subscription fell behind; it joins again"
                );
                joined[index].rejoin();
                schedule(&mut due, told, &s);
            }
            Event::Due => {
                due = None;
                told = Some(Instant::now());
                let told = peer
                    .notify_resource_updated(ResourceUpdatedNotificationParam::new(uri.clone()))
                    .await;
                if told.is_err() {
                    // The session is gone.
                    return;
                }
                if let Resource::Tasks = resource
                    && let Some(workspace) = principal.workspace()
                {
                    let rows = tasks(&s, &principal, workspace).await;
                    for room in &mut joined {
                        if let Interest::Tasks { rows: known, .. } = &mut room.interest {
                            known.clone_from(&rows);
                        }
                    }
                }
            }
        }
    }
}

/// Sets when to tell of a change: at once, unless the last was told within the interval,
/// then once it ends. A change while one is due is told with it.
fn schedule(due: &mut Option<Instant>, told: Option<Instant>, s: &Services) {
    if due.is_some() {
        return;
    }
    let interval: Duration = *s.settings.current().get(keys::NOTIFICATION_INTERVAL);
    let now = Instant::now();
    *due = Some(told.map_or(now, |told| (told + interval).max(now)));
}

#[cfg(test)]
mod tests {
    use axum::body::Bytes;
    use serde_json::json;

    use super::*;

    fn message(value: Json) -> Bytes {
        Bytes::from(value.to_string())
    }

    #[test]
    fn each_resource_wants_the_changes_that_change_it() {
        let (row, other, view) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let updated = |id: Uuid| {
            message(json!({"type": "row_updated", "row_id": id.to_string(), "values": {}}))
        };
        let created = |values: Json| {
            message(
                json!({"type": "row_created", "row": {"id": other.to_string(), "values": values}}),
            )
        };
        let schema = message(json!({"type": "schema_changed"}));
        let view_changed =
            |id: Uuid| message(json!({"type": "view_changed", "view_id": id.to_string()}));

        assert!(Interest::Row(row).wants(&updated(row)));
        assert!(!Interest::Row(row).wants(&updated(other)));
        assert!(!Interest::Row(row).wants(&schema));
        assert!(Interest::Schema.wants(&schema));
        assert!(Interest::Schema.wants(&view_changed(other)));
        assert!(!Interest::Schema.wants(&updated(row)));
        assert!(Interest::View(view).wants(&updated(other)));
        assert!(Interest::View(view).wants(&view_changed(view)));
        assert!(!Interest::View(view).wants(&view_changed(other)));

        let user = Uuid::from_u128(9).to_string();
        let tasks = Interest::Tasks {
            user: user.clone(),
            rows: HashSet::from([row]),
        };
        assert!(tasks.wants(&updated(row)));
        assert!(!tasks.wants(&updated(other)));
        assert!(tasks.wants(&created(json!({"p": [user.clone()]}))));
        assert!(!tasks.wants(&created(json!({"p": ["someone else"]}))));
        assert!(tasks.wants(&message(json!({
            "type": "row_updated",
            "row_id": other.to_string(),
            "values": {"p": [user]}
        }))));
        assert!(!tasks.wants(b"not json"));
        assert!(!Interest::Document.wants(b"{\"type\": \"row_updated\"}"));
    }
}
