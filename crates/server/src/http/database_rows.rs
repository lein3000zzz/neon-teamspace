//! A database's rows: querying them, adding them, reading and changing their values,
//! moving them, and finding one by its unique id. A row is a page too: its title, icon,
//! body, sharing and trash are the pages' routes'.
//!
//! Every route needs a browser session or an API token: `read` for `GET` and for the query,
//! a `POST` only because its filter is too large for a query string, and `write` for the
//! rest.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::databases::{GroupMove, NewRow, QueryInput, QueryResult, RowMove, RowView};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::databases::{Filter, Sort};
use super::error::{ApiError, ErrorBody};
use super::extract::{Json as Body, Path};
use super::pages::{PageAccess, Placement};
use super::principal::{Authenticated, Reader};

/// A row of a database, with its values.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Row {
    /// Its page's id.
    pub id: Uuid,
    pub database_id: Uuid,
    /// Its number in the database, which no other row of it has had or will have.
    pub number: i64,
    /// Its number after the unique id property's prefix, `NW-42`, or alone without one.
    #[schema(example = "NW-42")]
    pub unique_id: String,
    pub title: String,
    pub icon: Option<String>,
    /// Its key among the rows: their manual order.
    #[schema(example = "a0")]
    pub position: String,
    /// Each property's value, by property id, for those it has one of. A title, text or
    /// URL is a string; a number a number; a select or status an option's id; a
    /// multi-select a list of option ids; a date `{"start": ..., "end": ...}`, each a day
    /// `2026-11-22` or an RFC 3339 moment, the end optional; a person a list of members'
    /// ids; a checkbox `true`; a unique id its text, `NW-42`; a created or last edited time
    /// an RFC 3339 moment; created or last edited by a member's id. Guests, who know no
    /// other member, get no person values, nor who made or changed a row.
    #[schema(value_type = Object)]
    pub values: Map<String, Json>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Absent for guests, as is `last_edited_by`.
    pub created_by: Option<Uuid>,
    /// When its title, values, place or content last changed.
    #[serde(with = "time::serde::rfc3339")]
    pub last_edited_at: OffsetDateTime,
    pub last_edited_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,
    pub access: PageAccess,
    /// In a grouped query, the group it is listed in: an option's id, a person's id, or
    /// null for the rows without a value. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Uuid>)]
    pub group: Option<Option<Uuid>>,
}

impl From<RowView> for Row {
    fn from(row: RowView) -> Self {
        Self {
            id: row.id,
            database_id: row.database_id,
            number: row.number,
            unique_id: row.unique_id,
            title: row.title,
            icon: row.icon,
            position: row.position,
            values: row.values,
            created_at: row.created_at,
            created_by: row.created_by,
            last_edited_at: row.last_edited_at,
            last_edited_by: row.last_edited_by,
            trashed_at: row.trashed_at,
            access: row.access.into(),
            group: row.group,
        }
    }
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct QueryRows {
    /// The view whose filter, sorts and grouping to use; every row, in manual order, when
    /// absent.
    #[serde(default)]
    pub view_id: Option<Uuid>,
    /// In place of the view's filter.
    #[serde(default)]
    pub filter: Option<Filter>,
    /// In place of the view's sorts.
    #[serde(default)]
    pub sorts: Option<Vec<Sort>>,
    /// Rows in the part, 1 to 1000; 100 when absent.
    #[serde(default)]
    pub limit: Option<usize>,
    /// The `next` of the part before.
    #[serde(default)]
    pub after: Option<String>,
    /// The caller's UTC offset in minutes, -840 to 840, which days are told at: `today`,
    /// and the day a moment falls on. 0 when absent.
    #[serde(default)]
    pub utc_offset_minutes: Option<i16>,
}

/// A group of a grouped query.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QueryGroup {
    /// An option's id, a person's id, or null for the rows without a value.
    pub key: Option<Uuid>,
    /// How many rows that passed the filter it holds.
    pub count: usize,
    /// The view hides its rows, which the answer leaves out.
    pub hidden: bool,
}

/// A part of a query's answer.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QueryAnswer {
    /// In order; grouped, in the order of the groups, a row once in each group it is in.
    pub rows: Vec<Row>,
    /// For a view that groups: every group, in its order, hidden ones too.
    pub groups: Option<Vec<QueryGroup>>,
    /// How many entries the whole answer holds.
    pub total: usize,
    /// Pass it as `after` for the next part; absent on the last one.
    pub next: Option<String>,
}

impl From<QueryResult> for QueryAnswer {
    fn from(result: QueryResult) -> Self {
        Self {
            rows: result.rows.into_iter().map(Row::from).collect(),
            groups: result.groups.map(|groups| {
                groups
                    .into_iter()
                    .map(|group| QueryGroup {
                        key: group.key,
                        count: group.count,
                        hidden: group.hidden,
                    })
                    .collect()
            }),
            total: result.total,
            next: result.next,
        }
    }
}

#[utoipa::path(
    post,
    path = "/databases/{id}/query",
    tag = "databases",
    summary = "Query rows",
    description = "The rows of a database the caller sees, out of the trash, as a view has them \
                   or with a filter and sorts of the caller's, a part at a time. Grouped by its \
                   view, they come group by group, a row naming two people once in each \
                   person's group, with the groups and their counts. The databases reference \
                   lists the operators each type takes. Needs only the `read` scope.",
    params(("id" = Uuid, Path, description = "The database's id.")),
    request_body = QueryRows,
    responses(
        (status = OK, description = "The rows, or a part of them.", body = QueryAnswer),
        (status = BAD_REQUEST, description = "invalid_input: a filter or sort naming what the database lacks or a type does not take, past its bounds, or a limit, cursor or offset out of bounds.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the database or the view.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn query_rows(
    State(state): State<AppState>,
    Reader(principal): Reader,
    Path(id): Path<Uuid>,
    Body(body): Body<QueryRows>,
) -> Result<Body<QueryAnswer>, ApiError> {
    let input = QueryInput {
        view_id: body.view_id,
        filter: body.filter.map(Into::into),
        sorts: body
            .sorts
            .map(|sorts| sorts.into_iter().map(Into::into).collect()),
        limit: body.limit,
        after: body.after,
        utc_offset_minutes: body.utc_offset_minutes,
    };
    Ok(Body(
        state.databases.query(&principal, id, input).await?.into(),
    ))
}

/// A group of a view.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InGroup {
    pub view_id: Uuid,
    /// An option's id, a person's id, or null for the group of rows without a value.
    #[serde(default)]
    pub key: Option<Uuid>,
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct CreateRow {
    /// Values by property id or name, as rows give them; the title property's is the
    /// row's title. Options may be named by name too.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub values: Map<String, Json>,
    /// Among the rows; `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
    /// The group of a view the row goes in, as a card added to a board's column.
    #[serde(default)]
    pub group: Option<InGroup>,
}

#[utoipa::path(
    post,
    path = "/databases/{id}/rows",
    tag = "databases",
    summary = "Add a row",
    description = "A page of kind `row` under the database, for those who edit it, with its \
                   values, the database's next number, and an empty body. A database holds at \
                   most 10,000 rows out of the trash.",
    params(("id" = Uuid, Path, description = "The database's id.")),
    request_body = CreateRow,
    responses(
        (status = CREATED, description = "The row.", body = Row),
        (status = BAD_REQUEST, description = "invalid_input: a value its property does not take, a property the database lacks or computes, a person who is not a member, a view that does not group, or a placement beside a page that is not a row of it.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the database and does not edit it, it is in the trash, or a guest names people; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the database or the view.", body = ErrorBody),
        (status = CONFLICT, description = "too_many_rows.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_row(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<CreateRow>,
) -> Result<(StatusCode, Body<Row>), ApiError> {
    let new = NewRow {
        values: body.values,
        placement: body.placement.map(Into::into).unwrap_or_default(),
        group: body.group.map(|group| (group.view_id, group.key)),
    };
    let row = state.databases.create_row(&principal, id, new).await?;
    Ok((StatusCode::CREATED, Body(row.into())))
}

#[utoipa::path(
    get,
    path = "/databases/{id}/rows/{unique_id}",
    tag = "databases",
    summary = "A row by its unique id",
    description = "The row whose unique id is `NW-42`, its prefix in any case, or `42` alone: how \
                   people and agents name a task. One in the trash shows only to those who could \
                   edit it.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("unique_id" = String, Path, description = "The row's unique id, such as NW-42."),
    ),
    responses(
        (status = OK, description = "The row.", body = Row),
        (status = BAD_REQUEST, description = "invalid_input: not a unique id of this database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the database, or no row of it has the id.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn row_by_unique_id(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, unique_id)): Path<(Uuid, String)>,
) -> Result<Body<Row>, ApiError> {
    let row = state
        .databases
        .row_by_unique_id(&principal, id, &unique_id)
        .await?;
    Ok(Body(row.into()))
}

#[utoipa::path(
    get,
    path = "/rows/{id}",
    tag = "databases",
    summary = "A row",
    description = "Its values, for those who see it. One in the trash shows only to those who \
                   could edit it.",
    params(("id" = Uuid, Path, description = "The row's id: its page's.")),
    responses(
        (status = OK, description = "The row.", body = Row),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_row(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Body<Row>, ApiError> {
    Ok(Body(state.databases.row(&principal, id).await?.into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeRow {
    /// The values to set, by property id or name; null clears one. The others stay.
    #[schema(value_type = Object)]
    pub values: Map<String, Json>,
}

#[utoipa::path(
    patch,
    path = "/rows/{id}",
    tag = "databases",
    summary = "Change a row's values",
    description = "Sets the values the body names and leaves the others, for those who edit the \
                   row: of two changes to one property, the last wins, and changes to different \
                   properties both hold. The title property's value is the row's title.",
    params(("id" = Uuid, Path, description = "The row's id.")),
    request_body = ChangeRow,
    responses(
        (status = OK, description = "The row.", body = Row),
        (status = BAD_REQUEST, description = "invalid_input: no values, a value its property does not take, a property the database lacks or computes, or a person who is not a member.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views it and does not edit it, it is in the trash, or a guest names people; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_row(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<ChangeRow>,
) -> Result<Body<Row>, ApiError> {
    let row = state
        .databases
        .update_row(&principal, id, body.values)
        .await?;
    Ok(Body(row.into()))
}

/// A row moving between groups of a view.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ToGroup {
    pub view_id: Uuid,
    /// The group it leaves: for a person property, whose person makes room for the one of
    /// `to`, the others staying. Null or absent for the group without a value.
    #[serde(default)]
    pub from: Option<Uuid>,
    /// The group it goes to: an option's id, a person's id, or null for no value.
    #[serde(default)]
    pub to: Option<Uuid>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveRow {
    /// Among the rows, beside another row: their manual order, which views without sorts
    /// keep.
    #[serde(default)]
    pub placement: Option<Placement>,
    /// Into another group of a view, which sets the row's value of the view's grouping
    /// property.
    #[serde(default)]
    pub group: Option<ToGroup>,
}

#[utoipa::path(
    post,
    path = "/rows/{id}/move",
    tag = "databases",
    summary = "Move a row",
    description = "Elsewhere among the database's rows, into another group of a view, or both at \
                   once, as a card dragged across a board. A group needs edit access to the row; \
                   a placement, to the database too.",
    params(("id" = Uuid, Path, description = "The row's id.")),
    request_body = MoveRow,
    responses(
        (status = OK, description = "The row.", body = Row),
        (status = BAD_REQUEST, description = "invalid_input: neither a placement nor a group, a view that does not group, a group that is no option or member, or a placement beside a page that is not a row of the database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the row, its database or the view.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn move_row(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<MoveRow>,
) -> Result<Body<Row>, ApiError> {
    let to = RowMove {
        placement: body.placement.map(Into::into),
        group: body.group.map(|group| GroupMove {
            view_id: group.view_id,
            from: group.from,
            to: group.to,
        }),
    };
    Ok(Body(
        state.databases.move_row(&principal, id, to).await?.into(),
    ))
}
