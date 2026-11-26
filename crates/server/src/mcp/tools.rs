//! The MCP server's tools. Each acts in the workspace the caller's token is bound to, through
//! the app's services, as the API's routes do, and answers with the same JSON as its route
//! does, as structured content. Its arguments are checked against the JSON schema it
//! publishes, as their types read them: an unknown field, a missing one or a value of the
//! wrong type is refused. A refusal is a tool error, with the API's code and message.
//!
//! Each tool's action is also a route of the API, taking the same tokens: a test holds the
//! table of them, and fails when a tool has no route or a route goes.

use std::collections::HashMap;
use std::sync::Arc;

use neon_app::databases::{DatabaseError, NewDatabase, NewRow, QueryInput};
use neon_app::pages::NewPage;
use neon_blocks::edit::Edit;
use neon_blocks::markdown::from_markdown;
use neon_domain::auth::{Principal, Scope};
use neon_domain::pages::PageStyle;
use rmcp::handler::server::common::schema_for_input;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value as Json, json};
use uuid::Uuid;

use super::Services;
use crate::http::ApiError;
use crate::http::content::{EditOperation, EditedPage, page_markdown};
use crate::http::database_rows::{QueryAnswer, Row};
use crate::http::databases::{Database, Filter, Sort};
use crate::http::pages::{Page, Placement, TreePage, location};
use crate::http::teamspaces::TeamspaceList;
use crate::http::whoami::WhoAmI;
use crate::http::workspaces::{Member, MemberList};

/// How deep a tree goes unless asked otherwise, and at most.
const TREE_DEPTH: u8 = 3;
const MAX_TREE_DEPTH: u8 = 10;

/// The most pages a tree holds, read a part at a time.
const MAX_TREE_PAGES: usize = 2000;

/// Why a tool did not do what it was asked.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ToolError {
    /// The API's code, such as `not_found`.
    pub(crate) code: &'static str,
    pub(crate) message: String,
    /// The caller's to fix, rather than the server's failure.
    pub(crate) expected: bool,
    /// More of what went wrong, beside the code and message.
    details: Option<(&'static str, Json)>,
}

impl ToolError {
    pub(crate) fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            expected: true,
            details: None,
        }
    }

    /// The tool's answer: an error the caller reads, its code and message also as
    /// structured content.
    pub(crate) fn into_result(self) -> CallToolResult {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some((key, value)) = self.details {
            error[key] = value;
        }
        let mut result = CallToolResult::error(vec![ContentBlock::text(self.message)]);
        result.structured_content = Some(json!({"error": error}));
        result
    }
}

impl From<ApiError> for ToolError {
    fn from(error: ApiError) -> Self {
        Self {
            code: error.code(),
            message: error.message().to_owned(),
            expected: !error.status().is_server_error(),
            details: None,
        }
    }
}

/// A use case's refusal, as the API answers it.
fn refused(error: impl Into<ApiError>) -> ToolError {
    ToolError::from(error.into())
}

/// A tool: its name, what it is for, the scope it needs and the schema of its arguments.
pub(crate) struct Spec {
    pub(crate) name: &'static str,
    title: &'static str,
    description: &'static str,
    /// Needs the write scope; otherwise the tool only reads.
    writes: bool,
    /// May remove or overwrite what is there, rather than only add: values, a status, a
    /// page's place.
    destructive: bool,
    /// Called again with the same arguments, changes nothing more.
    idempotent: bool,
    schema: fn() -> Arc<JsonObject>,
}

impl Spec {
    pub(crate) fn tool(&self) -> Tool {
        let annotations = ToolAnnotations::with_title(self.title)
            .read_only(!self.writes)
            .destructive(self.destructive)
            .idempotent(self.idempotent)
            .open_world(false);
        Tool::new(self.name, self.description, (self.schema)())
            .with_title(self.title)
            .with_annotations(annotations)
    }
}

/// The JSON schema of a tool's arguments.
fn schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    schema_for_input::<T>().unwrap_or_else(|_| {
        let mut object = JsonObject::new();
        object.insert("type".to_owned(), json!("object"));
        Arc::new(object)
    })
}

/// Every tool, in the order tools/list gives them.
pub(crate) const TOOLS: [Spec; 16] = [
    Spec {
        name: "whoami",
        title: "Who am I",
        description: "Whom this connection acts as: the identity, a person or an agent with the \
                      person it acts for, its scopes, and the workspace it works in with its \
                      role there.",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<NoArguments>,
    },
    Spec {
        name: "list_members",
        title: "List members",
        description: "The workspace's members, people and agents, with their roles. An agent \
                      shows the person it acts for. A guest sees only themselves.",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<NoArguments>,
    },
    Spec {
        name: "list_teamspaces",
        title: "List teamspaces",
        description: "The workspace's teamspaces this connection sees, the default one first, \
                      with its role in each. Pages live in teamspaces, or are private.",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<NoArguments>,
    },
    Spec {
        name: "get_page_tree",
        title: "Get a page tree",
        description: "The pages of a teamspace, or the private pages when no teamspace is named, \
                      nested to a depth: each with its kind (page, database), title and how \
                      many pages are under it. Databases' rows are left out: query the database.",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<TreeArguments>,
    },
    Spec {
        name: "read_page",
        title: "Read a page",
        description: "A page's blocks as markdown, each block's id on a line of its own before \
                      it, `<!-- block <id> -->`, which edit_page names; and for a row of a \
                      database, its properties with their values. A database itself has no \
                      body: use get_database and query_database.",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<PageArguments>,
    },
    Spec {
        name: "create_page",
        title: "Create a page",
        description: "A page under another page (parent_id), at the top of a teamspace \
                      (teamspace_id), or among your private pages (private: true): name one. \
                      It starts with the blocks of `markdown`. Answers with its markdown and \
                      block ids. Rows of a database are made with create_row.",
        writes: true,
        destructive: false,
        idempotent: false,
        schema: schema::<CreatePageArguments>,
    },
    Spec {
        name: "edit_page",
        title: "Edit a page",
        description: "Changes a page's blocks, all operations or none, live: whoever has the page \
                      open sees it at once, and what they type meanwhile stays. Operations: \
                      append, insert_after a block, replace_range of sibling blocks (empty \
                      markdown removes them), find_replace of text within one line (once, unless \
                      all), replace_body (blocks that stay the same are kept). Blocks are named \
                      by the ids read_page shows.",
        writes: true,
        destructive: true,
        idempotent: false,
        schema: schema::<EditArguments>,
    },
    Spec {
        name: "move_page",
        title: "Move a page",
        description: "Moves a page, with the pages under it, under another page (parent_id), to \
                      the top of a teamspace (teamspace_id) or to your private pages (private: \
                      true), placed among its new siblings. Rows move within their database \
                      only.",
        writes: true,
        destructive: true,
        idempotent: true,
        schema: schema::<MoveArguments>,
    },
    Spec {
        name: "trash_page",
        title: "Trash a page",
        description: "Moves a page, with the pages under it, to the trash, where restore_page \
                      brings it back until the trash is emptied.",
        writes: true,
        destructive: true,
        idempotent: false,
        schema: schema::<PageArguments>,
    },
    Spec {
        name: "restore_page",
        title: "Restore a page",
        description: "Brings a page back from the trash, with the pages trashed with it, under \
                      its parent or, when that is in the trash too, at the top of its space.",
        writes: true,
        destructive: false,
        idempotent: false,
        schema: schema::<PageArguments>,
    },
    Spec {
        name: "get_database",
        title: "Get a database",
        description: "A database's page, its properties (the columns: each with its id, name, \
                      type and options) and its views (each with its filter, sorts and \
                      grouping).",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<DatabaseArguments>,
    },
    Spec {
        name: "query_database",
        title: "Query a database",
        description: "A database's rows, as a view has them or with a filter and sorts of your \
                      own, a part at a time: pass `next` back as `after` for the next part. \
                      Values come by property id; get_database names the properties. A filter \
                      is a condition {property, operator, value} or {\"and\": [...]} / \
                      {\"or\": [...]}; `value` may be `me` for a person property.",
        writes: false,
        destructive: false,
        idempotent: true,
        schema: schema::<QueryArguments>,
    },
    Spec {
        name: "create_database",
        title: "Create a database",
        description: "A database under a page (parent_id, inline to embed it in that page), at \
                      the top of a teamspace (teamspace_id) or among your private pages \
                      (private: true). It starts with a Name title, a Status with Not started, \
                      In progress and Done, a table view and a board grouped by status.",
        writes: true,
        destructive: false,
        idempotent: false,
        schema: schema::<CreateDatabaseArguments>,
    },
    Spec {
        name: "create_row",
        title: "Create a row",
        description: "Adds a row to a database with its values, by property id or name: the \
                      title property's is the row's title, options by name or id, people by \
                      member id. Its body is a page: edit it with edit_page.",
        writes: true,
        destructive: false,
        idempotent: false,
        schema: schema::<CreateRowArguments>,
    },
    Spec {
        name: "update_row",
        title: "Update a row",
        description: "Sets the values named, by property id or name, of a row named by row_id, \
                      or by database_id and its unique id such as NW-42; null clears one, and \
                      the others stay.",
        writes: true,
        destructive: true,
        idempotent: true,
        schema: schema::<UpdateRowArguments>,
    },
    Spec {
        name: "claim_task",
        title: "Claim a task",
        description: "Takes a row nobody holds: names you in its database's first person \
                      property, or the one named, and moves it to the first status in \
                      progress, at once. Held already, it answers already_claimed with who \
                      holds it, and changes nothing. Name the row by row_id, or by database_id \
                      and its unique id such as NW-42.",
        writes: true,
        destructive: true,
        idempotent: false,
        schema: schema::<ClaimArguments>,
    },
];

pub(crate) fn list() -> Vec<Tool> {
    TOOLS.iter().map(Spec::tool).collect()
}

pub(crate) fn find(name: &str) -> Option<&'static Spec> {
    TOOLS.iter().find(|spec| spec.name == name)
}

/// The arguments of a call, as their type reads them.
fn arguments<T: DeserializeOwned>(arguments: JsonObject) -> Result<T, ToolError> {
    serde_json::from_value(Json::Object(arguments)).map_err(|error| {
        ToolError::invalid(
            "invalid_arguments",
            format!("the arguments do not fit the tool's schema: {error}"),
        )
    })
}

fn answer(value: impl serde::Serialize) -> Result<Json, ToolError> {
    serde_json::to_value(value).map_err(|error| ToolError {
        code: "internal",
        message: format!("the answer does not serialize: {error}"),
        expected: false,
        details: None,
    })
}

/// The workspace the caller's token is bound to, which the gate made sure it has.
fn workspace(principal: &Principal) -> Result<Uuid, ToolError> {
    principal.workspace().ok_or_else(|| {
        ToolError::invalid(
            "token_not_bound",
            "the MCP server works in the one workspace a token is bound to",
        )
    })
}

/// Runs a tool for the principal.
pub(crate) async fn call(
    services: &Services,
    principal: &Principal,
    spec: &Spec,
    given: JsonObject,
) -> Result<Json, ToolError> {
    if spec.writes && !principal.can(Scope::Write) {
        return Err(ToolError::invalid(
            "insufficient_scope",
            format!(
                "{} changes the workspace, and this token lacks the write scope",
                spec.name
            ),
        ));
    }
    let s = services;
    match spec.name {
        "whoami" => {
            arguments::<NoArguments>(given)?;
            let identity = s.workspaces.identity(principal).await.map_err(refused)?;
            answer(WhoAmI::new(principal, identity))
        }
        "list_members" => {
            arguments::<NoArguments>(given)?;
            let members = s
                .workspaces
                .list_members(principal, workspace(principal)?)
                .await
                .map_err(refused)?;
            answer(MemberList {
                members: members.into_iter().map(Member::from).collect(),
            })
        }
        "list_teamspaces" => {
            arguments::<NoArguments>(given)?;
            let list = s
                .workspaces
                .list_teamspaces(principal, workspace(principal)?)
                .await
                .map_err(refused)?;
            answer(TeamspaceList::from(list))
        }
        "get_page_tree" => tree(s, principal, arguments(given)?).await,
        "read_page" => {
            let PageArguments { page_id } = arguments(given)?;
            answer(page_markdown(&s.sync, &s.databases, principal, page_id).await?)
        }
        "create_page" => create_page(s, principal, arguments(given)?).await,
        "edit_page" => {
            let EditArguments {
                page_id,
                operations,
            } = arguments(given)?;
            let edits = operations.into_iter().map(Edit::from).collect();
            let edited = s
                .sync
                .edit(principal, page_id, edits)
                .await
                .map_err(refused)?;
            answer(EditedPage::from(edited))
        }
        "move_page" => {
            let MoveArguments {
                page_id,
                parent_id,
                teamspace_id,
                private,
                placement,
            } = arguments(given)?;
            let to = location(parent_id, teamspace_id, private)?;
            let placement = placement.map(Into::into).unwrap_or_default();
            let moved = s
                .pages
                .move_page(principal, page_id, to, placement)
                .await
                .map_err(refused)?;
            answer(Page::from(moved))
        }
        "trash_page" => {
            let PageArguments { page_id } = arguments(given)?;
            let trashed = s
                .pages
                .trash_page(principal, page_id)
                .await
                .map_err(refused)?;
            answer(Page::from(trashed))
        }
        "restore_page" => {
            let PageArguments { page_id } = arguments(given)?;
            let restored = s
                .pages
                .restore_page(principal, page_id)
                .await
                .map_err(refused)?;
            answer(Page::from(restored))
        }
        "get_database" => {
            let DatabaseArguments { database_id } = arguments(given)?;
            let database = s
                .databases
                .database(principal, database_id)
                .await
                .map_err(refused)?;
            answer(Database::from(database))
        }
        "query_database" => query(s, principal, arguments(given)?).await,
        "create_database" => create_database(s, principal, arguments(given)?).await,
        "create_row" => {
            let CreateRowArguments {
                database_id,
                values,
                placement,
            } = arguments(given)?;
            let new = NewRow {
                values,
                placement: placement.map(Into::into).unwrap_or_default(),
                group: None,
            };
            let row = s
                .databases
                .create_row(principal, database_id, new)
                .await
                .map_err(refused)?;
            answer(Row::from(row))
        }
        "update_row" => {
            let UpdateRowArguments {
                row_id,
                database_id,
                unique_id,
                values,
            } = arguments(given)?;
            let row_id = row_named(s, principal, row_id, database_id, unique_id).await?;
            let row = s
                .databases
                .update_row(principal, row_id, values)
                .await
                .map_err(refused)?;
            answer(Row::from(row))
        }
        "claim_task" => {
            let ClaimArguments {
                row_id,
                database_id,
                unique_id,
                person_property,
            } = arguments(given)?;
            let row_id = row_named(s, principal, row_id, database_id, unique_id).await?;
            match s
                .databases
                .claim_row(principal, row_id, person_property.as_deref())
                .await
            {
                Ok(row) => answer(Row::from(row)),
                Err(DatabaseError::AlreadyClaimed(holders)) => {
                    let message = DatabaseError::AlreadyClaimed(holders.clone()).to_string();
                    let holders = holders
                        .into_iter()
                        .map(|holder| json!({"id": holder.id, "name": holder.name}))
                        .collect();
                    Err(ToolError {
                        code: "already_claimed",
                        message,
                        expected: true,
                        details: Some(("holders", Json::Array(holders))),
                    })
                }
                Err(error) => Err(refused(error)),
            }
        }
        other => Err(ToolError {
            code: "internal",
            message: format!("the tool {other} has no implementation"),
            expected: false,
            details: None,
        }),
    }
}

/// No arguments: an empty object.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArguments {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageArguments {
    /// The page's id, a UUID.
    page_id: Uuid,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DatabaseArguments {
    /// The database's id, a UUID: its page's.
    database_id: Uuid,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TreeArguments {
    /// The teamspace whose pages to list; your private pages when absent.
    #[serde(default)]
    teamspace_id: Option<Uuid>,
    /// How many levels of pages to give, 1 to 10; 3 when absent. Pages at the last level
    /// still tell how many pages are under them.
    #[serde(default)]
    #[schemars(range(min = 1, max = 10))]
    depth: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreatePageArguments {
    /// The page to put it under. Name this, teamspace_id, or private: true.
    #[serde(default)]
    parent_id: Option<Uuid>,
    /// The teamspace to put it at the top of.
    #[serde(default)]
    teamspace_id: Option<Uuid>,
    /// True to make it one of your private pages.
    #[serde(default)]
    private: bool,
    /// Where among its siblings: "first", "last", {"after": id} or {"before": id}; "last"
    /// when absent.
    #[serde(default)]
    placement: Option<Placement>,
    /// Up to 500 characters; empty shows as Untitled.
    #[serde(default)]
    title: String,
    /// An emoji or up to 16 characters.
    #[serde(default)]
    icon: Option<String>,
    /// Its content, as markdown.
    #[serde(default)]
    markdown: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditArguments {
    /// The page's id, a UUID.
    page_id: Uuid,
    /// Made in order, all or none: 1 to 50.
    operations: Vec<EditOperation>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveArguments {
    /// The page to move, a UUID.
    page_id: Uuid,
    /// The page to move it under. Name this, teamspace_id, or private: true.
    #[serde(default)]
    parent_id: Option<Uuid>,
    /// The teamspace to move it to the top of.
    #[serde(default)]
    teamspace_id: Option<Uuid>,
    /// True to move it to the top of your private pages.
    #[serde(default)]
    private: bool,
    /// Where among its new siblings; "last" when absent.
    #[serde(default)]
    placement: Option<Placement>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct QueryArguments {
    /// The database's id, a UUID.
    database_id: Uuid,
    /// The view whose filter, sorts and grouping to use; every row, in manual order, when
    /// absent.
    #[serde(default)]
    view_id: Option<Uuid>,
    /// In place of the view's filter.
    #[serde(default)]
    filter: Option<Filter>,
    /// In place of the view's sorts.
    #[serde(default)]
    sorts: Option<Vec<Sort>>,
    /// Rows in the part, 1 to 1000; 100 when absent.
    #[serde(default)]
    #[schemars(range(min = 1, max = 1000))]
    limit: Option<usize>,
    /// The `next` of the part before.
    #[serde(default)]
    after: Option<String>,
    /// Your UTC offset in minutes, which `today` is told at; 0 when absent.
    #[serde(default)]
    utc_offset_minutes: Option<i16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateDatabaseArguments {
    /// The page to put it under. Name this, teamspace_id, or private: true.
    #[serde(default)]
    parent_id: Option<Uuid>,
    /// The teamspace to put it at the top of.
    #[serde(default)]
    teamspace_id: Option<Uuid>,
    /// True to make it one of your private pages.
    #[serde(default)]
    private: bool,
    /// Embedded in the page it goes under, which parent_id names.
    #[serde(default)]
    inline: bool,
    /// Where among its siblings; "last" when absent.
    #[serde(default)]
    placement: Option<Placement>,
    /// Up to 500 characters.
    #[serde(default)]
    title: String,
    /// An emoji or up to 16 characters.
    #[serde(default)]
    icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateRowArguments {
    /// The database's id, a UUID.
    database_id: Uuid,
    /// Values by property id or name: a title, text or URL as a string, a number as a
    /// number, a select or status by option name or id, a multi-select as a list of them, a
    /// date as {"start": "2026-11-22", "end": ...}, a person as a list of member ids, a
    /// checkbox as true.
    #[serde(default)]
    values: Map<String, Json>,
    /// Where among the rows; "last" when absent.
    #[serde(default)]
    placement: Option<Placement>,
}

/// The row a call names: by its id, or by its database and unique id.
async fn row_named(
    s: &Services,
    principal: &Principal,
    row_id: Option<Uuid>,
    database_id: Option<Uuid>,
    unique_id: Option<String>,
) -> Result<Uuid, ToolError> {
    match (row_id, database_id, unique_id) {
        (Some(row_id), None, None) => Ok(row_id),
        (None, Some(database_id), Some(unique_id)) => Ok(s
            .databases
            .row_by_unique_id(principal, database_id, &unique_id)
            .await
            .map_err(refused)?
            .id),
        _ => Err(ToolError::invalid(
            "invalid_arguments",
            "name the row by row_id, or by database_id and unique_id",
        )),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateRowArguments {
    /// The row's id, a UUID: its page's. Or name database_id and unique_id.
    #[serde(default)]
    row_id: Option<Uuid>,
    /// With unique_id: the database the row is in.
    #[serde(default)]
    database_id: Option<Uuid>,
    /// With database_id: the row's unique id, such as NW-42, or its number alone.
    #[serde(default)]
    unique_id: Option<String>,
    /// The values to set, by property id or name, as create_row takes them; null clears
    /// one.
    values: Map<String, Json>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClaimArguments {
    /// The row's id, a UUID: its page's. Or name database_id and unique_id.
    #[serde(default)]
    row_id: Option<Uuid>,
    /// With unique_id: the database the row is in.
    #[serde(default)]
    database_id: Option<Uuid>,
    /// With database_id: the row's unique id, such as NW-42, or its number alone.
    #[serde(default)]
    unique_id: Option<String>,
    /// The person property to name you in, by id or name; the database's first when
    /// absent.
    #[serde(default)]
    person_property: Option<String>,
}

async fn create_page(
    s: &Services,
    principal: &Principal,
    given: CreatePageArguments,
) -> Result<Json, ToolError> {
    let at = location(given.parent_id, given.teamspace_id, given.private)?;
    let content = match &given.markdown {
        Some(markdown) => s
            .sync
            .first_snapshot(&from_markdown(markdown))
            .map_err(refused)?,
        None => None,
    };
    let new = NewPage {
        title: given.title,
        icon: given.icon,
        cover: None,
        style: PageStyle::default(),
        content,
    };
    let placement = given.placement.map(Into::into).unwrap_or_default();
    let created = s
        .pages
        .create_page(principal, workspace(principal)?, at, placement, new)
        .await
        .map_err(refused)?;
    answer(page_markdown(&s.sync, &s.databases, principal, created.page.id).await?)
}

async fn create_database(
    s: &Services,
    principal: &Principal,
    given: CreateDatabaseArguments,
) -> Result<Json, ToolError> {
    let at = location(given.parent_id, given.teamspace_id, given.private)?;
    let placement = given.placement.map(Into::into).unwrap_or_default();
    let created = s
        .databases
        .create_database(
            principal,
            workspace(principal)?,
            at,
            placement,
            NewDatabase {
                title: given.title,
                icon: given.icon,
                inline: given.inline,
            },
        )
        .await
        .map_err(refused)?;
    answer(Database::from(created))
}

async fn query(
    s: &Services,
    principal: &Principal,
    given: QueryArguments,
) -> Result<Json, ToolError> {
    let input = QueryInput {
        view_id: given.view_id,
        filter: given.filter.map(Into::into),
        sorts: given
            .sorts
            .map(|sorts| sorts.into_iter().map(Into::into).collect()),
        limit: given.limit,
        after: given.after,
        utc_offset_minutes: given.utc_offset_minutes,
    };
    let answered = s
        .databases
        .query(principal, given.database_id, input)
        .await
        .map_err(refused)?;
    answer(QueryAnswer::from(answered))
}

/// A page of a tree, with the pages under it to the depth asked.
#[derive(Debug, serde::Serialize)]
struct Node {
    #[serde(flatten)]
    page: TreePage,
    /// The pages under it; absent past the depth asked, when `child_count` still tells
    /// how many there are.
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<Node>>,
}

async fn tree(
    s: &Services,
    principal: &Principal,
    given: TreeArguments,
) -> Result<Json, ToolError> {
    let depth = given.depth.unwrap_or(TREE_DEPTH);
    if !(1..=MAX_TREE_DEPTH).contains(&depth) {
        return Err(ToolError::invalid(
            "invalid_arguments",
            format!("a tree is 1 to {MAX_TREE_DEPTH} levels deep"),
        ));
    }
    let ws = workspace(principal)?;
    let mut pages = Vec::new();
    let mut after: Option<String> = None;
    let mut truncated = false;
    loop {
        let part = match given.teamspace_id {
            Some(teamspace) => {
                s.pages
                    .teamspace_tree(principal, teamspace, after.as_deref(), None)
                    .await
            }
            None => {
                s.pages
                    .private_tree(principal, ws, after.as_deref(), None)
                    .await
            }
        }
        .map_err(refused)?;
        pages.extend(part.pages.into_iter().map(TreePage::from));
        match part.next {
            Some(next) if pages.len() < MAX_TREE_PAGES => after = Some(next),
            Some(_) => {
                truncated = true;
                break;
            }
            None => break,
        }
    }
    answer(json!({
        "teamspace_id": given.teamspace_id,
        "private": given.teamspace_id.is_none(),
        "pages": nest(pages, depth),
        "truncated": truncated,
    }))
}

/// The pages of a tree, each before the pages under it, nested to `depth` levels.
fn nest(pages: Vec<TreePage>, depth: u8) -> Vec<Node> {
    // The level of each page: its parent's plus one, at the top when its parent is not
    // listed.
    let mut levels: HashMap<Uuid, u8> = HashMap::new();
    let mut children: HashMap<Option<Uuid>, Vec<TreePage>> = HashMap::new();
    for page in pages {
        let level = page
            .parent_id
            .and_then(|parent| levels.get(&parent))
            .map_or(1, |level| level.saturating_add(1));
        levels.insert(page.id, level);
        let parent = page.parent_id.filter(|_| level > 1);
        children.entry(parent).or_default().push(page);
    }
    fn build(
        parent: Option<Uuid>,
        level: u8,
        depth: u8,
        children: &mut HashMap<Option<Uuid>, Vec<TreePage>>,
    ) -> Vec<Node> {
        children
            .remove(&parent)
            .unwrap_or_default()
            .into_iter()
            .map(|page| {
                let id = page.id;
                let under = (level < depth).then(|| build(Some(id), level + 1, depth, children));
                Node {
                    page,
                    children: under,
                }
            })
            .collect()
    }
    build(None, 1, depth, &mut children)
}

#[cfg(test)]
mod tests;
