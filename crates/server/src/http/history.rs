//! `/api/v1/pages/{id}/versions`: a page's history, the versions its snapshots left.
//!
//! Listing, reading and comparing need to see the page, with `read`; restoring needs to edit
//! it, with `write`, and the revision the comparison answered, which it is refused without
//! once the page changed: see [`neon_app::history`]. Guests, and agents of guests, get no
//! authors.

use axum::extract::State;
use neon_app::history::{
    Comparison as AppComparison, Restored, Version as AppVersion, VersionContent,
    VersionList as AppVersionList,
};
use neon_blocks::diff::{Change, Line};
use neon_blocks::markdown::{to_markdown, to_markdown_with_ids};
use neon_blocks::{Block, Document};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json as Body, Path, Query};
use super::pages::Page;
use super::principal::Authenticated;

/// A version of a page: its content each time the server saved it changed.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Version {
    pub id: Uuid,
    pub page_id: Uuid,
    /// When its latest change was made.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Who changed the page since the version before: people, agents and integrations, by
    /// member id. Empty for guests, who know no other member.
    pub authors: Vec<Uuid>,
    /// How many blocks it added, changed and removed since the version before, at any
    /// depth; the first version counts every block as added.
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    /// For a version a restore made: the version restored, while it is kept.
    pub restored_from: Option<Uuid>,
    /// And when that one was made.
    #[serde(with = "time::serde::rfc3339::option")]
    pub restored_from_at: Option<OffsetDateTime>,
}

impl From<AppVersion> for Version {
    fn from(version: AppVersion) -> Self {
        Self {
            id: version.id,
            page_id: version.page_id,
            created_at: version.created_at,
            authors: version.authors,
            added: version.summary.added,
            changed: version.summary.changed,
            removed: version.summary.removed,
            restored_from: version.restored_from,
            restored_from_at: version.restored_from_at,
        }
    }
}

/// A part of a page's history, latest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VersionList {
    pub page: Page,
    pub versions: Vec<Version>,
    /// Pass it as `after` for the next part; absent on the last one.
    pub next: Option<String>,
}

impl From<AppVersionList> for VersionList {
    fn from(list: AppVersionList) -> Self {
        Self {
            page: list.page.into(),
            versions: list.versions.into_iter().map(Version::from).collect(),
            next: list.next,
        }
    }
}

/// A version with its content.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PageVersion {
    pub page: Page,
    pub version: Version,
    /// Its blocks in BlockNote's JSON, as `/ws/pages/{id}` would give the editor them.
    pub blocks: Vec<Json>,
    /// Its blocks as markdown, each after a line `<!-- block <id> -->` that names it, as
    /// `GET /pages/{id}/markdown` writes them.
    pub markdown: String,
}

impl PageVersion {
    pub fn new(content: VersionContent) -> Result<Self, ApiError> {
        let blocks = content
            .document
            .blocks
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(|error| {
                tracing::error!(%error, "a version's blocks do not serialize");
                ApiError::internal()
            })?;
        Ok(Self {
            markdown: to_markdown_with_ids(&content.document),
            page: content.page.into(),
            version: content.version.into(),
            blocks,
        })
    }
}

/// What happened to a block, from the version to the page as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockChangeKind {
    Same,
    Changed,
    /// On the page, not in the version: restoring removes it.
    Added,
    /// In the version, not on the page: restoring brings it back.
    Removed,
}

impl From<Change> for BlockChangeKind {
    fn from(change: Change) -> Self {
        match change {
            Change::Same => Self::Same,
            Change::Changed => Self::Changed,
            Change::Added => Self::Added,
            Change::Removed => Self::Removed,
        }
    }
}

/// One block of a comparison, without the blocks under it, which follow it a level deeper.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BlockChange {
    pub change: BlockChangeKind,
    /// How deep the block is: 0 at the top of the page.
    pub depth: usize,
    /// The block's id, on the page or else in the version.
    pub block_id: Option<String>,
    /// The block as the version has it, as markdown, when it changed or was removed.
    pub before: Option<String>,
    /// The block as the page has it, as markdown, unless it was removed.
    pub after: Option<String>,
}

fn alone(block: Block) -> String {
    to_markdown(&Document::new(vec![block]))
}

impl From<Line> for BlockChange {
    fn from(line: Line) -> Self {
        let block_id = line
            .after
            .as_ref()
            .or(line.before.as_ref())
            .and_then(|block| block.id.clone());
        let before = match line.change {
            Change::Changed | Change::Removed => line.before.map(alone),
            Change::Same | Change::Added => None,
        };
        Self {
            change: line.change.into(),
            depth: line.depth,
            block_id,
            before,
            after: line.after.map(alone),
        }
    }
}

/// A version compared with the page as it is now.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Comparison {
    pub page: Page,
    pub version: Version,
    /// Every block of both, in the page's order, with the version's removed ones where they
    /// were.
    pub changes: Vec<BlockChange>,
    /// How many blocks the page added, changed and removed since the version.
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    /// The page's revision now, which a restore names as its base.
    #[schema(example = "AAAAAAAAAAAAAAAAAAAAAA")]
    pub revision: String,
}

impl From<AppComparison> for Comparison {
    fn from(comparison: AppComparison) -> Self {
        Self {
            page: comparison.page.into(),
            version: comparison.version.into(),
            changes: comparison
                .lines
                .into_iter()
                .map(BlockChange::from)
                .collect(),
            added: comparison.summary.added,
            changed: comparison.summary.changed,
            removed: comparison.summary.removed,
            revision: comparison.revision,
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreVersion {
    /// The revision of the page the version was compared with, as the comparison answered:
    /// once the page is at another, the restore is refused with `edit_conflict`.
    pub base: String,
}

/// What a restore did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RestoredPage {
    pub page: Page,
    /// The version restored.
    pub version: Version,
    /// The page's revision once restored.
    pub revision: Option<String>,
}

impl From<Restored> for RestoredPage {
    fn from(restored: Restored) -> Self {
        Self {
            page: restored.page.into(),
            version: restored.version.into(),
            revision: restored.revision,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListVersions {
    /// Where the part starts: the `next` of the part before.
    pub after: Option<String>,
    /// Versions in the part, 1 to 100; 100 when absent.
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/pages/{id}/versions",
    tag = "history",
    summary = "List a page's versions",
    description = "The page's versions, latest first, a part at a time: each with when its \
                   latest change was made, who changed the page since the version before, and \
                   how many blocks they added, changed and removed. A version is kept each time \
                   the server saves the page changed, and the hourly cleanup thins them out as \
                   the history settings say. The page as it is now, being edited, may be newer \
                   than the latest. A database has no body, and no history.",
    params(("id" = Uuid, Path, description = "The page's id."), ListVersions),
    responses(
        (status = OK, description = "The versions, or a part of them.", body = VersionList),
        (status = BAD_REQUEST, description = "invalid_input: a limit out of bounds, a cursor from no list, or a database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_versions(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListVersions>,
) -> Result<Body<VersionList>, ApiError> {
    let list = state
        .history
        .versions(&principal, id, query.after.as_deref(), query.limit)
        .await?;
    Ok(Body(list.into()))
}

#[utoipa::path(
    get,
    path = "/pages/{id}/versions/{version_id}",
    tag = "history",
    summary = "Read a version",
    description = "A version of the page, read-only: its blocks in BlockNote's JSON and as \
                   markdown with their ids.",
    params(
        ("id" = Uuid, Path, description = "The page's id."),
        ("version_id" = Uuid, Path, description = "The version's id."),
    ),
    responses(
        (status = OK, description = "The version.", body = PageVersion),
        (status = BAD_REQUEST, description = "invalid_input: a database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such page, or no such version of it.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_version(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, version_id)): Path<(Uuid, Uuid)>,
) -> Result<Body<PageVersion>, ApiError> {
    let content = state.history.version(&principal, id, version_id).await?;
    Ok(Body(PageVersion::new(content)?))
}

#[utoipa::path(
    get,
    path = "/pages/{id}/versions/{version_id}/compare",
    tag = "history",
    summary = "Compare a version with the page",
    description = "What changed from the version to the page as it is now, what is being \
                   edited included, block by block: every block of both, in the page's order, \
                   the version's removed ones where they were, each the same, changed, added or \
                   removed. Blocks are told apart by their ids. Answers the revision the page \
                   is at, which a restore names as its base.",
    params(
        ("id" = Uuid, Path, description = "The page's id."),
        ("version_id" = Uuid, Path, description = "The version's id."),
    ),
    responses(
        (status = OK, description = "The comparison.", body = Comparison),
        (status = BAD_REQUEST, description = "invalid_input: a database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such page, or no such version of it.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn compare_version(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, version_id)): Path<(Uuid, Uuid)>,
) -> Result<Body<Comparison>, ApiError> {
    let comparison = state.history.compare(&principal, id, version_id).await?;
    Ok(Body(comparison.into()))
}

#[utoipa::path(
    post,
    path = "/pages/{id}/versions/{version_id}/restore",
    tag = "history",
    summary = "Restore a version",
    description = "Makes the page hold the version's blocks again, as one change through its \
                   live document: whoever has it open sees it at once. Blocks the version and \
                   the page share stay as they are, so what is typed in them meanwhile stays. \
                   The page as it was is kept as a version first, so restoring that one undoes \
                   it. base is the revision the comparison answered: once the page is at \
                   another, as when someone edited it since, nothing changes and the answer is \
                   edit_conflict, to compare again. Needs edit access to the page, out of the \
                   trash, and counts as an edit of its content.",
    params(
        ("id" = Uuid, Path, description = "The page's id."),
        ("version_id" = Uuid, Path, description = "The version's id."),
    ),
    request_body = RestoreVersion,
    responses(
        (status = OK, description = "The page restored.", body = RestoredPage),
        (status = BAD_REQUEST, description = "invalid_input: a database.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the page and does not edit it, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such page, or no such version of it.", body = ErrorBody),
        (status = CONFLICT, description = "edit_conflict: the page is no longer at base; nothing changed.", body = ErrorBody),
        (status = PAYLOAD_TOO_LARGE, description = "document_too_large: the page would grow past what it holds.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited: too many edits of pages' content; Retry-After says when to try again.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "unavailable: the page could not be saved first, or the server is stopping.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn restore_version(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, version_id)): Path<(Uuid, Uuid)>,
    Body(body): Body<RestoreVersion>,
) -> Result<Body<RestoredPage>, ApiError> {
    let restored = state
        .history
        .restore(&principal, id, version_id, body.base)
        .await?;
    Ok(Body(restored.into()))
}
