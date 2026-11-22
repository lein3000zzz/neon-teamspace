//! What an export writes of a database and its rows, as text: the table of a database's
//! rows, and the properties of a row.

use std::collections::HashMap;

use neon_domain::auth::Principal;
use neon_domain::databases::filter::Context as Today;
use neon_domain::databases::query::{self, Query};
use neon_domain::databases::{PropertyKind, Schema};
use neon_domain::workspaces::Action;
use neon_store::databases as repo;
use time::UtcOffset;
use uuid::Uuid;

use super::schema::member_names;
use super::{DatabaseError, DatabaseService, domain_row, fitted_views};

/// A database's rows as a table: the title and the properties its first view shows, in
/// that view's order, and a line of text for each row in the view's sort order. Every row
/// is there, whatever the view's filter, as an export holds them all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportTable {
    /// The properties' names, the title's first.
    pub columns: Vec<String>,
    /// Each row's values as text, in the columns' order.
    pub rows: Vec<Vec<String>>,
}

/// A row's properties as text: the name and value of each but the title that has a value,
/// in the schema's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportRow {
    pub properties: Vec<(String, String)>,
}

impl DatabaseService {
    /// The table of a database the caller sees, for its export. A guest's leaves out the
    /// people its values name.
    pub async fn export_table(
        &self,
        principal: &Principal,
        database_id: Uuid,
    ) -> Result<ExportTable, DatabaseError> {
        self.observe("export_table", async {
            let mut conn = self.pages().pool().acquire().await?;
            let opened = self.open(&mut conn, principal, database_id, false).await?;
            Self::check(&opened.context, Action::ViewPage)?;
            let schema = &opened.schema;
            let views = fitted_views(schema, repo::views(&mut *conn, database_id).await?);
            let view = views.first();
            let title = schema.title().into_iter();
            let shown = view
                .map(|view| {
                    view.settings
                        .properties
                        .iter()
                        .filter(|shown| shown.visible)
                        .filter_map(|shown| schema.get(shown.property))
                        .filter(|property| property.kind() != PropertyKind::Title)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let columns: Vec<_> = title.chain(shown).collect();
            let members_hidden = opened.context.members_hidden();
            let names = if members_hidden {
                HashMap::new()
            } else {
                member_names(&mut conn, opened.database.workspace_id).await?
            };
            let batch = opened.context.page().trashed_with;
            let listed = repo::rows(&mut *conn, database_id, batch, principal.user.id).await?;
            let rows: Vec<_> = listed
                .iter()
                .map(|listed| {
                    let row = domain_row(schema, &listed.page, listed.number, &listed.cells);
                    if members_hidden {
                        row.without_people(schema)
                    } else {
                        row
                    }
                })
                .collect();
            let sorts = view
                .map(|view| view.settings.sorts.as_slice())
                .unwrap_or_default();
            let today = Today {
                me: principal.user.id,
                offset: UtcOffset::UTC,
                today: self.now().date(),
            };
            let outcome = query::run(
                schema,
                Query {
                    filter: None,
                    sorts,
                    grouping: None,
                },
                &rows,
                &today,
                &names,
            );
            let lines = outcome
                .entries
                .iter()
                .map(|entry| {
                    let row = &rows[entry.row];
                    columns
                        .iter()
                        .map(|property| {
                            row.value(property)
                                .map(|value| value.display(&property.ty, &names))
                                .unwrap_or_default()
                        })
                        .collect()
                })
                .collect();
            Ok(ExportTable {
                columns: columns
                    .iter()
                    .map(|property| property.name.clone())
                    .collect(),
                rows: lines,
            })
        })
        .await
    }

    /// A row's properties as text, for its export. A guest's leave out the people.
    pub async fn export_row(
        &self,
        principal: &Principal,
        row_id: Uuid,
    ) -> Result<ExportRow, DatabaseError> {
        self.observe("export_row", async {
            let mut conn = self.pages().pool().acquire().await?;
            let context = self.pages().context(&mut conn, principal, row_id).await?;
            context.check(Action::ViewPage)?;
            let Some(data) = repo::row(&mut *conn, row_id).await? else {
                return Ok(ExportRow::default());
            };
            let schema = Schema {
                properties: repo::properties(&mut *conn, data.database_id)
                    .await?
                    .into_iter()
                    .map(|stored| stored.property)
                    .collect(),
            };
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
            let properties = schema
                .properties
                .iter()
                .filter(|property| property.kind() != PropertyKind::Title)
                .filter_map(|property| {
                    let text = row.value(property)?.display(&property.ty, &names);
                    (!text.is_empty()).then(|| (property.name.clone(), text))
                })
                .collect();
            Ok(ExportRow { properties })
        })
        .await
    }
}
