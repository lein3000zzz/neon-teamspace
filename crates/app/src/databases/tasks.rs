//! Rows as tasks: claiming one, and the rows assigned to someone.
//!
//! A row's assignees are the members its person properties name. Claiming a row names the
//! caller in a person property, the database's first in schema order unless the claim names
//! another, and moves the row to the first option in progress of the database's status
//! property, its first in schema order, in one transaction that holds the row: only while
//! that person property names nobody. Claims of one row at once take their turn on its lock,
//! so exactly one wins, and the others hear who holds it.

use std::collections::{HashMap, HashSet};

use neon_domain::auth::Principal;
use neon_domain::databases::{Property, PropertyKind, Schema, StatusGroup, Value};
use neon_domain::workspaces::Action;
use neon_store::databases as repo;
use serde_json::Value as Json;
use tracing::info;
use uuid::Uuid;

use super::events::{Event, Snapshot};
use super::rows::{
    Changes, Written, check_people, lock_row, patch, people_in, row_check, row_context, schema_of,
    snapshot_view,
};
use super::schema::member_names;
use super::{DatabaseError, DatabaseService, RowView, StoredRow, domain_row, row_view};
use crate::pages::{Facts, in_workspace};

/// The most tasks a list holds: the most recently changed.
pub const MAX_TASKS: usize = 100;

/// How many rows naming the caller are read to find those they may see.
const CANDIDATES: i64 = 500;

/// Someone a row's person property names already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub id: Uuid,
    /// As the workspace names them: an agent with its owner's name.
    pub name: String,
}

/// A row assigned to someone, as they see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub row: RowView,
    /// Its database's title, when they see the database.
    pub database_title: Option<String>,
    /// Its value of its database's status property, if it has one.
    pub status: Option<TaskStatus>,
}

/// A row with its properties, as [`DatabaseService::row_details`] tells it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowDetails {
    pub row: RowView,
    /// Every property of its database, in order, with the row's value as text, if any.
    pub properties: Vec<(Property, Option<String>)>,
}

/// A status option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatus {
    pub id: Uuid,
    pub name: String,
    pub group: Option<StatusGroup>,
}

/// The person property a claim names someone in: `named`, by id or name, else the first.
fn assignee_property<'a>(
    schema: &'a Schema,
    named: Option<&str>,
) -> Result<&'a Property, DatabaseError> {
    match named {
        Some(key) => {
            let property = schema.find(key).ok_or_else(|| {
                DatabaseError::InvalidInput(format!("the database has no property {key:?}"))
            })?;
            if property.kind() != PropertyKind::Person {
                return Err(DatabaseError::InvalidInput(format!(
                    "{:?} is no person property: a claim names someone in one",
                    property.name
                )));
            }
            Ok(property)
        }
        None => schema
            .properties
            .iter()
            .find(|property| property.kind() == PropertyKind::Person)
            .ok_or_else(|| {
                DatabaseError::InvalidInput(
                    "the database has no person property to name whoever claims a row in"
                        .to_owned(),
                )
            }),
    }
}

/// The status property a claim moves on, and the option it moves to: the first in progress.
fn in_progress(schema: &Schema) -> Result<(&Property, Uuid), DatabaseError> {
    let status = schema
        .properties
        .iter()
        .find(|property| property.kind() == PropertyKind::Status)
        .ok_or_else(|| {
            DatabaseError::InvalidInput(
                "the database has no status property for a claim to move the row on".to_owned(),
            )
        })?;
    let option = status
        .ty
        .options()
        .iter()
        .find(|option| option.group() == Some(StatusGroup::InProgress))
        .ok_or_else(|| {
            DatabaseError::InvalidInput(format!(
                "{:?} has no option in progress for a claim to move the row to",
                status.name
            ))
        })?;
    Ok((status, option.id))
}

/// A row's value of its schema's first status property.
fn status_of(schema: &Schema, cells: &Json) -> Option<TaskStatus> {
    let status = schema
        .properties
        .iter()
        .find(|property| property.kind() == PropertyKind::Status)?;
    let Some(Value::Choice(id)) = Value::stored(&status.ty, cells.get(status.id.to_string())?)
    else {
        return None;
    };
    let option = status.ty.options().iter().find(|option| option.id == id)?;
    Some(TaskStatus {
        id,
        name: option.name.clone(),
        group: option.group(),
    })
}

impl DatabaseService {
    /// Claims a row for the caller, as the module docs set out: names them in the
    /// person property `person`, or the first, and moves the row to the first status in
    /// progress, while that property names nobody, for those who edit the row. Otherwise
    /// [`DatabaseError::AlreadyClaimed`] says who holds it.
    pub async fn claim_row(
        &self,
        principal: &Principal,
        row_id: Uuid,
        person: Option<&str>,
    ) -> Result<RowView, DatabaseError> {
        self.observe("claim_row", async {
            let mut tx = self.pages().pool().begin().await?;
            let context = row_context(self, &mut tx, principal, row_id).await?;
            row_check(&context, Action::EditPage)?;
            if context.members_hidden() {
                return Err(DatabaseError::Forbidden(
                    "guests know no other member: they do not claim rows",
                ));
            }
            // Held until the claim commits: a claim at the same time waits, then finds it
            // claimed.
            let data = lock_row(&mut tx, &context).await?;
            let schema = schema_of(&mut tx, data.database_id).await?;
            let assignee = assignee_property(&schema, person)?;
            let (status, option) = in_progress(&schema)?;
            let holding: HashSet<Uuid> = data
                .cells
                .get(assignee.id.to_string())
                .map(people_in)
                .unwrap_or_default();
            if !holding.is_empty() {
                let names = member_names(&mut tx, context.page().workspace_id).await?;
                let mut holders: Vec<Holder> = holding
                    .into_iter()
                    .map(|id| Holder {
                        id,
                        name: names.get(&id).cloned().unwrap_or_default(),
                    })
                    .collect();
                holders.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                return Err(DatabaseError::AlreadyClaimed(holders));
            }
            let mut changes = Changes::new();
            changes.put(assignee, Some(Value::People(vec![principal.user.id])));
            changes.put(status, Some(Value::Choice(option)));
            check_people(&mut tx, &context, &changes, Some(&data.cells)).await?;
            let mut page = context.page().clone();
            page.last_edited_by = Some(principal.user.id);
            page.updated_at = self.now();
            repo::touch_row(&mut *tx, row_id, principal.user.id, page.updated_at).await?;
            let (cells, notifier) = patch(
                &mut tx,
                self.pages(),
                Written {
                    page: &page,
                    database_id: data.database_id,
                    actor: principal.user.id,
                    schema: &schema,
                    before: &data.cells,
                },
                &changes,
            )
            .await?;
            tx.commit().await?;
            notifier.tell();
            info!(
                user_id = %principal.user.id,
                database_id = %data.database_id,
                row_id = %row_id,
                "database row claimed"
            );
            let snapshot = Snapshot {
                schema,
                page,
                number: data.number,
                cells,
            };
            self.publish(
                data.database_id,
                &Event::RowUpdated {
                    row: &snapshot,
                    properties: &changes.properties,
                },
            );
            Ok(snapshot_view(&snapshot, data.database_id, &context))
        })
        .await
    }

    /// The rows of a workspace's databases that a person property names the caller in, out
    /// of the trash, that they may see: at most [`MAX_TASKS`], the most recently changed
    /// first.
    pub async fn tasks(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Vec<Task>, DatabaseError> {
        self.observe("tasks", async {
            let mut conn = self.pages().pool().acquire().await?;
            let standing = in_workspace(&mut conn, principal, workspace_id).await?;
            let members_hidden = standing.members_hidden();
            let ids = repo::assigned_rows(&mut *conn, workspace_id, principal.user.id, CANDIDATES)
                .await?;
            let mut seen = Facts::read(&mut conn, principal, &ids)
                .await?
                .seen(principal)?;
            let visible: Vec<_> = ids
                .iter()
                .filter_map(|id| seen.remove(id).flatten())
                .filter(|view| !view.page.is_trashed())
                .take(MAX_TASKS)
                .collect();
            let rows: HashMap<Uuid, repo::RowData> = repo::rows_of(
                &mut *conn,
                &visible.iter().map(|view| view.page.id).collect::<Vec<_>>(),
            )
            .await?
            .into_iter()
            .map(|row| (row.page_id, row))
            .collect();
            let databases: Vec<Uuid> = rows
                .values()
                .map(|row| row.database_id)
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            let shown: HashMap<Uuid, String> = Facts::read(&mut conn, principal, &databases)
                .await?
                .seen(principal)?
                .into_iter()
                .filter_map(|(id, view)| Some((id, view?.page.title)))
                .collect();
            let mut schemas: HashMap<Uuid, Schema> = HashMap::new();
            for database_id in &databases {
                schemas.insert(*database_id, schema_of(&mut conn, *database_id).await?);
            }
            Ok(visible
                .into_iter()
                .filter_map(|view| {
                    let data = rows.get(&view.page.id)?;
                    let schema = schemas.get(&data.database_id)?;
                    let title = shown.get(&data.database_id).cloned();
                    // Someone who does not see the database sees none of its properties,
                    // the status among them.
                    let status = title.as_ref().and_then(|_| status_of(schema, &data.cells));
                    let row = row_view(
                        schema,
                        title.is_some().then_some(data.database_id),
                        StoredRow {
                            page: &view.page,
                            number: data.number,
                            cells: &data.cells,
                        },
                        view.access,
                        members_hidden,
                    );
                    Some(Task {
                        row,
                        database_title: title,
                        status,
                    })
                })
                .collect())
        })
        .await
    }

    /// A row the caller sees, with its values, and each property of its database with the
    /// row's value of it as people read it: options and people by name, numbers in their
    /// format. A guest's leave out the people. Someone who sees the row through its own
    /// share alone, not its database, gets none of the database's properties.
    pub async fn row_details(
        &self,
        principal: &Principal,
        row_id: Uuid,
    ) -> Result<RowDetails, DatabaseError> {
        self.observe("get_row", async {
            let mut conn = self.pages().pool().acquire().await?;
            let context = row_context(self, &mut conn, principal, row_id).await?;
            row_check(&context, Action::ViewPage)?;
            let data = repo::row(&mut *conn, row_id)
                .await?
                .ok_or_else(|| DatabaseError::Internal(format!("row {row_id} has no data")))?;
            let schema = schema_of(&mut conn, data.database_id).await?;
            let members_hidden = context.members_hidden();
            let names = if members_hidden {
                HashMap::new()
            } else {
                member_names(&mut conn, context.page().workspace_id).await?
            };
            let mut row = domain_row(&schema, context.page(), data.number, &data.cells);
            if members_hidden {
                row = row.without_people(&schema);
            }
            let shown = if context.parent_seen() {
                schema.properties.as_slice()
            } else {
                &[]
            };
            let texts = shown
                .iter()
                .map(|property| {
                    let text = row
                        .value(property)
                        .map(|value| value.display(&property.ty, &names));
                    (property.clone(), text.filter(|text| !text.is_empty()))
                })
                .collect();
            let snapshot = Snapshot {
                schema,
                page: context.page().clone(),
                number: data.number,
                cells: data.cells,
            };
            Ok(RowDetails {
                row: snapshot_view(&snapshot, data.database_id, &context),
                properties: texts,
            })
        })
        .await
    }
}
