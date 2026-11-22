//! A database's rows: querying them, making them, reading and changing their values, moving
//! them, and finding one by its unique id.

use std::collections::{HashMap, HashSet};

use neon_domain::auth::Principal;
use neon_domain::databases::filter::{Context as Today, FilterSpec, Fit};
use neon_domain::databases::query::{self, Group, Query};
use neon_domain::databases::sort::{SortSpec, resolve_sorts};
use neon_domain::databases::{
    GroupKey, MAX_ROW_BYTES, MAX_ROWS, Property, PropertyKind, Schema, Value, View, parse_unique_id,
};
use neon_domain::pages::{Page, PageAccess, PageKind, PageStyle, Placement};
use neon_domain::workspaces::Action;
use neon_store::databases::{self as repo};
use neon_store::pages::{self as pages_repo, Siblings};
use serde_json::{Map, Value as Json};
use sqlx::PgConnection;
use time::UtcOffset;
use tracing::info;
use uuid::Uuid;

use super::events::{Event, Snapshot};
use super::schema::member_names;
use super::{
    DatabaseError, DatabaseService, NO_DATABASE, NO_ROW, NO_VIEW, RowView, StoredRow, cell_keys,
    domain_row, fitted_views, names_people, not_found, row_view,
};
use crate::pages::{Context, PageError, access_of};

/// The most rows a query answers with at once, and how many unless told fewer.
pub const MAX_PAGE: usize = 1000;
pub const DEFAULT_PAGE: usize = 100;

/// The farthest a UTC offset goes, in minutes either way.
const MAX_OFFSET_MINUTES: i16 = 14 * 60;

/// What to query: a view's rows, or every row, with a filter and sorts of the caller's in
/// place of the view's, a part at a time.
#[derive(Debug, Clone, Default)]
pub struct QueryInput {
    pub view_id: Option<Uuid>,
    /// Replaces the view's filter.
    pub filter: Option<FilterSpec>,
    /// Replaces the view's sorts.
    pub sorts: Option<Vec<SortSpec>>,
    /// 1 to [`MAX_PAGE`]; [`DEFAULT_PAGE`] unless set.
    pub limit: Option<usize>,
    /// The `next` of the part before.
    pub after: Option<String>,
    /// The caller's UTC offset, which dates are told at: 0 unless set.
    pub utc_offset_minutes: Option<i16>,
}

/// A part of a query's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    /// In order; grouped, in the order of the groups, each row once in each group it is in.
    pub rows: Vec<RowView>,
    /// For a view that groups: every group, hidden ones too, with how many rows passed
    /// the filter in it.
    pub groups: Option<Vec<Group>>,
    /// How many entries the whole answer holds.
    pub total: usize,
    /// Where the next part starts, when there is more.
    pub next: Option<String>,
}

/// A row to make.
#[derive(Debug, Clone, Default)]
pub struct NewRow {
    /// Values by property id or name; the title property's is the row's title.
    pub values: Map<String, Json>,
    pub placement: Placement,
    /// The group of a view the row goes in, as a card added to a board's column: its
    /// value of the view's grouping property.
    pub group: Option<(Uuid, GroupKey)>,
}

/// Where a row goes: elsewhere among the rows, into another group of a view, or both.
#[derive(Debug, Clone, Default)]
pub struct RowMove {
    pub placement: Option<Placement>,
    pub group: Option<GroupMove>,
}

/// A row moving from one group of a view to another. For a person property, the person of
/// `from` makes room for the one of `to`, and the others stay.
#[derive(Debug, Clone, Copy)]
pub struct GroupMove {
    pub view_id: Uuid,
    pub from: GroupKey,
    pub to: GroupKey,
}

/// What a change to a row's values sets.
struct Changes {
    title: Option<String>,
    set: Map<String, Json>,
    remove: Vec<String>,
    /// Every property the change names, the title included.
    properties: Vec<Uuid>,
    /// The people the change names, who must be members.
    people: Vec<Uuid>,
}

impl Changes {
    fn new() -> Self {
        Self {
            title: None,
            set: Map::new(),
            remove: Vec::new(),
            properties: Vec::new(),
            people: Vec::new(),
        }
    }

    /// Sets, or clears, a stored property's value.
    fn put(&mut self, property: &Property, value: Option<Value>) {
        let key = property.id.to_string();
        self.set.remove(&key);
        self.remove.retain(|removed| *removed != key);
        if !self.properties.contains(&property.id) {
            self.properties.push(property.id);
        }
        match value {
            Some(value) => {
                if let Value::People(ids) = &value {
                    self.people.extend(ids);
                }
                self.set.insert(key, value.to_json());
            }
            None => self.remove.push(key),
        }
    }
}

/// The changes `values` asks for, by property id or name, each value checked against its
/// property's type.
fn parse_values(schema: &Schema, values: &Map<String, Json>) -> Result<Changes, DatabaseError> {
    let mut changes = Changes::new();
    for (key, json) in values {
        let property = schema.find(key).ok_or_else(|| {
            DatabaseError::InvalidInput(format!("the database has no property {key:?}"))
        })?;
        if changes.properties.contains(&property.id) {
            return Err(DatabaseError::InvalidInput(format!(
                "the values name {:?} twice",
                property.name
            )));
        }
        let value = Value::parse(&property.ty, json).map_err(|error| {
            DatabaseError::InvalidInput(format!("{:?}: {error}", property.name))
        })?;
        if property.kind() == PropertyKind::Title {
            changes.title = Some(match value {
                Some(Value::Text(title)) => title,
                _ => String::new(),
            });
            changes.properties.push(property.id);
        } else {
            changes.put(property, value);
        }
    }
    Ok(changes)
}

impl DatabaseService {
    /// The rows of a database the caller sees, as a view has them or with a filter and sorts
    /// of their own, a part at a time. A guest sees the rows without the people they name.
    pub async fn query(
        &self,
        principal: &Principal,
        database_id: Uuid,
        input: QueryInput,
    ) -> Result<QueryResult, DatabaseError> {
        self.observe("query", async {
            let limit = page_limit(input.limit)?;
            let offset = input.after.as_deref().map(cursor).transpose()?.unwrap_or(0);
            let utc = utc_offset(input.utc_offset_minutes)?;
            let mut conn = self.pages().pool().acquire().await?;
            let opened = self.open(&mut conn, principal, database_id, false).await?;
            Self::check(&opened.context, Action::ViewPage)?;
            let view = match input.view_id {
                Some(view_id) => {
                    Some(view_of(&mut conn, &opened.schema, database_id, view_id).await?)
                }
                None => None,
            };
            let settings = view.as_ref().map(|view| &view.settings);
            let filter = match &input.filter {
                Some(spec) => spec.resolve(&opened.schema, Fit::Strict)?,
                None => settings.and_then(|settings| settings.filter.clone()),
            };
            let sorts = match &input.sorts {
                Some(specs) => resolve_sorts(specs, &opened.schema, Fit::Strict)?,
                None => settings
                    .map(|settings| settings.sorts.clone())
                    .unwrap_or_default(),
            };
            let grouping = settings.and_then(|settings| settings.grouping.clone());

            // Every row is read with the values the filter, sorts and grouping need; those
            // of the part answered with, in full, once it is known.
            let needed: Vec<Uuid> = filter
                .iter()
                .flat_map(|filter| filter.conditions())
                .map(|condition| condition.property)
                .chain(sorts.iter().map(|sort| sort.property))
                .chain(grouping.iter().map(|grouping| grouping.property))
                .collect();
            let batch = opened.context.page().trashed_with;
            let keys = cell_keys(&opened.schema, &needed);
            let listed = repo::rows(
                &mut *conn,
                database_id,
                batch,
                principal.user.id,
                Some(&keys),
            )
            .await?;
            let members_hidden = opened.context.members_hidden();
            let by_name = sorts
                .iter()
                .map(|sort| sort.property)
                .chain(grouping.iter().map(|grouping| grouping.property))
                .any(|id| names_people(&opened.schema, id));
            let names = if members_hidden || !by_name {
                HashMap::new()
            } else {
                member_names(&mut conn, opened.database.workspace_id).await?
            };
            let rows: Vec<_> = listed
                .iter()
                .map(|listed| {
                    let row =
                        domain_row(&opened.schema, &listed.page, listed.number, &listed.cells);
                    if members_hidden {
                        row.without_people(&opened.schema)
                    } else {
                        row
                    }
                })
                .collect();
            let now = self.now().to_offset(utc);
            let today = Today {
                me: principal.user.id,
                offset: utc,
                today: now.date(),
            };
            let schema = opened.schema.clone();
            // Up to every row of the database, filtered and sorted: off the async workers.
            let outcome = tokio::task::spawn_blocking(move || {
                query::run(
                    &schema,
                    Query {
                        filter: filter.as_ref(),
                        sorts: &sorts,
                        grouping: grouping.as_ref(),
                    },
                    &rows,
                    &today,
                    &names,
                )
            })
            .await
            .map_err(|error| DatabaseError::Internal(format!("a query's work ended: {error}")))?;
            let total = outcome.entries.len();
            let end = offset.saturating_add(limit).min(total);
            let part = outcome
                .entries
                .get(offset.min(total)..end)
                .unwrap_or_default();
            let ids: Vec<Uuid> = part.iter().map(|entry| listed[entry.row].page.id).collect();
            let cells: HashMap<Uuid, Json> = repo::cells_of_rows(&mut *conn, &ids)
                .await?
                .into_iter()
                .collect();
            let shared = opened.context.chain[0].shared;
            let rows = part
                .iter()
                .map(|entry| {
                    let listed = &listed[entry.row];
                    let access = access_of(
                        opened.context.actor,
                        opened.context.space,
                        shared.max(listed.shared),
                    )
                    .unwrap_or(PageAccess::View);
                    let stored = StoredRow {
                        page: &listed.page,
                        number: listed.number,
                        // A row deleted since shows the values it was listed with.
                        cells: cells.get(&listed.page.id).unwrap_or(&listed.cells),
                    };
                    let mut view =
                        row_view(&opened.schema, database_id, stored, access, members_hidden);
                    view.group = entry.group;
                    view
                })
                .collect();
            Ok(QueryResult {
                rows,
                groups: outcome.groups,
                total,
                next: (end < total).then(|| end.to_string()),
            })
        })
        .await
    }

    /// Adds a row, for those who edit the database, with its values, placed among the rows
    /// and, if asked, in a group of a view. Its number is the database's next.
    pub async fn create_row(
        &self,
        principal: &Principal,
        database_id: Uuid,
        new: NewRow,
    ) -> Result<RowView, DatabaseError> {
        self.observe("create_row", async {
            let pages = self.pages();
            let mut tx = pages.pool().begin().await?;
            let database_page = pages_repo::get(&mut *tx, database_id)
                .await?
                .ok_or(DatabaseError::NotFound(NO_DATABASE))?;
            pages_repo::lock_tree(&mut *tx, database_page.workspace_id).await?;
            let opened = self.open(&mut tx, principal, database_id, true).await?;
            Self::check(&opened.context, Action::EditPage)?;
            let rows = repo::count_rows(&mut *tx, database_id).await?;
            if usize::try_from(rows).unwrap_or(usize::MAX) >= MAX_ROWS {
                return Err(PageError::TooManyRows.into());
            }
            let mut changes = parse_values(&opened.schema, &new.values)?;
            if let Some((view_id, key)) = new.group {
                let view = view_of(&mut tx, &opened.schema, database_id, view_id).await?;
                let property = grouping_property(&opened.schema, &view)?;
                let value = group_value(property, key, None)?;
                changes.put(property, value);
            }
            check_people(&mut tx, &opened.context, &changes).await?;
            let number = repo::take_number(&mut *tx, database_id).await?;
            let database = opened.context.page();
            let siblings = Siblings::of(database.workspace_id, Some(database_id), database.space);
            let position = pages
                .position(&mut tx, siblings, None, new.placement)
                .await?;
            let now = self.now();
            let user = principal.user.id;
            let page = Page {
                id: Uuid::now_v7(),
                workspace_id: database.workspace_id,
                kind: PageKind::Row,
                space: database.space,
                parent_id: Some(database_id),
                position,
                title: changes.title.clone().unwrap_or_default(),
                icon: None,
                cover: None,
                style: PageStyle::default(),
                created_by: Some(user),
                last_edited_by: Some(user),
                created_at: now,
                updated_at: now,
                content_updated_at: now,
                trashed_at: None,
                trashed_by: None,
                trashed_with: None,
            };
            pages_repo::insert(&mut *tx, &page).await?;
            let cells = Json::Object(changes.set);
            let bytes = repo::insert_row(&mut *tx, page.id, database_id, number, &cells).await?;
            check_size(bytes)?;
            tx.commit().await?;
            info!(
                user_id = %user,
                database_id = %database_id,
                row_id = %page.id,
                number,
                values = changes.properties.len(),
                "database row created"
            );
            let snapshot = Snapshot {
                schema: opened.schema.clone(),
                page,
                number,
                cells,
            };
            self.publish(database_id, &Event::RowCreated(&snapshot))
                .await;
            let access = opened.context.access().unwrap_or(PageAccess::Edit);
            Ok(row_view(
                &opened.schema,
                database_id,
                snapshot.stored(),
                access,
                opened.context.members_hidden(),
            ))
        })
        .await
    }

    /// A row the caller sees, with its values. One in the trash shows only to those who
    /// could edit it.
    pub async fn row(&self, principal: &Principal, row_id: Uuid) -> Result<RowView, DatabaseError> {
        self.observe("get_row", async {
            let mut conn = self.pages().pool().acquire().await?;
            let (view, _) = self.load_row(&mut conn, principal, row_id).await?;
            Ok(view)
        })
        .await
    }

    /// The row of a database whose unique id is `unique_id`: `NW-42`, or its number alone.
    pub async fn row_by_unique_id(
        &self,
        principal: &Principal,
        database_id: Uuid,
        unique_id: &str,
    ) -> Result<RowView, DatabaseError> {
        self.observe("get_row_by_unique_id", async {
            let mut conn = self.pages().pool().acquire().await?;
            let opened = self.open(&mut conn, principal, database_id, false).await?;
            Self::check(&opened.context, Action::ViewPage)?;
            let number = parse_unique_id(unique_id, opened.prefix())?;
            let row_id = repo::row_by_number(&mut *conn, database_id, number)
                .await?
                .ok_or(DatabaseError::NotFound(
                    "no row of the database has this unique id",
                ))?;
            let (view, _) = self.load_row(&mut conn, principal, row_id).await?;
            Ok(view)
        })
        .await
    }

    /// Sets the values `values` names, by property id or name, and leaves the others: the
    /// title property's is the row's title, `null` clears a value, and computed properties
    /// take none. For those who edit the row.
    pub async fn update_row(
        &self,
        principal: &Principal,
        row_id: Uuid,
        values: Map<String, Json>,
    ) -> Result<RowView, DatabaseError> {
        self.observe("update_row", async {
            if values.is_empty() {
                return Err(DatabaseError::InvalidInput(
                    "name at least one property to change".to_owned(),
                ));
            }
            let mut tx = self.pages().pool().begin().await?;
            let context = row_context(self, &mut tx, principal, row_id).await?;
            row_check(&context, Action::EditPage)?;
            let data = repo::row(&mut *tx, row_id)
                .await?
                .ok_or_else(|| DatabaseError::Internal(format!("row {row_id} has no data")))?;
            let schema = schema_of(&mut tx, data.database_id).await?;
            let changes = parse_values(&schema, &values)?;
            check_people(&mut tx, &context, &changes).await?;
            let mut page = context.page().clone();
            page.last_edited_by = Some(principal.user.id);
            page.updated_at = self.now();
            match &changes.title {
                Some(title) => {
                    page.title = title.clone();
                    pages_repo::update(&mut *tx, &page).await?;
                }
                None => {
                    repo::touch_row(&mut *tx, row_id, principal.user.id, page.updated_at).await?;
                }
            }
            let cells = patch(&mut tx, row_id, &changes).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %data.database_id,
                row_id = %row_id,
                values = changes.properties.len(),
                "database row changed"
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
            )
            .await;
            Ok(snapshot_view(&snapshot, data.database_id, &context))
        })
        .await
    }

    /// Moves a row among its database's rows, which needs edit access to the database too,
    /// into another group of a view, which changes its value of the view's grouping
    /// property, or both at once.
    pub async fn move_row(
        &self,
        principal: &Principal,
        row_id: Uuid,
        to: RowMove,
    ) -> Result<RowView, DatabaseError> {
        self.observe("move_row", async {
            if to.placement.is_none() && to.group.is_none() {
                return Err(DatabaseError::InvalidInput(
                    "name where the row goes: a placement, a group, or both".to_owned(),
                ));
            }
            let pages = self.pages();
            let mut tx = pages.pool().begin().await?;
            if to.placement.is_some() {
                let page = pages_repo::get(&mut *tx, row_id)
                    .await?
                    .ok_or(DatabaseError::NotFound(NO_ROW))?;
                pages_repo::lock_tree(&mut *tx, page.workspace_id).await?;
            }
            let context = row_context(self, &mut tx, principal, row_id).await?;
            row_check(&context, Action::EditPage)?;
            let data = repo::row(&mut *tx, row_id)
                .await?
                .ok_or_else(|| DatabaseError::Internal(format!("row {row_id} has no data")))?;
            let database_id = data.database_id;
            let mut page = context.page().clone();
            if let Some(placement) = to.placement {
                let database = pages
                    .context(&mut tx, principal, database_id)
                    .await
                    .map_err(not_found)?;
                DatabaseService::check(&database, Action::EditPage)?;
                let siblings = Siblings::of(page.workspace_id, Some(database_id), page.space);
                page.position = pages
                    .position(&mut tx, siblings, Some(row_id), placement)
                    .await?;
                repo::set_row_position(&mut *tx, row_id, &page.position).await?;
            }
            let schema = schema_of(&mut tx, database_id).await?;
            let mut changes = Changes::new();
            if let Some(group) = to.group {
                let view = view_of(&mut tx, &schema, database_id, group.view_id).await?;
                let property = grouping_property(&schema, &view)?;
                let current = domain_row(&schema, &page, data.number, &data.cells).value(property);
                let value = group_value(property, group.to, Some((group.from, current)))?;
                changes.put(property, value);
            }
            check_people(&mut tx, &context, &changes).await?;
            page.last_edited_by = Some(principal.user.id);
            page.updated_at = self.now();
            repo::touch_row(&mut *tx, row_id, principal.user.id, page.updated_at).await?;
            let cells = patch(&mut tx, row_id, &changes).await?;
            tx.commit().await?;
            info!(
                user_id = %principal.user.id,
                database_id = %database_id,
                row_id = %row_id,
                placed = to.placement.is_some(),
                regrouped = to.group.is_some(),
                "database row moved"
            );
            let snapshot = Snapshot {
                schema,
                page,
                number: data.number,
                cells,
            };
            self.publish(
                database_id,
                &Event::RowMoved {
                    row: &snapshot,
                    properties: &changes.properties,
                },
            )
            .await;
            Ok(snapshot_view(&snapshot, database_id, &context))
        })
        .await
    }

    /// A row the principal may see, with its values, and its schema.
    async fn load_row(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        row_id: Uuid,
    ) -> Result<(RowView, Schema), DatabaseError> {
        let context = row_context(self, conn, principal, row_id).await?;
        row_check(&context, Action::ViewPage)?;
        let data = repo::row(&mut *conn, row_id)
            .await?
            .ok_or_else(|| DatabaseError::Internal(format!("row {row_id} has no data")))?;
        let schema = schema_of(conn, data.database_id).await?;
        let snapshot = Snapshot {
            schema,
            page: context.page().clone(),
            number: data.number,
            cells: data.cells,
        };
        let view = snapshot_view(&snapshot, data.database_id, &context);
        Ok((view, snapshot.schema))
    }
}

/// A row as the principal of `context`, the row's, sees it.
fn snapshot_view(snapshot: &Snapshot, database_id: Uuid, context: &Context) -> RowView {
    row_view(
        &snapshot.schema,
        database_id,
        snapshot.stored(),
        context.access().unwrap_or(PageAccess::View),
        context.members_hidden(),
    )
}

/// A row's page and what the rules need to know of it; a page that is not a row is not
/// found as one.
async fn row_context(
    service: &DatabaseService,
    conn: &mut PgConnection,
    principal: &Principal,
    row_id: Uuid,
) -> Result<Context, DatabaseError> {
    let context = service
        .pages()
        .context(&mut *conn, principal, row_id)
        .await
        .map_err(|error| match error {
            PageError::NotFound(_) => DatabaseError::NotFound(NO_ROW),
            other => other.into(),
        })?;
    if context.page().kind != PageKind::Row {
        return Err(DatabaseError::NotFound(NO_ROW));
    }
    Ok(context)
}

fn row_check(context: &Context, action: Action) -> Result<(), DatabaseError> {
    context.check(action).map_err(|error| match error {
        PageError::NotFound(_) => DatabaseError::NotFound(NO_ROW),
        other => other.into(),
    })
}

async fn schema_of(conn: &mut PgConnection, database_id: Uuid) -> Result<Schema, DatabaseError> {
    Ok(Schema {
        properties: repo::properties(&mut *conn, database_id)
            .await?
            .into_iter()
            .map(|stored| stored.property)
            .collect(),
    })
}

/// A view of a database, fitted to its schema.
async fn view_of(
    conn: &mut PgConnection,
    schema: &Schema,
    database_id: Uuid,
    view_id: Uuid,
) -> Result<View, DatabaseError> {
    fitted_views(schema, repo::views(&mut *conn, database_id).await?)
        .into_iter()
        .find(|view| view.id == view_id)
        .ok_or(DatabaseError::NotFound(NO_VIEW))
}

/// The property a view groups its rows by.
fn grouping_property<'a>(schema: &'a Schema, view: &View) -> Result<&'a Property, DatabaseError> {
    view.settings
        .grouping
        .as_ref()
        .and_then(|grouping| schema.get(grouping.property))
        .ok_or_else(|| DatabaseError::InvalidInput("the view does not group its rows".to_owned()))
}

/// A row's value of a grouping property once it is in the group `to`: the option, or no
/// value; for a person property, its people but the one of `from`, with the one of `to`.
fn group_value(
    property: &Property,
    to: GroupKey,
    from: Option<(GroupKey, Option<Value>)>,
) -> Result<Option<Value>, DatabaseError> {
    match property.kind() {
        PropertyKind::Person => {
            let (from, current) = from.unwrap_or((None, None));
            let mut people = match current {
                Some(Value::People(ids)) => ids,
                _ => Vec::new(),
            };
            if let Some(from) = from {
                people.retain(|id| *id != from);
            }
            if let Some(to) = to
                && !people.contains(&to)
            {
                people.push(to);
            }
            Ok((!people.is_empty()).then_some(Value::People(people)))
        }
        _ => match to {
            Some(id) if property.ty.options().iter().any(|option| option.id == id) => {
                Ok(Some(Value::Choice(id)))
            }
            Some(_) => Err(DatabaseError::InvalidInput(format!(
                "the group is no option of {:?}",
                property.name
            ))),
            None => Ok(None),
        },
    }
}

/// Writes a change to a row's cells, within [`MAX_ROW_BYTES`], and answers them as they are
/// then.
async fn patch(
    conn: &mut PgConnection,
    row_id: Uuid,
    changes: &Changes,
) -> Result<Json, DatabaseError> {
    let patched = repo::patch_cells(
        &mut *conn,
        row_id,
        &Json::Object(changes.set.clone()),
        &changes.remove,
    )
    .await?
    .ok_or_else(|| DatabaseError::Internal(format!("row {row_id} has no data")))?;
    check_size(patched.bytes)?;
    Ok(patched.cells)
}

/// Refuses a row whose values would take more than [`MAX_ROW_BYTES`].
pub(crate) fn check_size(bytes: i64) -> Result<(), DatabaseError> {
    if usize::try_from(bytes).unwrap_or(usize::MAX) > MAX_ROW_BYTES {
        return Err(DatabaseError::TooMany(format!(
            "a row's values take at most {} KiB together, as JSON: this would make them larger",
            MAX_ROW_BYTES / 1024
        )));
    }
    Ok(())
}

/// Refuses people who are not members of the workspace, and any to a guest, who knows no
/// other member.
async fn check_people(
    conn: &mut PgConnection,
    context: &Context,
    changes: &Changes,
) -> Result<(), DatabaseError> {
    if changes.people.is_empty() {
        return Ok(());
    }
    if context.members_hidden() {
        return Err(DatabaseError::Forbidden(
            "guests know no other member: they do not set people",
        ));
    }
    let wanted: HashSet<Uuid> = changes.people.iter().copied().collect();
    let ids: Vec<Uuid> = wanted.iter().copied().collect();
    let members: HashSet<Uuid> = repo::members_among(&mut *conn, context.page().workspace_id, &ids)
        .await?
        .into_iter()
        .collect();
    if members != wanted {
        return Err(DatabaseError::InvalidInput(
            "a person property names members of the workspace".to_owned(),
        ));
    }
    Ok(())
}

fn page_limit(limit: Option<usize>) -> Result<usize, DatabaseError> {
    match limit {
        None => Ok(DEFAULT_PAGE),
        Some(limit) if (1..=MAX_PAGE).contains(&limit) => Ok(limit),
        Some(_) => Err(DatabaseError::InvalidInput(format!(
            "a query answers with 1 to {MAX_PAGE} rows at a time"
        ))),
    }
}

/// A query's cursor: how many entries the parts before held.
fn cursor(text: &str) -> Result<usize, DatabaseError> {
    text.parse::<usize>()
        .ok()
        .filter(|_| text.len() <= 8)
        .ok_or_else(|| {
            DatabaseError::InvalidInput(
                "this cursor comes from no query: pass back the one a query answered".to_owned(),
            )
        })
}

fn utc_offset(minutes: Option<i16>) -> Result<UtcOffset, DatabaseError> {
    let minutes = minutes.unwrap_or(0);
    if !(-MAX_OFFSET_MINUTES..=MAX_OFFSET_MINUTES).contains(&minutes) {
        return Err(DatabaseError::InvalidInput(format!(
            "a UTC offset is -{MAX_OFFSET_MINUTES} to {MAX_OFFSET_MINUTES} minutes"
        )));
    }
    UtcOffset::from_whole_seconds(i32::from(minutes) * 60)
        .map_err(|_| DatabaseError::InvalidInput("not a UTC offset".to_owned()))
}

#[cfg(test)]
mod tests {
    use neon_domain::databases::{Choice, PropertyType};

    use super::*;

    #[test]
    fn query_bounds_are_checked() {
        assert_eq!(page_limit(None).ok(), Some(DEFAULT_PAGE));
        assert_eq!(page_limit(Some(MAX_PAGE)).ok(), Some(MAX_PAGE));
        assert!(page_limit(Some(0)).is_err());
        assert!(page_limit(Some(MAX_PAGE + 1)).is_err());
        assert_eq!(cursor("200").ok(), Some(200));
        for bad in ["", "-1", "x", "123456789"] {
            assert!(cursor(bad).is_err(), "{bad:?}");
        }
        assert_eq!(utc_offset(None).ok(), Some(UtcOffset::UTC));
        assert_eq!(
            utc_offset(Some(-330)).ok(),
            UtcOffset::from_hms(-5, -30, 0).ok()
        );
        assert!(utc_offset(Some(15 * 60)).is_err());
    }

    fn property(ty: PropertyType) -> Property {
        Property {
            id: Uuid::from_u128(1),
            name: "Group".into(),
            ty,
        }
    }

    #[test]
    fn moving_between_groups_sets_the_grouping_value() {
        let status = property(PropertyType::Status {
            options: vec![Choice {
                id: Uuid::from_u128(5),
                name: "Done".into(),
                hue: None,
                group: Some(neon_domain::databases::StatusGroup::Done),
            }],
        });
        assert!(matches!(
            group_value(&status, Some(Uuid::from_u128(5)), None),
            Ok(Some(Value::Choice(id))) if id == Uuid::from_u128(5)
        ));
        assert!(matches!(group_value(&status, None, None), Ok(None)));
        assert!(group_value(&status, Some(Uuid::from_u128(6)), None).is_err());

        let person = property(PropertyType::Person);
        let (ada, bea, cy) = (Uuid::from_u128(7), Uuid::from_u128(8), Uuid::from_u128(9));
        let both = Some(Value::People(vec![ada, bea]));
        assert_eq!(
            group_value(&person, Some(cy), Some((Some(ada), both.clone()))).ok(),
            Some(Some(Value::People(vec![bea, cy])))
        );
        assert_eq!(
            group_value(
                &person,
                None,
                Some((Some(bea), Some(Value::People(vec![bea]))))
            )
            .ok(),
            Some(None)
        );
        assert_eq!(
            group_value(&person, Some(ada), Some((None, both.clone()))).ok(),
            Some(both)
        );
        assert_eq!(
            group_value(&person, Some(ada), None).ok(),
            Some(Some(Value::People(vec![ada])))
        );
    }

    #[test]
    fn values_are_parsed_by_property_id_or_name() {
        let schema = Schema {
            properties: vec![
                Property {
                    id: Uuid::from_u128(1),
                    name: "Name".into(),
                    ty: PropertyType::Title,
                },
                Property {
                    id: Uuid::from_u128(2),
                    name: "Notes".into(),
                    ty: PropertyType::Text,
                },
                Property {
                    id: Uuid::from_u128(3),
                    name: "Owner".into(),
                    ty: PropertyType::Person,
                },
            ],
        };
        let values = |json: Json| json.as_object().cloned().expect("an object");
        let changes = parse_values(
            &schema,
            &values(serde_json::json!({
                "name": "Fix login",
                Uuid::from_u128(2).to_string(): null,
                "Owner": [Uuid::from_u128(7).to_string()],
            })),
        )
        .expect("valid");
        assert_eq!(changes.title.as_deref(), Some("Fix login"));
        assert_eq!(changes.remove, [Uuid::from_u128(2).to_string()]);
        assert_eq!(changes.people, [Uuid::from_u128(7)]);
        assert_eq!(changes.properties.len(), 3);
        for bad in [
            serde_json::json!({"Nope": 1}),
            serde_json::json!({"Notes": 3}),
            serde_json::json!({"Notes": "a", "notes": "b"}),
        ] {
            assert!(parse_values(&schema, &values(bad)).is_err());
        }
    }
}
