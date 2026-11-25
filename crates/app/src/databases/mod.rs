//! Databases: making them, their properties and views, their rows and querying them, and
//! the live feed of their changes.
//!
//! A database is a page of kind database; its rows are its subpages, of kind row, which the
//! page service otherwise treats as any page: shares, the trash, favorites and exports work
//! on them, and their block bodies are edited live like any page's. What the rules let
//! someone do with a database is what they may do with its page: seeing it shows its
//! schema, views and rows; editing it changes its schema and views and adds rows. A row
//! takes its access from its database, as any subpage does, and changing a row's values or
//! moving it within a group is editing the row; placing it among the rows by hand is
//! editing both the row and the database.
//!
//! Filters, sorts and groups are worked out by the domain rules over every row of the
//! database the caller may see, at most [`MAX_ROWS`](neon_domain::databases::MAX_ROWS), read
//! at once with the values they name, off the async workers; the rows of the part answered
//! with are then read whole, each at most
//! [`MAX_ROW_BYTES`](neon_domain::databases::MAX_ROW_BYTES). Values are plain data: each
//! change sets the properties it names, and of two changes to one property, the last wins.
//! A change to a row's values holds its database's schema and locks the row while it is
//! checked and written, so changes to the schema wait for it, and it for them. Once a change
//! commits it goes to everyone following the database, through the
//! [`Broadcaster`](crate::sync::Broadcaster): see [`LiveDatabases`].
//!
//! Every public use case runs in a span named `databases <operation>` and is measured as
//! RED metrics of [`SUBSYSTEM`], counted as the page use cases are: a refusal is `ok`. No
//! log line holds a property's value: ids and counts only.

mod error;
pub(crate) mod events;
mod export;
mod live;
mod rows;
mod schema;
mod tasks;
mod views;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use neon_domain::auth::Principal;
use neon_domain::databases::{
    ChoiceSpec, GroupKey, NumberFormat, Property, PropertyKind, PropertyType, Row, Schema, Value,
    View, ViewSpec, unique_id,
};
use neon_domain::pages::{Page, PageAccess, PageKind, Placement};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::databases::{self as repo, Database, StoredView};
use serde_json::{Map, Value as Json, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::{Instrument as _, info_span};
use uuid::Uuid;

use crate::pages::{Context, PageError, PageService, PageView, key_among, renumbered};

pub use error::DatabaseError;
pub use export::{ExportRow, ExportTable};
pub use live::{Admission, Follow, LiveDatabases, LiveDatabasesBuilder, LiveLimits};
pub use rows::{GroupMove, NewRow, QueryInput, QueryResult, RowMove};
pub use schema::{NewDatabase, NewProperty, PropertyChange, PropertySpec};
pub use tasks::{Holder, MAX_TASKS, Task, TaskStatus};
pub use views::{NewView, ViewChange};

/// RED subsystem of the database use cases.
pub const SUBSYSTEM: &str = "databases";

pub(crate) const NO_DATABASE: &str = "no such database";
pub(crate) const NO_PROPERTY: &str = "no such property";
pub(crate) const NO_VIEW: &str = "no such view";
pub(crate) const NO_ROW: &str = "no such row";

/// A database as the caller sees it: its page, its properties and its views.
#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseView {
    pub page: PageView,
    /// Embedded in the page it is under.
    pub inline: bool,
    /// In their order.
    pub properties: Vec<Property>,
    /// In their order, each with its settings as they fit the schema now.
    pub views: Vec<View>,
}

/// A row as the caller sees it, with its values. For a guest, who knows no other member,
/// the people its properties name and who made and changed it are left out.
#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub id: Uuid,
    /// Its database; `None` for someone who sees the row only through its own share, as
    /// its page leaves out the page above it then.
    pub database_id: Option<Uuid>,
    /// Its number in the database.
    pub number: i64,
    /// Its number after the unique id property's prefix: `NW-42`.
    pub unique_id: String,
    pub title: String,
    pub icon: Option<String>,
    /// Its key among the database's rows: the manual order.
    pub position: String,
    /// Each property's value, by property id, for the properties it has one of.
    pub values: Map<String, Json>,
    pub created_at: OffsetDateTime,
    pub created_by: Option<Uuid>,
    pub last_edited_at: OffsetDateTime,
    pub last_edited_by: Option<Uuid>,
    /// Set while it is in the trash.
    pub trashed_at: Option<OffsetDateTime>,
    pub access: PageAccess,
    /// In a grouped query, the group it is listed in.
    pub group: Option<GroupKey>,
}

/// The database use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct DatabaseService {
    inner: Arc<Inner>,
}

struct Inner {
    pages: PageService,
    red: Red,
}

/// Puts a [`DatabaseService`] together. Its changes go to the broadcaster of `pages`.
pub struct DatabaseServiceBuilder {
    pages: PageService,
}

impl DatabaseServiceBuilder {
    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> DatabaseService {
        DatabaseService {
            inner: Arc::new(Inner {
                pages: self.pages,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

/// A database opened for a use case: its page and what the rules need to know of it, its
/// own data, and its schema with each property's position.
pub(crate) struct Opened {
    pub(crate) context: Context,
    pub(crate) database: Database,
    pub(crate) schema: Schema,
    pub(crate) positions: Vec<(Uuid, String)>,
}

impl Opened {
    /// The prefix of its unique ids, if it has a unique id property.
    pub(crate) fn prefix(&self) -> Option<&str> {
        self.schema
            .unique_id()
            .and_then(|property| property.ty.prefix())
    }
}

impl DatabaseService {
    /// The use cases of the databases among `pages`.
    pub fn builder(pages: PageService) -> DatabaseServiceBuilder {
        DatabaseServiceBuilder { pages }
    }

    pub(crate) fn pages(&self) -> &PageService {
        &self.inner.pages
    }

    fn now(&self) -> OffsetDateTime {
        self.inner.pages.now()
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, DatabaseError>>,
    ) -> Result<T, DatabaseError> {
        let span = info_span!(
            "databases",
            otel.name = %format!("databases {operation}"),
            databases.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }

    /// Reads a database for the principal: its page's context, which the caller asks the
    /// rules about, its data and schema. One that does not exist, and a page that is not a
    /// database, are not found. `lock` holds the database until the transaction ends, so
    /// changes to its schema and views run one at a time.
    pub(crate) async fn open(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        id: Uuid,
        lock: bool,
    ) -> Result<Opened, DatabaseError> {
        let context = self
            .inner
            .pages
            .context(&mut *conn, principal, id)
            .await
            .map_err(not_found)?;
        if context.page().kind != PageKind::Database {
            return Err(DatabaseError::NotFound(NO_DATABASE));
        }
        let database = if lock {
            repo::lock(&mut *conn, id).await?
        } else {
            repo::get(&mut *conn, id).await?
        }
        .ok_or_else(|| DatabaseError::Internal(format!("database page {id} has no database")))?;
        let stored = repo::properties(&mut *conn, id).await?;
        let positions = stored
            .iter()
            .map(|stored| (stored.property.id, stored.position.clone()))
            .collect();
        let schema = Schema {
            properties: stored.into_iter().map(|stored| stored.property).collect(),
        };
        Ok(Opened {
            context,
            database,
            schema,
            positions,
        })
    }

    /// Asks the rules about an action on the database, a page someone may not see being
    /// not found as no database.
    pub(crate) fn check(
        context: &Context,
        action: neon_domain::workspaces::Action,
    ) -> Result<(), DatabaseError> {
        context.check(action).map_err(not_found)
    }
}

/// A page that is not found, as a database that is not.
pub(crate) fn not_found(error: PageError) -> DatabaseError {
    match error {
        PageError::NotFound(_) => DatabaseError::NotFound(NO_DATABASE),
        other => DatabaseError::Page(other),
    }
}

/// A database's views, each with its settings fitted to the schema as it is now.
pub(crate) fn fitted_views(schema: &Schema, stored: Vec<StoredView>) -> Vec<View> {
    stored
        .into_iter()
        .map(|view| fitted_view(schema, view))
        .collect()
}

/// A database's views as the caller reads them: fitted to the schema as it is now, and for
/// a guest, who knows no other member, without the people their settings name.
pub(crate) fn shown_views(
    schema: &Schema,
    stored: Vec<StoredView>,
    members_hidden: bool,
) -> Vec<View> {
    stored
        .into_iter()
        .map(|view| shown_view(schema, view, members_hidden))
        .collect()
}

/// A view as the caller reads it: see [`shown_views`].
pub(crate) fn shown_view(schema: &Schema, view: StoredView, members_hidden: bool) -> View {
    let view = fitted_view(schema, view);
    if members_hidden {
        view.without_people(schema)
    } else {
        view
    }
}

/// A view with its settings fitted to the schema as it is now.
pub(crate) fn fitted_view(schema: &Schema, view: StoredView) -> View {
    // Settings that do not read, which the server never writes, start afresh.
    let settings = serde_json::from_value::<ViewSpec>(view.config)
        .unwrap_or_default()
        .fit(schema, view.layout);
    View {
        id: view.id,
        database_id: view.database_id,
        position: view.position,
        name: view.name,
        layout: view.layout,
        settings,
    }
}

/// The key for an entry placed by `placement` among `list`, ids and keys in order, and
/// when the list wanted fresh keys first, those fresh keys, to save.
pub(crate) fn key_for(
    list: &[(Uuid, String)],
    placement: Placement,
    missing: &'static str,
) -> Result<(String, Vec<(Uuid, String)>), DatabaseError> {
    if let Some(key) = key_among(list, placement, missing)? {
        return Ok((key, Vec::new()));
    }
    let fresh = renumbered(list);
    let key = key_among(&fresh, placement, missing)?
        .ok_or_else(|| DatabaseError::Internal("fresh positions are too long".to_owned()))?;
    Ok((key, fresh))
}

/// A row of a database as the rules read it, from its page and data.
pub(crate) fn domain_row(schema: &Schema, page: &Page, number: i64, cells: &Json) -> Row {
    let cells = schema
        .properties
        .iter()
        .filter(|property| property.kind().is_stored())
        .filter_map(|property| {
            let value = Value::stored(&property.ty, cells.get(property.id.to_string())?)?;
            Some((property.id, value))
        })
        .collect::<HashMap<_, _>>();
    Row {
        id: page.id,
        position: page.position.clone(),
        number,
        title: page.title.clone(),
        created_at: page.created_at,
        created_by: page.created_by,
        last_edited_at: page.last_edited_at(),
        last_edited_by: page.last_edited_by,
        cells,
    }
}

/// A property's value as the API sends it, from a row's: a unique id as its text, `NW-42`,
/// and who made or changed the row as their id.
pub(crate) fn value_json(property: &Property, value: &Value, prefix: Option<&str>) -> Json {
    match (property.kind(), value) {
        (PropertyKind::UniqueId, Value::Serial(number)) => json!(unique_id(prefix, *number)),
        (PropertyKind::CreatedBy | PropertyKind::LastEditedBy, Value::People(ids)) => {
            ids.first().map_or(Json::Null, |id| json!(id.to_string()))
        }
        _ => value.to_json(),
    }
}

/// Each property's value of a row, by property id, for those it has one of.
pub(crate) fn values_json(schema: &Schema, row: &Row, prefix: Option<&str>) -> Map<String, Json> {
    schema
        .properties
        .iter()
        .filter_map(|property| {
            let value = row.value(property)?;
            Some((
                property.id.to_string(),
                value_json(property, &value, prefix),
            ))
        })
        .collect()
}

/// The keys of the cells that hold the values of `properties`, those of them the schema
/// keeps in cells, each once.
pub(crate) fn cell_keys(schema: &Schema, properties: &[Uuid]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for id in properties {
        let stored = schema
            .get(*id)
            .is_some_and(|property| property.kind().is_stored());
        let key = id.to_string();
        if stored && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// Whether a property of the schema names people: whose names a sort or grouping by it
/// reads.
pub(crate) fn names_people(schema: &Schema, id: Uuid) -> bool {
    schema
        .get(id)
        .is_some_and(|property| property.kind().names_people())
}

/// What a row is made of: its page, its number and its cells.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StoredRow<'a> {
    pub(crate) page: &'a Page,
    pub(crate) number: i64,
    pub(crate) cells: &'a Json,
}

/// A row as the caller sees it. `members_hidden` leaves out the people its values name.
pub(crate) fn row_view(
    schema: &Schema,
    database_id: Option<Uuid>,
    stored: StoredRow<'_>,
    access: PageAccess,
    members_hidden: bool,
) -> RowView {
    let StoredRow {
        page,
        number,
        cells,
    } = stored;
    let prefix = schema.unique_id().and_then(|property| property.ty.prefix());
    let mut row = domain_row(schema, page, number, cells);
    if members_hidden {
        row = row.without_people(schema);
    }
    RowView {
        id: page.id,
        database_id,
        number,
        unique_id: unique_id(prefix, number),
        title: page.title.clone(),
        icon: page.icon.clone(),
        position: page.position.clone(),
        values: values_json(schema, &row, prefix),
        created_at: page.created_at,
        created_by: row.created_by,
        last_edited_at: page.last_edited_at(),
        last_edited_by: row.last_edited_by,
        trashed_at: page.trashed_at,
        access,
        group: None,
    }
}

/// A new property's type from what a request asks: the kind with its settings, and options
/// for a kind that has some, checked; a status without options gets the default ones.
pub(crate) fn new_type(
    spec: &PropertySpec,
    options: Option<Vec<ChoiceSpec>>,
) -> Result<PropertyType, DatabaseError> {
    let mut ty = settings_type(spec)?;
    let ids = &mut Uuid::now_v7;
    match (ty.options_mut(), options) {
        (Some(current), Some(specs)) => {
            *current = neon_domain::databases::check_options(spec.kind, &[], specs, ids)?;
        }
        (Some(current), None) if spec.kind == PropertyKind::Status => {
            *current = neon_domain::databases::default_status_options(ids);
        }
        (None, Some(_)) => {
            return Err(DatabaseError::InvalidInput(format!(
                "a {} property has no options",
                spec.kind
            )));
        }
        _ => {}
    }
    Ok(ty)
}

/// A type of the spec's kind with its settings, its options empty, checked.
pub(crate) fn settings_type(spec: &PropertySpec) -> Result<PropertyType, DatabaseError> {
    let refuse = |what: &str| {
        Err(DatabaseError::InvalidInput(format!(
            "only {what}: a {} property has none",
            spec.kind
        )))
    };
    if spec.kind != PropertyKind::Number
        && (spec.number_format.is_some() || spec.currency.is_some())
    {
        return refuse("number properties have a format and a currency");
    }
    if spec.kind != PropertyKind::UniqueId && spec.prefix.is_some() {
        return refuse("unique id properties have a prefix");
    }
    let ty = match spec.kind {
        PropertyKind::Number => PropertyType::Number {
            format: spec.number_format.unwrap_or(NumberFormat::Plain),
            currency: spec.currency.clone(),
        },
        PropertyKind::UniqueId => PropertyType::UniqueId {
            prefix: spec.prefix.clone(),
        },
        kind => PropertyType::of(kind),
    };
    Ok(ty.checked()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: PropertyKind) -> PropertySpec {
        PropertySpec {
            kind,
            number_format: None,
            currency: None,
            prefix: None,
        }
    }

    #[test]
    fn settings_belong_to_their_kind() {
        assert_eq!(
            settings_type(&PropertySpec {
                number_format: Some(NumberFormat::Currency),
                currency: Some("usd".into()),
                ..spec(PropertyKind::Number)
            })
            .ok(),
            Some(PropertyType::Number {
                format: NumberFormat::Currency,
                currency: Some("USD".into())
            })
        );
        assert!(
            settings_type(&PropertySpec {
                number_format: Some(NumberFormat::Percent),
                ..spec(PropertyKind::Text)
            })
            .is_err()
        );
        assert!(
            settings_type(&PropertySpec {
                prefix: Some("NW".into()),
                ..spec(PropertyKind::Number)
            })
            .is_err()
        );
        assert_eq!(
            settings_type(&PropertySpec {
                prefix: Some("nw".into()),
                ..spec(PropertyKind::UniqueId)
            })
            .ok(),
            Some(PropertyType::UniqueId {
                prefix: Some("NW".into())
            })
        );
    }

    #[test]
    fn new_statuses_get_the_default_options_and_others_none() {
        let status = new_type(&spec(PropertyKind::Status), None).expect("valid");
        assert_eq!(status.options().len(), 3);
        let select = new_type(&spec(PropertyKind::Select), None).expect("valid");
        assert!(select.options().is_empty());
        let named = new_type(
            &spec(PropertyKind::MultiSelect),
            Some(vec![ChoiceSpec {
                id: None,
                name: "UI".into(),
                hue: Some(185),
                group: None,
            }]),
        )
        .expect("valid");
        assert_eq!(named.options()[0].name, "UI");
        assert!(new_type(&spec(PropertyKind::Text), Some(vec![])).is_err());
    }

    #[test]
    fn keys_are_found_for_any_placement() {
        let list = vec![
            (Uuid::from_u128(1), "a0".to_owned()),
            (Uuid::from_u128(2), "a1".to_owned()),
        ];
        assert_eq!(
            key_for(&list, Placement::Last, "missing").ok(),
            Some(("a2".to_owned(), vec![]))
        );
        assert!(matches!(
            key_for(&list, Placement::After(Uuid::from_u128(9)), "missing"),
            Err(DatabaseError::Page(PageError::InvalidInput(_)))
        ));
    }
}
