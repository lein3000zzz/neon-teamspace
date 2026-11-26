//! The events a change to a row's values makes, and those of mentions and replies, and
//! their deliveries, queued on the transaction of the change.
//!
//! A row's assignees are the identities in its person properties. Every writer of values,
//! the API, MCP and the web app alike, goes through the database service, which hands each
//! change here before it commits:
//!
//! - **`row.assigned`**, for each identity a change adds to a person property, a new row's
//!   included, once per property it is added to.
//! - **`row.status_changed`**, when a change sets the database's status property, its first
//!   in the schema, to another value, for each identity in one of the row's person
//!   properties once the change is made. A new row's status changes nothing.
//!
//! The comments and notifications of pages hand over two more, each in the transaction that
//! makes its notification:
//!
//! - **`page.mentioned`**, for each identity a comment, or a page's text once stored,
//!   mentions, who may see the page.
//! - **`comment.replied`**, for each identity that wrote an earlier comment of a thread
//!   someone replies in, who may see the page.
//!
//! Each event goes to every webhook of its identity that is on and asks for its type, as a
//! job of its own, but not for the identity's own change unless the webhook says so. A
//! change that rolls back queues nothing. Changes to the schema, such as deleting a person
//! property or one of the status's options, change no row's values in this sense and make
//! no event.

use std::collections::BTreeSet;

use neon_domain::databases::{Property, PropertyKind, Schema, Value};
use neon_domain::webhooks::EventType;
use neon_store::jobs::{self, NewJob};
use neon_store::webhooks as repo;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::debug;
use uuid::Uuid;

use super::{DELIVERY_KIND, MAX_ATTEMPTS};

/// A change to a row's values, as the database service hands it over before it commits.
pub(crate) struct RowChange<'a> {
    pub(crate) workspace_id: Uuid,
    pub(crate) database_id: Uuid,
    pub(crate) row_id: Uuid,
    /// Who made it.
    pub(crate) actor: Uuid,
    pub(crate) schema: &'a Schema,
    /// The row's cells before, `None` for a new row.
    pub(crate) before: Option<&'a Json>,
    pub(crate) after: &'a Json,
    pub(crate) at: OffsetDateTime,
}

/// An option of the status property, as an event names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Choice {
    pub(crate) id: Uuid,
    pub(crate) name: String,
}

/// An event as its deliveries carry it, in their jobs' payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Event {
    pub(crate) id: Uuid,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    /// Unix time in microseconds.
    pub(crate) occurred_at: i64,
    pub(crate) workspace_id: Uuid,
    pub(crate) database_id: Uuid,
    pub(crate) row_id: Uuid,
    pub(crate) property_id: Uuid,
    /// The identity it concerns.
    pub(crate) identity_id: Uuid,
    /// For a status change, the status before and after.
    #[serde(default)]
    pub(crate) from: Option<Choice>,
    #[serde(default)]
    pub(crate) to: Option<Choice>,
}

impl Event {
    pub(crate) fn occurred_at(&self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.occurred_at) * 1000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }
}

/// An event about a page rather than a row: someone mentioned on it, or a reply to a
/// comment on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PageEvent {
    pub(crate) id: Uuid,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    /// Unix time in microseconds.
    pub(crate) occurred_at: i64,
    pub(crate) workspace_id: Uuid,
    pub(crate) page_id: Uuid,
    /// The identity it concerns.
    pub(crate) identity_id: Uuid,
    /// Who mentioned or replied.
    pub(crate) actor_id: Option<Uuid>,
    /// The comment it is about, none for a mention in the page's text.
    pub(crate) thread_id: Option<Uuid>,
    pub(crate) comment_id: Option<Uuid>,
}

impl PageEvent {
    pub(crate) fn occurred_at(&self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.occurred_at) * 1000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }
}

/// What a delivery tells of: a row's change, or a page's mention or reply. A row's event
/// names its database, so the payloads read apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Told {
    Row(Event),
    Page(PageEvent),
}

impl Told {
    pub(crate) fn id(&self) -> Uuid {
        match self {
            Self::Row(event) => event.id,
            Self::Page(event) => event.id,
        }
    }

    pub(crate) fn kind(&self) -> &str {
        match self {
            Self::Row(event) => &event.kind,
            Self::Page(event) => &event.kind,
        }
    }
}

/// A delivery's job payload: which webhook, and what to tell it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Delivery {
    pub(crate) webhook_id: Uuid,
    pub(crate) event: Told,
}

/// An event a change makes, before it is given to webhooks.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Detected {
    kind: EventType,
    identity: Uuid,
    property: Uuid,
    from: Option<Choice>,
    to: Option<Choice>,
}

/// The people a person property names in a row's cells.
fn people(property: &Property, cells: Option<&Json>) -> Vec<Uuid> {
    let stored = cells
        .and_then(|cells| cells.get(property.id.to_string()))
        .and_then(|json| Value::stored(&property.ty, json));
    match stored {
        Some(Value::People(ids)) => ids,
        _ => Vec::new(),
    }
}

/// The option a status property holds in a row's cells.
fn status(property: &Property, cells: Option<&Json>) -> Option<Choice> {
    let stored = cells
        .and_then(|cells| cells.get(property.id.to_string()))
        .and_then(|json| Value::stored(&property.ty, json))?;
    let Value::Choice(id) = stored else {
        return None;
    };
    let name = property
        .ty
        .options()
        .iter()
        .find(|option| option.id == id)
        .map(|option| option.name.clone())
        .unwrap_or_default();
    Some(Choice { id, name })
}

/// The database's status property: its first in the schema.
pub(crate) fn status_property(schema: &Schema) -> Option<&Property> {
    schema
        .properties
        .iter()
        .find(|property| property.kind() == PropertyKind::Status)
}

/// The events a change makes, in schema order.
fn detect(schema: &Schema, before: Option<&Json>, after: &Json) -> Vec<Detected> {
    let mut events = Vec::new();
    let mut assignees = BTreeSet::new();
    for property in schema
        .properties
        .iter()
        .filter(|property| property.kind() == PropertyKind::Person)
    {
        let was = people(property, before);
        let now = people(property, Some(after));
        for identity in now.iter().filter(|id| !was.contains(id)) {
            events.push(Detected {
                kind: EventType::Assigned,
                identity: *identity,
                property: property.id,
                from: None,
                to: None,
            });
        }
        assignees.extend(now);
    }
    if let (Some(property), Some(_)) = (status_property(schema), before) {
        let (from, to) = (status(property, before), status(property, Some(after)));
        if from.as_ref().map(|c| c.id) != to.as_ref().map(|c| c.id) {
            for identity in assignees {
                events.push(Detected {
                    kind: EventType::StatusChanged,
                    identity,
                    property: property.id,
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
    }
    events
}

/// Queues the deliveries of the events a change makes, on its transaction, and says how
/// many.
pub(crate) async fn queue(conn: &mut PgConnection, change: RowChange<'_>) -> sqlx::Result<usize> {
    let detected = detect(change.schema, change.before, change.after);
    if detected.is_empty() {
        return Ok(0);
    }
    let micros = i64::try_from(change.at.unix_timestamp_nanos() / 1000).unwrap_or(i64::MAX);
    let mut queued = 0;
    for kind in EventType::ALL {
        let of_kind: Vec<&Detected> = detected
            .iter()
            .filter(|event| event.kind == *kind)
            .collect();
        if of_kind.is_empty() {
            continue;
        }
        let identities: Vec<Uuid> = of_kind.iter().map(|event| event.identity).collect();
        let subscribers =
            repo::subscribers(&mut *conn, change.workspace_id, &identities, kind.as_str()).await?;
        for detected in of_kind {
            let event = Event {
                id: Uuid::now_v7(),
                kind: kind.as_str().to_owned(),
                occurred_at: micros,
                workspace_id: change.workspace_id,
                database_id: change.database_id,
                row_id: change.row_id,
                property_id: detected.property,
                identity_id: detected.identity,
                from: detected.from.clone(),
                to: detected.to.clone(),
            };
            for subscriber in subscribers.iter().filter(|subscriber| {
                subscriber.user_id == detected.identity
                    && (subscriber.include_own || subscriber.user_id != change.actor)
            }) {
                let payload = serde_json::to_value(Delivery {
                    webhook_id: subscriber.webhook_id,
                    event: Told::Row(event.clone()),
                })
                .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
                let job = NewJob::new(DELIVERY_KIND, payload).max_attempts(MAX_ATTEMPTS);
                let id = jobs::enqueue(&mut *conn, &job).await?;
                debug!(
                    webhook_id = %subscriber.webhook_id,
                    delivery_id = %id,
                    event = kind.as_str(),
                    "webhook delivery queued"
                );
                queued += 1;
            }
        }
    }
    Ok(queued)
}

/// The identities a change adds to a row's person properties, each with the property,
/// a new row's everyone in them: whom it assigns.
pub(crate) fn assignments(
    schema: &Schema,
    before: Option<&Json>,
    after: &Json,
) -> Vec<(Uuid, Uuid)> {
    detect(schema, before, after)
        .into_iter()
        .filter(|event| event.kind == EventType::Assigned)
        .map(|event| (event.identity, event.property))
        .collect()
}

/// A page's event, before it is given to webhooks.
pub(crate) struct PageChange {
    pub(crate) kind: EventType,
    pub(crate) workspace_id: Uuid,
    pub(crate) page_id: Uuid,
    pub(crate) identity: Uuid,
    pub(crate) actor: Option<Uuid>,
    pub(crate) thread_id: Option<Uuid>,
    pub(crate) comment_id: Option<Uuid>,
    pub(crate) at: OffsetDateTime,
}

/// Queues the deliveries of a page's event on the caller's transaction, to every webhook of
/// its identity that is on and asks for it, but not for the identity's own doing unless the
/// webhook says so. Says how many.
pub(crate) async fn queue_page(conn: &mut PgConnection, change: PageChange) -> sqlx::Result<usize> {
    let subscribers = repo::subscribers(
        &mut *conn,
        change.workspace_id,
        &[change.identity],
        change.kind.as_str(),
    )
    .await?;
    let event = PageEvent {
        id: Uuid::now_v7(),
        kind: change.kind.as_str().to_owned(),
        occurred_at: i64::try_from(change.at.unix_timestamp_nanos() / 1000).unwrap_or(i64::MAX),
        workspace_id: change.workspace_id,
        page_id: change.page_id,
        identity_id: change.identity,
        actor_id: change.actor,
        thread_id: change.thread_id,
        comment_id: change.comment_id,
    };
    let mut queued = 0;
    for subscriber in subscribers.iter().filter(|subscriber| {
        subscriber.user_id == change.identity
            && (subscriber.include_own || Some(subscriber.user_id) != change.actor)
    }) {
        let payload = serde_json::to_value(Delivery {
            webhook_id: subscriber.webhook_id,
            event: Told::Page(event.clone()),
        })
        .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
        let job = NewJob::new(DELIVERY_KIND, payload).max_attempts(MAX_ATTEMPTS);
        let id = jobs::enqueue(&mut *conn, &job).await?;
        debug!(
            webhook_id = %subscriber.webhook_id,
            delivery_id = %id,
            event = change.kind.as_str(),
            "webhook delivery queued"
        );
        queued += 1;
    }
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use neon_domain::databases::{Choice as StatusOption, PropertyType, StatusGroup};
    use serde_json::json;

    use super::*;

    const ASSIGNEE: Uuid = Uuid::from_u128(1);
    const REVIEWER: Uuid = Uuid::from_u128(2);
    const STATUS: Uuid = Uuid::from_u128(3);
    const TODO: Uuid = Uuid::from_u128(4);
    const DOING: Uuid = Uuid::from_u128(5);
    const ADA: Uuid = Uuid::from_u128(10);
    const BEA: Uuid = Uuid::from_u128(11);

    fn schema() -> Schema {
        let option = |id, name: &str| StatusOption {
            id,
            name: name.to_owned(),
            hue: None,
            group: Some(StatusGroup::ToDo),
        };
        Schema {
            properties: vec![
                Property {
                    id: Uuid::from_u128(9),
                    name: "Name".into(),
                    ty: PropertyType::Title,
                },
                Property {
                    id: ASSIGNEE,
                    name: "Assignee".into(),
                    ty: PropertyType::Person,
                },
                Property {
                    id: REVIEWER,
                    name: "Reviewer".into(),
                    ty: PropertyType::Person,
                },
                Property {
                    id: STATUS,
                    name: "Status".into(),
                    ty: PropertyType::Status {
                        options: vec![option(TODO, "To do"), option(DOING, "Doing")],
                    },
                },
            ],
        }
    }

    fn cells(assignee: &[Uuid], reviewer: &[Uuid], status: Option<Uuid>) -> Json {
        let mut cells = json!({});
        if !assignee.is_empty() {
            cells[ASSIGNEE.to_string()] = json!(assignee);
        }
        if !reviewer.is_empty() {
            cells[REVIEWER.to_string()] = json!(reviewer);
        }
        if let Some(status) = status {
            cells[STATUS.to_string()] = json!(status);
        }
        cells
    }

    fn kinds(events: &[Detected]) -> Vec<(EventType, Uuid, Uuid)> {
        events
            .iter()
            .map(|event| (event.kind, event.identity, event.property))
            .collect()
    }

    #[test]
    fn adding_someone_to_a_person_property_assigns_them() {
        let schema = schema();
        let before = cells(&[ADA], &[], Some(TODO));
        let after = cells(&[ADA, BEA], &[ADA], Some(TODO));

        assert_eq!(
            kinds(&detect(&schema, Some(&before), &after)),
            [
                (EventType::Assigned, BEA, ASSIGNEE),
                (EventType::Assigned, ADA, REVIEWER)
            ]
        );
        // Taking someone off, or leaving them, assigns nobody.
        assert!(detect(&schema, Some(&after), &before).is_empty());
        assert!(detect(&schema, Some(&after), &after).is_empty());
    }

    #[test]
    fn a_new_row_assigns_everyone_in_it_and_changes_no_status() {
        let schema = schema();
        let after = cells(&[ADA], &[BEA], Some(DOING));

        assert_eq!(
            kinds(&detect(&schema, None, &after)),
            [
                (EventType::Assigned, ADA, ASSIGNEE),
                (EventType::Assigned, BEA, REVIEWER)
            ]
        );
    }

    #[test]
    fn a_status_change_tells_everyone_assigned_after_it() {
        let schema = schema();
        let before = cells(&[ADA], &[], Some(TODO));
        let after = cells(&[ADA], &[BEA], Some(DOING));

        let events = detect(&schema, Some(&before), &after);
        assert_eq!(
            kinds(&events),
            [
                (EventType::Assigned, BEA, REVIEWER),
                (EventType::StatusChanged, ADA, STATUS),
                (EventType::StatusChanged, BEA, STATUS)
            ]
        );
        let change = &events[1];
        assert_eq!(change.from.as_ref().map(|c| c.name.as_str()), Some("To do"));
        assert_eq!(change.to.as_ref().map(|c| c.name.as_str()), Some("Doing"));

        // Cleared, it changes too; to the same, it does not; with nobody assigned, nobody
        // hears.
        let cleared = cells(&[ADA], &[], None);
        let events = detect(&schema, Some(&before), &cleared);
        assert_eq!(kinds(&events), [(EventType::StatusChanged, ADA, STATUS)]);
        assert_eq!(events[0].to, None);
        assert!(detect(&schema, Some(&before), &before).is_empty());
        let nobody = cells(&[], &[], Some(TODO));
        assert!(detect(&schema, Some(&nobody), &cells(&[], &[], Some(DOING))).is_empty());
    }

    #[test]
    fn events_keep_their_time_to_the_microsecond() {
        let event = Event {
            id: Uuid::nil(),
            kind: EventType::Assigned.as_str().to_owned(),
            occurred_at: 1_795_348_800_000_001,
            workspace_id: Uuid::nil(),
            database_id: Uuid::nil(),
            row_id: Uuid::nil(),
            property_id: Uuid::nil(),
            identity_id: Uuid::nil(),
            from: None,
            to: None,
        };
        assert_eq!(event.occurred_at().unix_timestamp(), 1_795_348_800);
        assert_eq!(event.occurred_at().microsecond(), 1);
    }
}

#[cfg(test)]
mod told_tests {
    use super::*;

    #[test]
    fn deliveries_of_rows_and_of_pages_read_back_as_what_they_were() {
        let row = Delivery {
            webhook_id: Uuid::from_u128(1),
            event: Told::Row(Event {
                id: Uuid::from_u128(2),
                kind: EventType::Assigned.as_str().to_owned(),
                occurred_at: 1,
                workspace_id: Uuid::from_u128(3),
                database_id: Uuid::from_u128(4),
                row_id: Uuid::from_u128(5),
                property_id: Uuid::from_u128(6),
                identity_id: Uuid::from_u128(7),
                from: None,
                to: None,
            }),
        };
        let page = Delivery {
            webhook_id: Uuid::from_u128(1),
            event: Told::Page(PageEvent {
                id: Uuid::from_u128(8),
                kind: EventType::Mentioned.as_str().to_owned(),
                occurred_at: 2,
                workspace_id: Uuid::from_u128(3),
                page_id: Uuid::from_u128(9),
                identity_id: Uuid::from_u128(7),
                actor_id: Some(Uuid::from_u128(10)),
                thread_id: None,
                comment_id: None,
            }),
        };
        for delivery in [row, page] {
            let json = serde_json::to_value(&delivery).expect("serializes");
            let back: Delivery = serde_json::from_value(json).expect("reads");
            assert_eq!(back, delivery);
        }
        assert_eq!(
            Told::Page(PageEvent {
                id: Uuid::from_u128(8),
                kind: "page.mentioned".into(),
                occurred_at: 2,
                workspace_id: Uuid::nil(),
                page_id: Uuid::nil(),
                identity_id: Uuid::nil(),
                actor_id: None,
                thread_id: None,
                comment_id: None,
            })
            .kind(),
            "page.mentioned"
        );
    }
}
