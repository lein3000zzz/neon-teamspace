//! Databases: a database page's own data, its properties and views, and its rows' numbers
//! and cells. A database and its rows are pages too, which [`pages`](crate::pages) keeps;
//! these queries keep the rest.
//!
//! Cells are a JSON object on the row, a value by property id, in the shape the domain
//! rules write. A change sets or removes single keys of it, so changes to different
//! properties never undo one another.

use neon_domain::databases::{Layout, Property, PropertyType};
use neon_domain::pages::{Page, PageAccess};
use serde_json::Value as Json;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::decode_error;
use crate::pages::parse_access;

/// A database page's own data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Database {
    pub page_id: Uuid,
    pub workspace_id: Uuid,
    /// Embedded in the page it is under.
    pub inline: bool,
    /// The number the next row gets.
    pub next_number: i64,
}

pub async fn insert(executor: impl PgExecutor<'_>, database: &Database) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO databases (page_id, workspace_id, inline, next_number)
         VALUES ($1, $2, $3, $4)",
        database.page_id,
        database.workspace_id,
        database.inline,
        database.next_number,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<Option<Database>> {
    sqlx::query_as!(
        Database,
        "SELECT page_id, workspace_id, inline, next_number FROM databases WHERE page_id = $1",
        page_id,
    )
    .fetch_optional(executor)
    .await
}

/// Locks a database until the transaction ends, so changes to its schema and views run one
/// at a time and see each other's.
pub async fn lock(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<Option<Database>> {
    sqlx::query_as!(
        Database,
        "SELECT page_id, workspace_id, inline, next_number FROM databases
         WHERE page_id = $1 FOR UPDATE",
        page_id,
    )
    .fetch_optional(executor)
    .await
}

pub async fn set_inline(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    inline: bool,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE databases SET inline = $2 WHERE page_id = $1",
        page_id,
        inline,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Takes the next row number of a database, which no other row will get.
pub async fn take_number(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"UPDATE databases SET next_number = next_number + 1 WHERE page_id = $1
           RETURNING next_number - 1 AS "number!""#,
        page_id,
    )
    .fetch_one(executor)
    .await
}

/// A property as kept, with its key among the database's properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProperty {
    pub property: Property,
    pub position: String,
}

fn property(id: Uuid, name: String, config: Json) -> sqlx::Result<Property> {
    let ty: PropertyType = serde_json::from_value(config)
        .map_err(|error| decode_error(format!("property {id} has an unreadable type: {error}")))?;
    Ok(Property { id, name, ty })
}

fn config(ty: &PropertyType) -> sqlx::Result<Json> {
    serde_json::to_value(ty).map_err(|error| sqlx::Error::Encode(Box::new(error)))
}

pub async fn insert_property(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
    property: &Property,
    position: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO database_properties (id, database_id, position, name, type, config)
         VALUES ($1, $2, $3, $4, $5, $6)",
        property.id,
        database_id,
        position,
        property.name,
        property.kind().as_str(),
        config(&property.ty)?,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A database's properties, in order.
pub async fn properties(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
) -> sqlx::Result<Vec<StoredProperty>> {
    sqlx::query!(
        "SELECT id, position, name, config FROM database_properties
         WHERE database_id = $1 ORDER BY position, id",
        database_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(StoredProperty {
            property: property(row.id, row.name, row.config)?,
            position: row.position,
        })
    })
    .collect()
}

/// Saves a property's name and type.
pub async fn update_property(
    executor: impl PgExecutor<'_>,
    property: &Property,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE database_properties SET name = $2, type = $3, config = $4 WHERE id = $1",
        property.id,
        property.name,
        property.kind().as_str(),
        config(&property.ty)?,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete_property(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM database_properties WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Gives each of `ids`, properties, the position at the same index of `positions`.
pub async fn set_property_positions(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    positions: &[String],
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE database_properties p SET position = u.position
         FROM UNNEST($1::uuid[], $2::text[]) AS u (id, position)
         WHERE p.id = u.id",
        ids,
        positions,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// The constraint an insert or rename breaks when another property has the name.
pub const PROPERTY_NAME_TAKEN: &str = "database_properties_name";

/// A view as kept: its settings as the domain rules wrote them.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredView {
    pub id: Uuid,
    pub database_id: Uuid,
    pub position: String,
    pub name: String,
    pub layout: Layout,
    pub config: Json,
}

pub async fn insert_view(executor: impl PgExecutor<'_>, view: &StoredView) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO database_views (id, database_id, position, name, layout, config)
         VALUES ($1, $2, $3, $4, $5, $6)",
        view.id,
        view.database_id,
        view.position,
        view.name,
        view.layout.as_str(),
        view.config,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A database's views, in order.
pub async fn views(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
) -> sqlx::Result<Vec<StoredView>> {
    sqlx::query!(
        "SELECT id, database_id, position, name, layout, config FROM database_views
         WHERE database_id = $1 ORDER BY position, id",
        database_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(StoredView {
            layout: Layout::parse(&row.layout)
                .ok_or_else(|| decode_error(format!("unknown view layout {:?}", row.layout)))?,
            id: row.id,
            database_id: row.database_id,
            position: row.position,
            name: row.name,
            config: row.config,
        })
    })
    .collect()
}

/// Saves a view's name, layout and settings.
pub async fn update_view(executor: impl PgExecutor<'_>, view: &StoredView) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE database_views SET name = $2, layout = $3, config = $4 WHERE id = $1",
        view.id,
        view.name,
        view.layout.as_str(),
        view.config,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete_view(executor: impl PgExecutor<'_>, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM database_views WHERE id = $1", id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Gives each of `ids`, views, the position at the same index of `positions`.
pub async fn set_view_positions(
    executor: impl PgExecutor<'_>,
    ids: &[Uuid],
    positions: &[String],
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE database_views v SET position = u.position
         FROM UNNEST($1::uuid[], $2::text[]) AS u (id, position)
         WHERE v.id = u.id",
        ids,
        positions,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

pub async fn insert_row(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    database_id: Uuid,
    number: i64,
    cells: &Json,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO database_rows (page_id, database_id, number, cells) VALUES ($1, $2, $3, $4)",
        page_id,
        database_id,
        number,
        cells,
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// A row's own data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowData {
    pub page_id: Uuid,
    pub database_id: Uuid,
    pub number: i64,
    pub cells: Json,
}

pub async fn row(executor: impl PgExecutor<'_>, page_id: Uuid) -> sqlx::Result<Option<RowData>> {
    sqlx::query_as!(
        RowData,
        "SELECT page_id, database_id, number, cells FROM database_rows WHERE page_id = $1",
        page_id,
    )
    .fetch_optional(executor)
    .await
}

/// The row of a database with a number, if it is still there.
pub async fn row_by_number(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
    number: i64,
) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!(
        "SELECT page_id FROM database_rows WHERE database_id = $1 AND number = $2",
        database_id,
        number,
    )
    .fetch_optional(executor)
    .await
}

/// A row as a query reads it: its page, its data, and what its page itself is shared with
/// the person querying for.
#[derive(Debug, Clone)]
pub struct ListedRow {
    pub page: Page,
    pub number: i64,
    pub cells: Json,
    pub shared: Option<PageAccess>,
}

/// The rows of a database that are where it is: out of the trash for a database out of
/// it, or for one in it, those that went to the trash with it, `batch` being its
/// `trashed_with`. Each with what its page is shared with `user_id` for.
pub async fn rows(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
    batch: Option<Uuid>,
    user_id: Uuid,
) -> sqlx::Result<Vec<ListedRow>> {
    sqlx::query!(
        r#"SELECT p.id, p.workspace_id, p.kind, p.teamspace_id, p.owner_id, p.parent_id,
                  p.position, p.title, p.icon, p.cover, p.font, p.small_text, p.full_width,
                  p.created_by, p.last_edited_by, p.created_at, p.updated_at,
                  p.content_updated_at, p.trashed_at, p.trashed_by, p.trashed_with,
                  r.number, r.cells, s.access AS "shared?"
           FROM database_rows r
           JOIN pages p ON p.id = r.page_id
           LEFT JOIN page_shares s ON s.page_id = p.id AND s.user_id = $3
           WHERE r.database_id = $1 AND p.trashed_with IS NOT DISTINCT FROM $2
           ORDER BY p.position, p.id"#,
        database_id,
        batch,
        user_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let page = crate::pages::page_of(crate::pages::PageColumns {
            id: row.id,
            workspace_id: row.workspace_id,
            kind: row.kind,
            teamspace_id: row.teamspace_id,
            owner_id: row.owner_id,
            parent_id: row.parent_id,
            position: row.position,
            title: row.title,
            icon: row.icon,
            cover: row.cover,
            font: row.font,
            small_text: row.small_text,
            full_width: row.full_width,
            created_by: row.created_by,
            last_edited_by: row.last_edited_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            content_updated_at: row.content_updated_at,
            trashed_at: row.trashed_at,
            trashed_by: row.trashed_by,
            trashed_with: row.trashed_with,
        })?;
        Ok(ListedRow {
            page,
            number: row.number,
            cells: row.cells,
            shared: row.shared.as_deref().map(parse_access).transpose()?,
        })
    })
    .collect()
}

/// How many rows of a database are out of the trash.
pub async fn count_rows(executor: impl PgExecutor<'_>, database_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM database_rows r JOIN pages p ON p.id = r.page_id
           WHERE r.database_id = $1 AND p.trashed_at IS NULL"#,
        database_id,
    )
    .fetch_one(executor)
    .await
}

/// Sets the cells of `set`, a JSON object of values by property id, and removes those of
/// `remove`, leaving the row's other cells as they are.
pub async fn patch_cells(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    set: &Json,
    remove: &[String],
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE database_rows SET cells = (cells || $2::jsonb) - $3::text[] WHERE page_id = $1",
        page_id,
        set,
        remove,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Every row of a database with a cell of the property, in the trash or not, and the cell.
pub async fn cells_of(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
    property_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, Json)>> {
    let key = property_id.to_string();
    Ok(sqlx::query!(
        r#"SELECT page_id, cells -> $2 AS "cell!" FROM database_rows
           WHERE database_id = $1 AND cells ? $2"#,
        database_id,
        key,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.page_id, row.cell))
    .collect())
}

/// Sets the cell of the property on each of `rows` to the value at the same index of
/// `values`, or removes it where that is JSON `null`.
pub async fn set_cells_of(
    executor: impl PgExecutor<'_>,
    property_id: Uuid,
    rows: &[Uuid],
    values: &[Json],
) -> sqlx::Result<u64> {
    let key = property_id.to_string();
    let result = sqlx::query!(
        "UPDATE database_rows r
         SET cells = CASE WHEN u.value = 'null'::jsonb THEN r.cells - $1
                          ELSE r.cells || jsonb_build_object($1::text, u.value) END
         FROM UNNEST($2::uuid[], $3::jsonb[]) AS u (id, value)
         WHERE r.page_id = u.id",
        key,
        rows,
        values,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Removes the cells of a property from every row of a database.
pub async fn remove_cells_of(
    executor: impl PgExecutor<'_>,
    database_id: Uuid,
    property_id: Uuid,
) -> sqlx::Result<u64> {
    let key = property_id.to_string();
    let result = sqlx::query!(
        "UPDATE database_rows SET cells = cells - $2 WHERE database_id = $1 AND cells ? $2",
        database_id,
        key,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Which of `ids` are members of the workspace.
pub async fn members_among(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
    ids: &[Uuid],
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT user_id FROM workspace_members WHERE workspace_id = $1 AND user_id = ANY($2)",
        workspace_id,
        ids,
    )
    .fetch_all(executor)
    .await
}

/// The display names of a workspace's members, by id.
pub async fn member_names(
    executor: impl PgExecutor<'_>,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<(Uuid, String)>> {
    Ok(sqlx::query!(
        "SELECT u.id, u.display_name FROM workspace_members m JOIN users u ON u.id = m.user_id
         WHERE m.workspace_id = $1",
        workspace_id,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| (row.id, row.display_name))
    .collect())
}

/// Records a change to a row's page that is not its title, icon, cover or style: its
/// values, or its place among the rows.
pub async fn touch_row(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    by: Uuid,
    at: OffsetDateTime,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE pages SET last_edited_by = $2, updated_at = $3 WHERE id = $1",
        page_id,
        by,
        at,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Moves a row among its database's rows.
pub async fn set_row_position(
    executor: impl PgExecutor<'_>,
    page_id: Uuid,
    position: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE pages SET position = $2 WHERE id = $1",
        page_id,
        position,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}
