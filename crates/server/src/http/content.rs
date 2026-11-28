//! `/api/v1/pages/{id}/markdown`: a page's content as markdown with its blocks' ids, and
//! edits of it that name those blocks, made through the page's live document so whoever has
//! it open sees them at once.
//!
//! Reading needs a session or a token with `read`, editing one with `write`. The markdown
//! is the one the docs' block id section describes: a line `<!-- block <id> -->` before each
//! block, which markdown sent back may keep or leave out. Each read has a revision, which an
//! edit that replaces a range or the body names as its `base`, and is refused with
//! `409 edit_conflict` once the page is at another: see [`neon_app::sync::revision`]. Markdown a caller sends, here, to
//! a new page or over MCP, nests at most [`MAX_DEPTH`] levels of blocks: deeper is refused,
//! as an incoming webhook refuses it, rather than read at the last level as an import is.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::databases::{DatabaseService, RowDetails};
use neon_app::sync::{Edited, SyncService, revision_of_markdown};
use neon_blocks::Document;
use neon_blocks::edit::{Applied, Edit};
use neon_blocks::markdown::{MAX_DEPTH, TooDeep, to_markdown_with_ids, try_from_markdown};
use neon_domain::pages::PageKind as DomainPageKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::databases::PropertyKind;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json as Body, Path};
use super::pages::Page;
use super::principal::Authenticated;

/// A page's content as markdown.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PageMarkdown {
    pub page: Page,
    /// Its blocks as markdown, each after a line `<!-- block <id> -->` that names it.
    #[schema(example = "<!-- block 0193a5b2-0000-7000-8000-0000000000b1 -->\n# Plan\n")]
    pub markdown: String,
    /// The revision of what `markdown` shows, which changes whenever it does: an edit that
    /// replaces a range or the body sends it as its `base`.
    #[schema(example = "AAAAAAAAAAAAAAAAAAAAAA")]
    pub revision: String,
    /// For a row: each property of its database, in order, with the row's value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<RowProperty>>,
}

/// A property of a row's database, with the row's value of it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RowProperty {
    pub id: Uuid,
    #[schema(example = "Status")]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PropertyKind,
    /// The value as rows give it, by id; null without one.
    pub value: Option<Json>,
    /// The value as people read it: options and people by name, numbers in their format;
    /// null without one.
    #[schema(example = "In progress")]
    pub text: Option<String>,
}

impl PageMarkdown {
    pub fn new(page: Page, document: &neon_blocks::Document, row: Option<RowDetails>) -> Self {
        let markdown = to_markdown_with_ids(document);
        Self {
            page,
            revision: revision_of_markdown(&markdown),
            markdown,
            properties: row.map(|details| {
                details
                    .properties
                    .into_iter()
                    .map(|(property, text)| RowProperty {
                        value: details.row.values.get(&property.id.to_string()).cloned(),
                        kind: property.kind().into(),
                        id: property.id,
                        name: property.name,
                        text,
                    })
                    .collect()
            }),
        }
    }
}

/// One change to a page's blocks. Markdown may hold the lines naming blocks that reading
/// it gives; they are left out.
#[derive(Debug, Clone, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditOperation {
    /// Adds the blocks of `markdown` at the end of the page, before the empty paragraph
    /// the editor keeps last.
    Append { markdown: String },
    /// Adds the blocks of `markdown` right after the block `block_id`, beside it under the
    /// same block.
    InsertAfter { block_id: String, markdown: String },
    /// Replaces the blocks from `from` to `to`, siblings in order, both included, with the
    /// blocks of `markdown`, or removes them when it is empty. `to` is `from` when absent.
    ReplaceRange {
        from: String,
        #[serde(default)]
        to: Option<String>,
        #[serde(default)]
        markdown: String,
    },
    /// Replaces `find`, matched exactly within one line of a block's text, with
    /// `replace`, which takes the styles of the first character it replaces. Unless `all`,
    /// `find` must be in the page once; with `all`, it is replaced at most 10000 times.
    FindReplace {
        find: String,
        replace: String,
        #[serde(default)]
        all: bool,
    },
    /// Replaces every block with those of `markdown`, but those that come out the same,
    /// which stay as they are with their ids. The markdown reading gives, sent back with
    /// its lines naming blocks, changes nothing.
    ReplaceBody { markdown: String },
}

/// The blocks of markdown a caller sends, refused with `invalid_input` when they nest deeper
/// than [`MAX_DEPTH`] levels.
pub(crate) fn read_markdown(markdown: &str) -> Result<Document, ApiError> {
    try_from_markdown(markdown).map_err(|TooDeep| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("the markdown nests blocks more than {MAX_DEPTH} levels deep"),
        )
    })
}

impl TryFrom<EditOperation> for Edit {
    type Error = ApiError;

    fn try_from(operation: EditOperation) -> Result<Self, ApiError> {
        let blocks = |markdown: &str| read_markdown(markdown).map(|document| document.blocks);
        Ok(match operation {
            EditOperation::Append { markdown } => Self::Append {
                blocks: blocks(&markdown)?,
            },
            EditOperation::InsertAfter { block_id, markdown } => Self::InsertAfter {
                block_id,
                blocks: blocks(&markdown)?,
            },
            EditOperation::ReplaceRange { from, to, markdown } => Self::ReplaceRange {
                to: to.unwrap_or_else(|| from.clone()),
                from,
                blocks: blocks(&markdown)?,
            },
            EditOperation::FindReplace { find, replace, all } => {
                Self::FindReplace { find, replace, all }
            }
            EditOperation::ReplaceBody { markdown } => Self::ReplaceBody {
                blocks: blocks(&markdown)?,
            },
        })
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditPage {
    /// Made in order, all or none: 1 to 50.
    pub operations: Vec<EditOperation>,
    /// The revision of the page the operations were made from, as reading it or the last
    /// edit answered. Needed by `replace_range` and `replace_body`, which are refused with
    /// `edit_conflict` once the page is at another revision.
    #[serde(default)]
    pub base: Option<String>,
}

/// What an edit did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EditedPage {
    pub page: Page,
    /// What each operation did, in order.
    pub results: Vec<EditResult>,
    /// The page's revision after the edit, when it was made on its `base`: the next edit
    /// may name it without reading again. Null otherwise, as the page may hold changes the
    /// caller has not read.
    pub revision: Option<String>,
}

/// What one operation did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EditResult {
    /// The ids of the blocks it added, in order, those under them left out.
    pub added: Vec<String>,
    /// How many blocks it removed, those under them left out.
    pub removed: usize,
    /// The ids of the blocks it removed, in order, those under them left out.
    pub removed_ids: Vec<String>,
    /// How many times it replaced the text it was to find.
    pub replaced: usize,
}

impl From<Applied> for EditResult {
    fn from(applied: Applied) -> Self {
        Self {
            added: applied.added,
            removed: applied.removed,
            removed_ids: applied.removed_ids,
            replaced: applied.replaced,
        }
    }
}

impl From<Edited> for EditedPage {
    fn from(edited: Edited) -> Self {
        Self {
            page: edited.page.into(),
            results: edited.applied.into_iter().map(Into::into).collect(),
            revision: edited.revision,
        }
    }
}

/// A page's content as markdown, with a row's properties: what reading it shows.
pub(crate) async fn page_markdown(
    sync: &SyncService,
    databases: &DatabaseService,
    principal: &neon_domain::auth::Principal,
    id: Uuid,
) -> Result<PageMarkdown, ApiError> {
    let (view, document) = sync.read(principal, id).await?;
    let row = match view.page.kind {
        DomainPageKind::Row => Some(databases.row_details(principal, id).await?),
        _ => None,
    };
    Ok(PageMarkdown::new(view.into(), &document, row))
}

#[utoipa::path(
    get,
    path = "/pages/{id}/markdown",
    tag = "pages",
    summary = "A page's content as markdown",
    description = "Its blocks as markdown, each block's id on a line of its own before it, \
                   `<!-- block <id> -->`, which the edits name, and for a row its properties \
                   with their values; and the revision they are at, which an edit replacing \
                   blocks names as its base. A database has no body of its own: read its rows.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = OK, description = "The page and its markdown.", body = PageMarkdown),
        (status = BAD_REQUEST, description = "invalid_input: a database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_markdown(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Body<PageMarkdown>, ApiError> {
    Ok(Body(
        page_markdown(&state.sync, &state.databases, &principal, id).await?,
    ))
}

#[utoipa::path(
    patch,
    path = "/pages/{id}/markdown",
    tag = "pages",
    summary = "Edit a page's content",
    description = "Makes the operations in order, all or none, through the page's live \
                   document: whoever has the page open sees the change at once, and what they \
                   type meanwhile merges with it. Operations name blocks by the ids reading the \
                   markdown shows. replace_range and replace_body need base, the revision the \
                   markdown they were made from was read at: once the page is at another, they \
                   are refused, as they would remove what was added or changed since. Editing \
                   needs edit access to the page, out of the trash. A page holds at most 16 \
                   MiB. Each user has 100 edits at once, through this route and MCP together, \
                   then two a second.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    request_body = EditPage,
    responses(
        (status = OK, description = "What each operation did.", body = EditedPage),
        (status = BAD_REQUEST, description = "invalid_input: no operations or more than 50, markdown nesting blocks more than 32 levels deep, or a database; base_required: replace_range or replace_body without base; edit_refused: an operation names a block the page lacks or blocks that are no range, or its text is not in the page, there more than once, or with all more than 10000 times, or its blocks would go in so far into a long page that one edit's writes pass more than 4000000 blocks, or take longer than a second, and nothing changed.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the page and does not edit it, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "edit_conflict: the page is no longer at base, and the edit replaces blocks; nothing changed.", body = ErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "document_too_large: the page would grow past what it holds, or the operations' markdown makes more than 100000 blocks, table rows and cells, runs of text, line breaks and mentions together.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited: too many edits of pages' content; Retry-After says when to try again.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "unavailable: the server is stopping.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn edit_markdown(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Body(body): Body<EditPage>,
) -> Result<Body<EditedPage>, ApiError> {
    let edits = body
        .operations
        .into_iter()
        .map(Edit::try_from)
        .collect::<Result<_, _>>()?;
    let edited = state.sync.edit(&principal, id, edits, body.base).await?;
    Ok(Body(edited.into()))
}
