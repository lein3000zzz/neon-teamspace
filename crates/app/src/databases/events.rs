//! What those following a database hear: a JSON message for each change, once it committed.
//!
//! Every message is an object whose `type` says what changed, with the database's id:
//!
//! | `type` | Fields |
//! |---|---|
//! | `ready` | `access`, `edit` or `view`: sent first, once the connection hears every change |
//! | `access` | `access`: what the person may do with the database now |
//! | `row_created`, `row_restored` | `row`: the row, as the API's query lists it, without `access` |
//! | `row_updated` | `row_id`, `values`: the values that changed, `null` for those cleared, by property id; `title` when it changed; `last_edited_at`, `last_edited_by` |
//! | `row_moved` | as `row_updated`, with `position`, the row's new key among the rows |
//! | `row_trashed` | `row_id` |
//! | `schema_changed` | nothing more: read the database again |
//! | `view_changed` | `view_id`, `deleted` |
//!
//! Each message goes to two rooms of the [`Broadcaster`]: one for members, and one for
//! guests, who know no other member, without the people values name or who changed rows.
//! Those of rows and schemas also go to the workspace's room, as members read them, for
//! what follows on the server whom rows are assigned to.

use bytes::Bytes;
use neon_domain::databases::Schema;
use neon_domain::pages::{Page, PageAccess};
use neon_store::databases as repo;
use serde_json::{Map, Value as Json, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::warn;
use uuid::Uuid;

use super::{RowView, StoredRow, row_view};
use crate::pages::PageService;
use crate::sync::{Broadcaster, ClientId, Room};

/// A row as it is after a change, to tell of it.
pub(crate) struct Snapshot {
    pub(crate) schema: Schema,
    pub(crate) page: Page,
    pub(crate) number: i64,
    pub(crate) cells: Json,
}

impl Snapshot {
    pub(crate) fn stored(&self) -> StoredRow<'_> {
        StoredRow {
            page: &self.page,
            number: self.number,
            cells: &self.cells,
        }
    }

    fn view(&self, database_id: Uuid, members_hidden: bool) -> RowView {
        row_view(
            &self.schema,
            Some(database_id),
            self.stored(),
            PageAccess::View,
            members_hidden,
        )
    }
}

/// A change to tell those following a database.
pub(crate) enum Event<'a> {
    RowCreated(&'a Snapshot),
    /// The row and the properties whose values changed.
    RowUpdated {
        row: &'a Snapshot,
        properties: &'a [Uuid],
    },
    /// The row, placed anew among the rows, and the properties whose values changed, as
    /// moving it to another group changes its value.
    RowMoved {
        row: &'a Snapshot,
        properties: &'a [Uuid],
    },
    RowTrashed {
        row_id: Uuid,
        workspace_id: Uuid,
    },
    RowRestored(&'a Snapshot),
    SchemaChanged {
        workspace_id: Uuid,
    },
    ViewChanged {
        view_id: Uuid,
        deleted: bool,
    },
}

impl Event<'_> {
    /// A short name, the message's `type`.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::RowCreated(_) => "row_created",
            Self::RowUpdated { .. } => "row_updated",
            Self::RowMoved { .. } => "row_moved",
            Self::RowTrashed { .. } => "row_trashed",
            Self::RowRestored(_) => "row_restored",
            Self::SchemaChanged { .. } => "schema_changed",
            Self::ViewChanged { .. } => "view_changed",
        }
    }

    /// The workspace of the database, for a change to its rows or schema.
    fn workspace_id(&self) -> Option<Uuid> {
        match self {
            Self::RowCreated(row)
            | Self::RowRestored(row)
            | Self::RowUpdated { row, .. }
            | Self::RowMoved { row, .. } => Some(row.page.workspace_id),
            Self::RowTrashed { workspace_id, .. } | Self::SchemaChanged { workspace_id } => {
                Some(*workspace_id)
            }
            Self::ViewChanged { .. } => None,
        }
    }

    /// The message as members, or guests, read it.
    pub(crate) fn render(&self, database_id: Uuid, members_hidden: bool) -> Json {
        let mut message = Map::new();
        message.insert("type".to_owned(), json!(self.name()));
        message.insert("database_id".to_owned(), json!(database_id.to_string()));
        match self {
            Self::RowCreated(row) | Self::RowRestored(row) => {
                message.insert(
                    "row".to_owned(),
                    row_json(&row.view(database_id, members_hidden)),
                );
            }
            Self::RowUpdated { row, properties } | Self::RowMoved { row, properties } => {
                let view = row.view(database_id, members_hidden);
                message.insert("row_id".to_owned(), json!(view.id.to_string()));
                let title = row.schema.title().map(|property| property.id);
                let values: Map<String, Json> = properties
                    .iter()
                    .map(|id| {
                        let key = id.to_string();
                        let value = view.values.get(&key).cloned().unwrap_or(Json::Null);
                        (key, value)
                    })
                    .collect();
                if properties.iter().any(|id| Some(*id) == title) {
                    message.insert("title".to_owned(), json!(view.title));
                }
                message.insert("values".to_owned(), Json::Object(values));
                if matches!(self, Self::RowMoved { .. }) {
                    message.insert("position".to_owned(), json!(view.position));
                }
                message.insert("last_edited_at".to_owned(), time_json(view.last_edited_at));
                message.insert("last_edited_by".to_owned(), id_json(view.last_edited_by));
            }
            Self::RowTrashed { row_id, .. } => {
                message.insert("row_id".to_owned(), json!(row_id.to_string()));
            }
            Self::SchemaChanged { .. } => {}
            Self::ViewChanged { view_id, deleted } => {
                message.insert("view_id".to_owned(), json!(view_id.to_string()));
                message.insert("deleted".to_owned(), json!(deleted));
            }
        }
        Json::Object(message)
    }
}

/// A row as a message holds it: as the API's query lists it, without `access` and `group`.
pub(crate) fn row_json(view: &RowView) -> Json {
    json!({
        "id": view.id.to_string(),
        "database_id": id_json(view.database_id),
        "number": view.number,
        "unique_id": view.unique_id,
        "title": view.title,
        "icon": view.icon,
        "position": view.position,
        "values": view.values,
        "created_at": time_json(view.created_at),
        "created_by": id_json(view.created_by),
        "last_edited_at": time_json(view.last_edited_at),
        "last_edited_by": id_json(view.last_edited_by),
        "trashed_at": view.trashed_at.map_or(Json::Null, time_json),
    })
}

fn time_json(at: OffsetDateTime) -> Json {
    json!(at.format(&Rfc3339).unwrap_or_default())
}

fn id_json(id: Option<Uuid>) -> Json {
    id.map_or(Json::Null, |id| json!(id.to_string()))
}

/// The message telling a connection what it may do with the database, first as it opens
/// (`ready`) and again when that changes (`access`).
pub(crate) fn access_message(kind: &'static str, database_id: Uuid, access: PageAccess) -> Bytes {
    Bytes::from(
        json!({
            "type": kind,
            "database_id": database_id.to_string(),
            "access": access.as_str(),
        })
        .to_string(),
    )
}

/// Tells both rooms of a database of a change, members and guests without the people,
/// and the workspace's room of a change to rows or the schema.
pub(crate) fn publish(
    broadcaster: &dyn Broadcaster,
    database_id: Uuid,
    except: Option<ClientId>,
    event: &Event<'_>,
) {
    for members_hidden in [false, true] {
        let message = Bytes::from(event.render(database_id, members_hidden).to_string());
        if !members_hidden && let Some(workspace_id) = event.workspace_id() {
            broadcaster.broadcast(Room::Workspace(workspace_id), message.clone(), None);
        }
        broadcaster.broadcast(
            Room::Database {
                id: database_id,
                members_hidden,
            },
            message,
            except,
        );
    }
    metrics::counter!(super::live::EVENTS, "type" => event.name()).increment(1);
}

/// What happened to a row through the page service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowChange {
    Renamed,
    Trashed,
    Restored,
}

/// Tells those following a row's database of a change the page service made to it, once it
/// committed. When the row cannot be read again, the log says so and nobody hears.
pub(crate) async fn row_changed(pages: &PageService, row_id: Uuid, change: RowChange) {
    let pool = pages.pool();
    let loaded = async {
        let Some(data) = repo::row(pool, row_id).await? else {
            return Ok(None);
        };
        let Some(page) = neon_store::pages::get(pool, row_id).await? else {
            return Ok(None);
        };
        let schema = Schema {
            properties: repo::properties(pool, data.database_id)
                .await?
                .into_iter()
                .map(|stored| stored.property)
                .collect(),
        };
        Ok::<_, sqlx::Error>(Some((
            data.database_id,
            Snapshot {
                schema,
                page,
                number: data.number,
                cells: data.cells,
            },
        )))
    }
    .await;
    let (database_id, row) = match loaded {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return,
        Err(error) => {
            warn!(%error, %row_id, "cannot read a changed row to tell of it");
            return;
        }
    };
    let title: Vec<Uuid> = row
        .schema
        .title()
        .map(|property| property.id)
        .into_iter()
        .collect();
    let event = match change {
        RowChange::Renamed => Event::RowUpdated {
            row: &row,
            properties: &title,
        },
        RowChange::Trashed => Event::RowTrashed {
            row_id,
            workspace_id: row.page.workspace_id,
        },
        RowChange::Restored => Event::RowRestored(&row),
    };
    publish(pages.broadcaster().as_ref(), database_id, None, &event);
}
