//! `/api/v1/databases`: databases, their properties and their views.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. A database is a page: who may see and change it is the pages' rules', and one
//! someone may not see answers `not_found`, as one that does not exist would. Seeing it
//! shows its properties, views and rows; editing it changes them.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::databases::{
    DatabaseView, NewDatabase, NewProperty, NewView, PropertyChange, PropertySpec, ViewChange,
};
use neon_domain::databases as domain;
use neon_domain::databases::filter as filters;
use neon_domain::databases::sort as sorts;
use neon_domain::databases::view as views;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::dto::nullable;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json as Body, Path};
use super::pages::{Page, Placement, location};
use super::principal::Authenticated;

/// What a property holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PropertyKind {
    /// The row's title: its page's. Every database has exactly one.
    Title,
    Text,
    Number,
    Select,
    MultiSelect,
    /// Options in groups: to do, in progress, done.
    Status,
    /// A day or a moment, with an optional end.
    Date,
    /// Members of the workspace.
    Person,
    Checkbox,
    Url,
    /// The row's number after the property's prefix: `NW-42`. Computed.
    UniqueId,
    CreatedTime,
    CreatedBy,
    LastEditedTime,
    LastEditedBy,
}

/// The two `From`s between an enum of the API and the domain's of the same variants.
macro_rules! mirror {
    ($dto:ident, $domain:path, [$($variant:ident),+ $(,)?]) => {
        impl From<$domain> for $dto {
            fn from(value: $domain) -> Self {
                use $domain as Domain;
                match value {
                    $(Domain::$variant => Self::$variant),+
                }
            }
        }

        impl From<$dto> for $domain {
            fn from(value: $dto) -> Self {
                match value {
                    $($dto::$variant => Self::$variant),+
                }
            }
        }
    };
}

mirror!(
    PropertyKind,
    domain::PropertyKind,
    [
        Title,
        Text,
        Number,
        Select,
        MultiSelect,
        Status,
        Date,
        Person,
        Checkbox,
        Url,
        UniqueId,
        CreatedTime,
        CreatedBy,
        LastEditedTime,
        LastEditedBy,
    ]
);

/// How a number shows; values are any finite number whatever the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NumberFormat {
    /// `1234.5`.
    Plain,
    /// Rounded: `1235`.
    Integer,
    /// A hundred times the value, with `%`: `0.25` shows `25%`.
    Percent,
    /// With two decimals and `currency`: `12.50 EUR`.
    Currency,
}

mirror!(
    NumberFormat,
    domain::NumberFormat,
    [Plain, Integer, Percent, Currency]
);

/// The group of a status option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusGroup {
    ToDo,
    InProgress,
    Done,
}

mirror!(StatusGroup, domain::StatusGroup, [ToDo, InProgress, Done]);

/// An option of a select, multi-select or status property.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PropertyOption {
    /// Values hold it by its id, so renaming it keeps them.
    pub id: Uuid,
    #[schema(example = "In progress")]
    pub name: String,
    /// 0 to 359; absent for grey.
    #[schema(minimum = 0, maximum = 359)]
    pub hue: Option<u16>,
    /// A status option's group; absent for the others.
    pub group: Option<StatusGroup>,
}

/// An option as a change lists it: an existing one by its id, a new one without.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionChange {
    #[serde(default)]
    pub id: Option<Uuid>,
    /// 1 to 100 characters, unique in the property, ignoring case.
    pub name: String,
    #[serde(default)]
    #[schema(minimum = 0, maximum = 359)]
    pub hue: Option<u16>,
    /// For a status option, which needs one.
    #[serde(default)]
    pub group: Option<StatusGroup>,
}

impl From<OptionChange> for domain::ChoiceSpec {
    fn from(option: OptionChange) -> Self {
        Self {
            id: option.id,
            name: option.name,
            hue: option.hue,
            group: option.group.map(Into::into),
        }
    }
}

/// A property of a database.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Property {
    pub id: Uuid,
    /// 1 to 100 characters, unique in the database, ignoring case.
    #[schema(example = "Status")]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PropertyKind,
    /// A number's format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_format: Option<NumberFormat>,
    /// The ISO 4217 code of the currency format.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "EUR")]
    pub currency: Option<String>,
    /// A unique id's prefix, before the row's number.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "NW")]
    pub prefix: Option<String>,
    /// A select's, multi-select's or status's options, in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<PropertyOption>>,
}

impl From<domain::Property> for Property {
    fn from(property: domain::Property) -> Self {
        let kind = property.kind();
        let (number_format, currency) = match &property.ty {
            domain::PropertyType::Number { format, currency } => {
                (Some((*format).into()), currency.clone())
            }
            _ => (None, None),
        };
        Self {
            id: property.id,
            kind: kind.into(),
            number_format,
            currency,
            prefix: property.ty.prefix().map(str::to_owned),
            options: kind.has_options().then(|| {
                property
                    .ty
                    .options()
                    .iter()
                    .map(|option| PropertyOption {
                        id: option.id,
                        name: option.name.clone(),
                        hue: option.hue,
                        group: option.group().map(Into::into),
                    })
                    .collect()
            }),
            name: property.name,
        }
    }
}

/// How a view lays its rows out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Table,
    /// Cards in a column per group: a board always groups.
    Board,
    List,
}

mirror!(Layout, domain::Layout, [Table, Board, List]);

/// How a condition compares a row's value with its own; which a property takes depends on
/// its type, as the databases reference lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    Is,
    IsNot,
    Contains,
    DoesNotContain,
    StartsWith,
    EndsWith,
    IsEmpty,
    IsNotEmpty,
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Before,
    After,
    OnOrBefore,
    OnOrAfter,
    Within,
    InGroup,
}

mirror!(
    Operator,
    filters::Operator,
    [
        Is,
        IsNot,
        Contains,
        DoesNotContain,
        StartsWith,
        EndsWith,
        IsEmpty,
        IsNotEmpty,
        Equals,
        NotEquals,
        GreaterThan,
        GreaterThanOrEqual,
        LessThan,
        LessThanOrEqual,
        Before,
        After,
        OnOrBefore,
        OnOrAfter,
        Within,
        InGroup,
    ]
);

/// A filter: a group of filters that must all pass, `{"and": [...]}`, or of which one must,
/// `{"or": [...]}`, or a condition. Groups nest at most three deep, with at most 50
/// conditions in all. A filter that does not read says where and why, naming the fields a
/// condition has and the operators: see [`Filter::read`].
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(untagged)]
pub enum Filter {
    And {
        #[schema(no_recursion)]
        and: Vec<Filter>,
    },
    Or {
        #[schema(no_recursion)]
        or: Vec<Filter>,
    },
    Condition(Condition),
}

/// A condition on one property.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// The property, by id or name.
    #[schema(example = "Status")]
    pub property: String,
    pub operator: Operator,
    /// What to compare with, as the operator and the property's type take it: a text, a
    /// number, an option by id or name, a group, a member's id or `me`, `true` or `false`,
    /// a day such as `2026-11-22` or one relative to today such as `one_week_ago`, or a
    /// range such as `past_week`. Absent for `is_empty` and `is_not_empty`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Json>,
}

impl Filter {
    /// The filter `value` writes, or what is wrong with it and where, as `and[1].operator`.
    fn read(value: Json) -> Result<Self, String> {
        const SHAPE: &str = "a filter is a condition {\"property\", \"operator\", \"value\"}, \
                             or {\"and\": [...]} or {\"or\": [...]} of filters";
        let Json::Object(mut fields) = value else {
            return Err(SHAPE.to_owned());
        };
        for group in ["and", "or"] {
            let Some(members) = fields.remove(group) else {
                continue;
            };
            if let Some(other) = fields.keys().next() {
                return Err(format!(
                    "a group holds \"{group}\" alone, and no field `{other}`"
                ));
            }
            let Json::Array(members) = members else {
                return Err(format!("`{group}` holds a list of filters"));
            };
            let members = members
                .into_iter()
                .enumerate()
                .map(|(index, member)| {
                    Self::read(member).map_err(|why| format!("{group}[{index}]: {why}"))
                })
                .collect::<Result<_, _>>()?;
            return Ok(match group {
                "and" => Self::And { and: members },
                _ => Self::Or { or: members },
            });
        }
        if let Some(other) = fields
            .keys()
            .find(|key| !["property", "operator", "value"].contains(&key.as_str()))
        {
            return Err(format!(
                "unknown field `{other}`: a condition has property, operator and value; {SHAPE}"
            ));
        }
        let operators = || {
            filters::Operator::ALL
                .iter()
                .map(|operator| operator.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let property = match fields.remove("property") {
            Some(Json::String(property)) => property,
            Some(_) => return Err("`property` names a property by its id or name".to_owned()),
            None => {
                return Err(format!(
                    "a condition names its `property`, by id or name, and its `operator`; \
                     {SHAPE}"
                ));
            }
        };
        let operator = match fields.remove("operator") {
            Some(Json::String(operator)) => {
                filters::Operator::parse(&operator).ok_or_else(|| {
                    format!(
                        "unknown operator `{operator}`: the operators are {}",
                        operators()
                    )
                })?
            }
            Some(_) => {
                return Err(format!(
                    "`operator` is one of the operators' names: {}",
                    operators()
                ));
            }
            None => {
                return Err(format!(
                    "a condition names its `operator`, one of {}",
                    operators()
                ));
            }
        };
        Ok(Self::Condition(Condition {
            property,
            operator: operator.into(),
            value: fields.remove("value").filter(|value| !value.is_null()),
        }))
    }
}

impl<'de> Deserialize<'de> for Filter {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::read(Json::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl From<Filter> for filters::FilterSpec {
    fn from(filter: Filter) -> Self {
        match filter {
            Filter::And { and } => Self::And {
                and: and.into_iter().map(Into::into).collect(),
            },
            Filter::Or { or } => Self::Or {
                or: or.into_iter().map(Into::into).collect(),
            },
            Filter::Condition(condition) => Self::Condition {
                property: condition.property,
                operator: condition.operator.into(),
                value: condition.value.unwrap_or(Json::Null),
            },
        }
    }
}

impl From<filters::FilterSpec> for Filter {
    fn from(spec: filters::FilterSpec) -> Self {
        match spec {
            filters::FilterSpec::And { and } => Self::And {
                and: and.into_iter().map(Into::into).collect(),
            },
            filters::FilterSpec::Or { or } => Self::Or {
                or: or.into_iter().map(Into::into).collect(),
            },
            filters::FilterSpec::Condition {
                property,
                operator,
                value,
            } => Self::Condition(Condition {
                property,
                operator: operator.into(),
                value: (!value.is_null()).then_some(value),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Ascending,
    Descending,
}

mirror!(Direction, sorts::Direction, [Ascending, Descending]);

/// A sort by one property. Rows without a value come last either way.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sort {
    /// The property, by id or name.
    #[schema(example = "Due")]
    pub property: String,
    pub direction: Direction,
}

impl From<Sort> for sorts::SortSpec {
    fn from(sort: Sort) -> Self {
        Self {
            property: sort.property,
            direction: sort.direction.into(),
        }
    }
}

/// How a view groups its rows: by a select, status or person property.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Grouping {
    /// The property, by id or name.
    pub property: String,
    /// Groups placed by hand, first; the others follow in their natural order. A group is
    /// an option's id or name, a person's id, or `null` for the rows without a value.
    #[serde(default)]
    pub order: Vec<Option<String>>,
    /// Groups whose rows the view leaves out, likewise.
    #[serde(default)]
    pub hidden: Vec<Option<String>>,
}

impl From<Grouping> for views::GroupingSpec {
    fn from(grouping: Grouping) -> Self {
        Self {
            property: grouping.property,
            order: grouping.order,
            hidden: grouping.hidden,
        }
    }
}

/// A property as a view shows it, in the view's order.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewProperty {
    /// The property, by id or name; answers name it by id.
    pub property: String,
    pub visible: bool,
    /// Its column's width in pixels, 40 to 2000, once resized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
}

impl From<ViewProperty> for views::ViewPropertySpec {
    fn from(shown: ViewProperty) -> Self {
        Self {
            property: shown.property,
            visible: shown.visible,
            width: shown.width,
        }
    }
}

/// A view of a database, its settings naming properties by id.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct View {
    pub id: Uuid,
    pub database_id: Uuid,
    /// Its key among the database's views.
    pub position: String,
    #[schema(example = "Board")]
    pub name: String,
    pub layout: Layout,
    pub filter: Option<Filter>,
    pub sorts: Vec<Sort>,
    pub group: Option<Grouping>,
    /// Every property, in the view's order.
    pub properties: Vec<ViewProperty>,
}

impl From<domain::View> for View {
    fn from(view: domain::View) -> Self {
        let spec = view.settings.to_spec();
        Self {
            id: view.id,
            database_id: view.database_id,
            position: view.position,
            name: view.name,
            layout: view.layout.into(),
            filter: spec.filter.map(Into::into),
            sorts: spec
                .sorts
                .into_iter()
                .map(|sort| Sort {
                    property: sort.property,
                    direction: sort.direction.into(),
                })
                .collect(),
            group: spec.group.map(|group| Grouping {
                property: group.property,
                order: group.order,
                hidden: group.hidden,
            }),
            properties: spec
                .properties
                .into_iter()
                .map(|shown| ViewProperty {
                    property: shown.property,
                    visible: shown.visible,
                    width: shown.width,
                })
                .collect(),
        }
    }
}

/// A database: its page, its properties and its views.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Database {
    pub page: Page,
    /// Embedded in the page it is under, rather than a page of its own there.
    pub inline: bool,
    /// In their order.
    pub properties: Vec<Property>,
    /// In their order.
    pub views: Vec<View>,
}

impl From<DatabaseView> for Database {
    fn from(database: DatabaseView) -> Self {
        Self {
            page: database.page.into(),
            inline: database.inline,
            properties: database.properties.into_iter().map(Into::into).collect(),
            views: database.views.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDatabase {
    /// The page to put it under. Name this, `teamspace_id` or `private`.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub teamspace_id: Option<Uuid>,
    #[serde(default)]
    pub private: bool,
    /// Embedded in the page it goes under, which `parent_id` names: the page's body shows
    /// it where a block of it stands.
    #[serde(default)]
    pub inline: bool,
    /// With `inline`, also adds the block that shows it at the end of the page it goes
    /// under, as an edit's `append` would. The web app's editor adds its own where the
    /// cursor is, and so leaves this out.
    #[serde(default)]
    pub embed: bool,
    /// `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
    #[serde(default)]
    #[schema(example = "Tasks")]
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/databases",
    tag = "databases",
    summary = "Create a database",
    description = "A page of kind `database`, under a page the caller edits, at the top of a \
                   teamspace they are in, or among their private pages; inline, it is embedded \
                   in the page it goes under, which shows it where a block of it stands: \
                   `embed` adds that block at the end of the page, or an edit of the page's \
                   markdown adds `:::database <id>` where it should go. It starts with a `Name` \
                   title property and a `Status` with the options Not started, In progress and \
                   Done, a `Table` view and a `Board` view grouped by status.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreateDatabase,
    responses(
        (status = CREATED, description = "The database.", body = Database),
        (status = BAD_REQUEST, description = "invalid_input: no place or several, an inline database without a parent, embed without inline, a parent that is a database or a row, or a title or icon out of bounds.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller may not add pages there; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "too_deep.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_database(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<CreateDatabase>,
) -> Result<(StatusCode, Body<Database>), ApiError> {
    let location = location(body.parent_id, body.teamspace_id, body.private)?;
    let embed = embedding(body.embed, body.inline, body.parent_id)?;
    let created = state
        .databases
        .create_database(
            &principal,
            id,
            location,
            body.placement.map(Into::into).unwrap_or_default(),
            NewDatabase {
                title: body.title,
                icon: body.icon,
                inline: body.inline,
            },
        )
        .await?;
    if let Some(parent) = embed {
        embed_in(&state.sync, &principal, parent, created.page.page.id).await?;
    }
    Ok((StatusCode::CREATED, Body(created.into())))
}

/// The page to add a block of a new database to, when `embed` asks for one: an inline
/// database's parent alone.
pub(crate) fn embedding(
    embed: bool,
    inline: bool,
    parent_id: Option<Uuid>,
) -> Result<Option<Uuid>, ApiError> {
    match (embed, inline, parent_id) {
        (false, _, _) => Ok(None),
        (true, true, Some(parent)) => Ok(Some(parent)),
        (true, _, _) => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "embed adds an inline database's block to its parent: set inline and parent_id",
        )),
    }
}

/// Adds the block that shows database `id` at the end of page `parent`, through its live
/// document.
pub(crate) async fn embed_in(
    sync: &neon_app::sync::SyncService,
    principal: &neon_domain::auth::Principal,
    parent: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let block = neon_blocks::Block::new(neon_blocks::BlockKind::Database {
        props: neon_blocks::DatabaseProps {
            database_id: id.to_string(),
            view_id: None,
        },
    });
    sync.append(principal, parent, vec![block]).await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/databases/{id}",
    tag = "databases",
    summary = "A database",
    description = "Its page, its properties and its views, for those who see it. Guests read \
                   the views without the people a filter or a person grouping names.",
    params(("id" = Uuid, Path, description = "The database's id: its page's.")),
    responses(
        (status = OK, description = "The database.", body = Database),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such database, or none the caller may see.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_database(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Body<Database>, ApiError> {
    Ok(Body(state.databases.database(&principal, id).await?.into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddProperty {
    #[schema(example = "Priority")]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PropertyKind,
    /// For a number: `plain` when absent.
    #[serde(default)]
    pub number_format: Option<NumberFormat>,
    /// For a number in the currency format.
    #[serde(default)]
    pub currency: Option<String>,
    /// For a unique id: 1 to 10 letters and digits, starting with a letter.
    #[serde(default)]
    pub prefix: Option<String>,
    /// For a select, multi-select or status, at most 200; a status without them gets Not
    /// started, In progress and Done.
    #[serde(default)]
    pub options: Option<Vec<OptionChange>>,
    /// Among the properties; `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
}

#[utoipa::path(
    post,
    path = "/databases/{id}/properties",
    tag = "databases",
    summary = "Add a property",
    description = "For those who edit the database. A database has exactly one title, made with \
                   it, at most one unique id, and at most 100 properties.",
    params(("id" = Uuid, Path, description = "The database's id.")),
    request_body = AddProperty,
    responses(
        (status = CREATED, description = "The property.", body = Property),
        (status = BAD_REQUEST, description = "invalid_input: a name, setting or option out of bounds, settings of another type, a second title or unique id, or a placement beside a property the database lacks.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the database and does not edit it, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "name_taken: another property or option has the name; or limit_reached.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn add_property(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<AddProperty>,
) -> Result<(StatusCode, Body<Property>), ApiError> {
    let new = NewProperty {
        name: body.name,
        spec: PropertySpec {
            kind: body.kind.into(),
            number_format: body.number_format.map(Into::into),
            currency: body.currency,
            prefix: body.prefix,
        },
        options: body
            .options
            .map(|options| options.into_iter().map(Into::into).collect()),
        placement: body.placement.map(Into::into).unwrap_or_default(),
    };
    let property = state.databases.add_property(&principal, id, new).await?;
    Ok((StatusCode::CREATED, Body(property.into())))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeProperty {
    #[serde(default)]
    pub name: Option<String>,
    /// Another type converts each row's value, as the databases reference says; the same
    /// type changes only its settings, which come with it.
    #[serde(default, rename = "type")]
    pub kind: Option<PropertyKind>,
    #[serde(default)]
    pub number_format: Option<NumberFormat>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    /// Every option, in its new order: those named by id keep their values, renamed or
    /// recoloured as asked, and the values of those left out are cleared from the rows.
    #[serde(default)]
    pub options: Option<Vec<OptionChange>>,
    /// Changes the type even when some rows lose their value.
    #[serde(default)]
    pub confirm: bool,
}

#[utoipa::path(
    patch,
    path = "/databases/{id}/properties/{property_id}",
    tag = "databases",
    summary = "Change a property",
    description = "Its name, type with settings, or options, for those who edit the database. A \
                   type that loses values, such as text becoming numbers where some do not read \
                   as one, answers `lossy_change` with how many rows would lose theirs, unless \
                   `confirm` is true, and one that would make a row's values larger than 64 KiB \
                   answers `limit_reached`. The title keeps its type, and guests do not change \
                   a person property's type, whose values would become people's names. Views \
                   that no longer fit leave out what does not.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("property_id" = Uuid, Path, description = "The property's id."),
    ),
    request_body = ChangeProperty,
    responses(
        (status = OK, description = "The property.", body = Property),
        (status = BAD_REQUEST, description = "invalid_input: as for adding one, settings without a type, or options with another type.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the database or the property.", body = ErrorBody),
        (status = CONFLICT, description = "name_taken; lossy_change: the type would lose values, and confirm is not set; or limit_reached: a row's values would take more than 64 KiB.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_property(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, property_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<ChangeProperty>,
) -> Result<Body<Property>, ApiError> {
    let settings = body.number_format.is_some() || body.currency.is_some() || body.prefix.is_some();
    let spec = match body.kind {
        Some(kind) => Some(PropertySpec {
            kind: kind.into(),
            number_format: body.number_format.map(Into::into),
            currency: body.currency,
            prefix: body.prefix,
        }),
        None if settings => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                "settings come with the property's type: name its type too",
            ));
        }
        None => None,
    };
    let change = PropertyChange {
        name: body.name,
        spec,
        options: body
            .options
            .map(|options| options.into_iter().map(Into::into).collect()),
        confirm: body.confirm,
    };
    let property = state
        .databases
        .update_property(&principal, id, property_id, change)
        .await?;
    Ok(Body(property.into()))
}

/// Where something goes among its siblings.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveTo {
    pub placement: Placement,
}

#[utoipa::path(
    post,
    path = "/databases/{id}/properties/{property_id}/move",
    tag = "databases",
    summary = "Place a property elsewhere",
    description = "Among the database's properties, for those who edit it.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("property_id" = Uuid, Path, description = "The property's id."),
    ),
    request_body = MoveTo,
    responses(
        (status = OK, description = "The property.", body = Property),
        (status = BAD_REQUEST, description = "invalid_input: a placement beside a property the database lacks.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn move_property(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, property_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<MoveTo>,
) -> Result<Body<Property>, ApiError> {
    let property = state
        .databases
        .move_property(&principal, id, property_id, body.placement.into())
        .await?;
    Ok(Body(property.into()))
}

#[utoipa::path(
    delete,
    path = "/databases/{id}/properties/{property_id}",
    tag = "databases",
    summary = "Delete a property",
    description = "Any but the title, with its values on every row, for those who edit the \
                   database. Views leave out what named it.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("property_id" = Uuid, Path, description = "The property's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = BAD_REQUEST, description = "invalid_input: the title.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_property(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, property_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .databases
        .delete_property(&principal, id, property_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateView {
    #[schema(example = "Sprint board")]
    pub name: String,
    pub layout: Layout,
    #[serde(default)]
    pub filter: Option<Filter>,
    /// At most 10.
    #[serde(default)]
    pub sorts: Vec<Sort>,
    /// A board's needs one.
    #[serde(default)]
    pub group: Option<Grouping>,
    /// Those listed first, in this order; the others follow, shown in a table and hidden
    /// elsewhere, but the title.
    #[serde(default)]
    pub properties: Vec<ViewProperty>,
    /// Among the views; `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
}

#[utoipa::path(
    post,
    path = "/databases/{id}/views",
    tag = "databases",
    summary = "Add a view",
    description = "For those who edit the database, which has at most 50.",
    params(("id" = Uuid, Path, description = "The database's id.")),
    request_body = CreateView,
    responses(
        (status = CREATED, description = "The view.", body = View),
        (status = BAD_REQUEST, description = "invalid_input: a name out of bounds, a board without grouping, or settings that name what the database lacks or a property's type does not take.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "limit_reached.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_view(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<CreateView>,
) -> Result<(StatusCode, Body<View>), ApiError> {
    let new = NewView {
        name: body.name,
        layout: body.layout.into(),
        settings: domain::ViewSpec {
            filter: body.filter.map(Into::into),
            sorts: body.sorts.into_iter().map(Into::into).collect(),
            group: body.group.map(Into::into),
            properties: body.properties.into_iter().map(Into::into).collect(),
        },
        placement: body.placement.map(Into::into).unwrap_or_default(),
    };
    let view = state.databases.create_view(&principal, id, new).await?;
    Ok((StatusCode::CREATED, Body(view.into())))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeView {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub layout: Option<Layout>,
    /// Null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<Filter>)]
    pub filter: Option<Option<Filter>>,
    #[serde(default)]
    pub sorts: Option<Vec<Sort>>,
    /// Null removes it, but from a board.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<Grouping>)]
    pub group: Option<Option<Grouping>>,
    #[serde(default)]
    pub properties: Option<Vec<ViewProperty>>,
}

#[utoipa::path(
    patch,
    path = "/databases/{id}/views/{view_id}",
    tag = "databases",
    summary = "Change a view",
    description = "What the body names, each part checked against the schema, for those who edit \
                   the database. A board must group once it is made one or its grouping \
                   changes; one whose grouping property is gone takes other changes meanwhile.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("view_id" = Uuid, Path, description = "The view's id."),
    ),
    request_body = ChangeView,
    responses(
        (status = OK, description = "The view.", body = View),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the database or the view.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_view(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, view_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<ChangeView>,
) -> Result<Body<View>, ApiError> {
    let change = ViewChange {
        name: body.name,
        layout: body.layout.map(Into::into),
        filter: body.filter.map(|filter| filter.map(Into::into)),
        sorts: body
            .sorts
            .map(|sorts| sorts.into_iter().map(Into::into).collect()),
        group: body.group.map(|group| group.map(Into::into)),
        properties: body
            .properties
            .map(|properties| properties.into_iter().map(Into::into).collect()),
    };
    let view = state
        .databases
        .update_view(&principal, id, view_id, change)
        .await?;
    Ok(Body(view.into()))
}

#[utoipa::path(
    post,
    path = "/databases/{id}/views/{view_id}/move",
    tag = "databases",
    summary = "Place a view elsewhere",
    description = "Among the database's views, for those who edit it.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("view_id" = Uuid, Path, description = "The view's id."),
    ),
    request_body = MoveTo,
    responses(
        (status = OK, description = "The view.", body = View),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn move_view(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, view_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<MoveTo>,
) -> Result<Body<View>, ApiError> {
    let view = state
        .databases
        .move_view(&principal, id, view_id, body.placement.into())
        .await?;
    Ok(Body(view.into()))
}

#[derive(Deserialize, ToSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct DuplicateView {
    /// The copy's name; the view's with ` (copy)` when absent.
    #[serde(default)]
    pub name: Option<String>,
}

#[utoipa::path(
    post,
    path = "/databases/{id}/views/{view_id}/duplicate",
    tag = "databases",
    summary = "Duplicate a view",
    description = "A copy with the same layout and settings, right after it, for those who edit \
                   the database.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("view_id" = Uuid, Path, description = "The view's id."),
    ),
    request_body = DuplicateView,
    responses(
        (status = CREATED, description = "The copy.", body = View),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "limit_reached.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn duplicate_view(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, view_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<DuplicateView>,
) -> Result<(StatusCode, Body<View>), ApiError> {
    let view = state
        .databases
        .duplicate_view(&principal, id, view_id, body.name)
        .await?;
    Ok((StatusCode::CREATED, Body(view.into())))
}

#[utoipa::path(
    delete,
    path = "/databases/{id}/views/{view_id}",
    tag = "databases",
    summary = "Delete a view",
    description = "For those who edit the database, which keeps at least one.",
    params(
        ("id" = Uuid, Path, description = "The database's id."),
        ("view_id" = Uuid, Path, description = "The view's id."),
    ),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = BAD_REQUEST, description = "invalid_input: the database's last view.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_view(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, view_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.databases.delete_view(&principal, id, view_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
