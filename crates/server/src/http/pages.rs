//! `/api/v1/pages`, a workspace's pages and trees, and its trash.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. Who may see and change a page is the rules' to say; a page someone may not see
//! answers `not_found`, as one that does not exist would.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::pages::{
    Location, NewPage, PageUpdate, PageView, TrashList as AppTrashList, Tree,
    TreePage as AppTreePage,
};
use neon_domain::pages as domain;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::dto::nullable;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::principal::Authenticated;

/// What the caller may do with a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageAccess {
    /// Read it and the pages under it.
    View,
    /// Also change it, add pages under it, move it and trash it.
    Edit,
}

impl From<domain::PageAccess> for PageAccess {
    fn from(access: domain::PageAccess) -> Self {
        match access {
            domain::PageAccess::View => Self::View,
            domain::PageAccess::Edit => Self::Edit,
        }
    }
}

/// The font a page is set in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Font {
    /// The system's sans-serif.
    Default,
    /// The system's serif.
    Serif,
    /// The bundled monospaced font.
    Mono,
}

impl From<domain::Font> for Font {
    fn from(font: domain::Font) -> Self {
        match font {
            domain::Font::Default => Self::Default,
            domain::Font::Serif => Self::Serif,
            domain::Font::Mono => Self::Mono,
        }
    }
}

impl From<Font> for domain::Font {
    fn from(font: Font) -> Self {
        match font {
            Font::Default => Self::Default,
            Font::Serif => Self::Serif,
            Font::Mono => Self::Mono,
        }
    }
}

/// A cover drawn from the theme's tokens: a hue, or a gradient between two, at the
/// lightness and chroma of the reader's palette and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Cover {
    /// The reader's accent.
    Accent,
    /// From the reader's accent to the hue opposite it.
    AccentGradient,
    /// Hue 25.
    Red,
    /// Hue 55.
    Orange,
    /// Hue 90.
    Yellow,
    /// Hue 145.
    Green,
    /// Hue 185.
    Teal,
    /// Hue 250.
    Blue,
    /// Hue 295.
    Violet,
    /// Hue 345.
    Pink,
    /// From hue 55 to hue 345.
    Sunrise,
    /// From hue 185 to hue 250.
    Lagoon,
    /// From hue 145 to hue 295.
    Aurora,
    /// From hue 250 to hue 345.
    Dusk,
}

impl From<domain::Cover> for Cover {
    fn from(cover: domain::Cover) -> Self {
        match cover {
            domain::Cover::Accent => Self::Accent,
            domain::Cover::AccentGradient => Self::AccentGradient,
            domain::Cover::Red => Self::Red,
            domain::Cover::Orange => Self::Orange,
            domain::Cover::Yellow => Self::Yellow,
            domain::Cover::Green => Self::Green,
            domain::Cover::Teal => Self::Teal,
            domain::Cover::Blue => Self::Blue,
            domain::Cover::Violet => Self::Violet,
            domain::Cover::Pink => Self::Pink,
            domain::Cover::Sunrise => Self::Sunrise,
            domain::Cover::Lagoon => Self::Lagoon,
            domain::Cover::Aurora => Self::Aurora,
            domain::Cover::Dusk => Self::Dusk,
        }
    }
}

impl From<Cover> for domain::Cover {
    fn from(cover: Cover) -> Self {
        match cover {
            Cover::Accent => Self::Accent,
            Cover::AccentGradient => Self::AccentGradient,
            Cover::Red => Self::Red,
            Cover::Orange => Self::Orange,
            Cover::Yellow => Self::Yellow,
            Cover::Green => Self::Green,
            Cover::Teal => Self::Teal,
            Cover::Blue => Self::Blue,
            Cover::Violet => Self::Violet,
            Cover::Pink => Self::Pink,
            Cover::Sunrise => Self::Sunrise,
            Cover::Lagoon => Self::Lagoon,
            Cover::Aurora => Self::Aurora,
            Cover::Dusk => Self::Dusk,
        }
    }
}

/// How a page is set.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct PageStyle {
    pub font: Font,
    pub small_text: bool,
    pub full_width: bool,
}

/// Where a page goes among its siblings: `"first"`, `"last"`, `{"after": id}` or
/// `{"before": id}`, the id being a sibling's.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    First,
    Last,
    After(Uuid),
    Before(Uuid),
}

impl From<Placement> for domain::Placement {
    fn from(placement: Placement) -> Self {
        match placement {
            Placement::First => Self::First,
            Placement::Last => Self::Last,
            Placement::After(id) => Self::After(id),
            Placement::Before(id) => Self::Before(id),
        }
    }
}

/// The parts of a page's style to set; each left out stays as it is, or takes its default
/// on a new page.
#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleChange {
    #[serde(default)]
    pub font: Option<Font>,
    #[serde(default)]
    pub small_text: Option<bool>,
    #[serde(default)]
    pub full_width: Option<bool>,
}

/// A page, without its content, and what the caller may do with it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Page {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The teamspace it is in; absent for a private page.
    pub teamspace_id: Option<Uuid>,
    /// The member a private page belongs to; absent for a teamspace's page, and for
    /// guests, who know no other member.
    pub owner_id: Option<Uuid>,
    /// The page it is under; absent at the top of its space, and when the caller may not
    /// see that page.
    pub parent_id: Option<Uuid>,
    /// Its key among its siblings, which sort by it, byte by byte.
    #[schema(example = "a0")]
    pub position: String,
    /// Up to 500 characters; empty shows as Untitled.
    #[schema(example = "Roadmap")]
    pub title: String,
    pub icon: Option<String>,
    pub cover: Option<Cover>,
    pub style: PageStyle,
    pub access: PageAccess,
    /// Absent for guests, as are `last_edited_by` and `trashed_by`.
    pub created_by: Option<Uuid>,
    /// Who changed it last, its content included.
    pub last_edited_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When its title, icon, cover, style or place last changed.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// When its content last changed.
    #[serde(with = "time::serde::rfc3339")]
    pub content_updated_at: OffsetDateTime,
    /// Set while it is in the trash.
    #[serde(with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,
    pub trashed_by: Option<Uuid>,
    /// While it is in the trash: the page trashed with it, itself when it was trashed on
    /// its own. Restoring that page brings this one back too.
    pub trashed_with: Option<Uuid>,
}

impl From<PageView> for Page {
    fn from(view: PageView) -> Self {
        let page = view.page;
        let member = |id: Option<Uuid>| id.filter(|_| !view.members_hidden);
        Self {
            id: page.id,
            workspace_id: page.workspace_id,
            teamspace_id: page.space.teamspace_id(),
            owner_id: member(page.space.owner_id()),
            parent_id: page.parent_id,
            position: page.position,
            title: page.title,
            icon: page.icon,
            cover: page.cover.map(Into::into),
            style: PageStyle {
                font: page.style.font.into(),
                small_text: page.style.small_text,
                full_width: page.style.full_width,
            },
            access: view.access.into(),
            created_by: member(page.created_by),
            last_edited_by: member(page.last_edited_by),
            created_at: page.created_at,
            updated_at: page.updated_at,
            content_updated_at: page.content_updated_at,
            trashed_at: page.trashed_at,
            trashed_by: member(page.trashed_by),
            trashed_with: page.trashed_with,
        }
    }
}

/// The one place a page is made in, moved to or imported to.
pub(super) fn location(
    parent_id: Option<Uuid>,
    teamspace_id: Option<Uuid>,
    private: bool,
) -> Result<Location, ApiError> {
    match (parent_id, teamspace_id, private) {
        (Some(parent), None, false) => Ok(Location::Under(parent)),
        (None, Some(teamspace), false) => Ok(Location::Teamspace(teamspace)),
        (None, None, true) => Ok(Location::Private),
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "name one place for the page: parent_id, teamspace_id, or private: true",
        )),
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePage {
    /// The page to put it under, in this workspace. Name this, `teamspace_id` or `private`.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// The teamspace to put it at the top of.
    #[serde(default)]
    pub teamspace_id: Option<Uuid>,
    /// True to make it one of the caller's private pages.
    #[serde(default)]
    pub private: bool,
    /// `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
    /// Up to 500 characters; empty when absent.
    #[serde(default)]
    #[schema(example = "Roadmap")]
    pub title: String,
    /// An emoji or up to 16 characters of text.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub cover: Option<Cover>,
    #[serde(default)]
    pub style: StyleChange,
}

#[utoipa::path(
    post,
    path = "/workspaces/{id}/pages",
    tag = "pages",
    summary = "Create a page",
    description = "Under a page the caller edits, at the top of a teamspace they are in, or as \
                   one of their private pages, which guests have none of. The caller edits it.",
    params(("id" = Uuid, Path, description = "The workspace's id.")),
    request_body = CreatePage,
    responses(
        (status = CREATED, description = "The page.", body = Page),
        (status = BAD_REQUEST, description = "invalid_input: no place or several, a title or icon out of bounds, or a placement beside a page that is not a sibling.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the parent and does not edit it, is not in the teamspace, or is a guest making a private page; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace, teamspace or parent, or none the caller may see.", body = ErrorBody),
        (status = CONFLICT, description = "too_deep: pages nest at most 100 levels.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn create_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreatePage>,
) -> Result<(StatusCode, Json<Page>), ApiError> {
    let location = location(body.parent_id, body.teamspace_id, body.private)?;
    let defaults = domain::PageStyle::default();
    let new = NewPage {
        title: body.title,
        icon: body.icon,
        cover: body.cover.map(Into::into),
        style: domain::PageStyle {
            font: body.style.font.map_or(defaults.font, Into::into),
            small_text: body.style.small_text.unwrap_or(defaults.small_text),
            full_width: body.style.full_width.unwrap_or(defaults.full_width),
        },
    };
    let placement = body.placement.map(Into::into).unwrap_or_default();
    let created = state
        .pages
        .create_page(&principal, id, location, placement, new)
        .await?;
    Ok((StatusCode::CREATED, Json(created.into())))
}

#[utoipa::path(
    get,
    path = "/pages/{id}",
    tag = "pages",
    summary = "A page",
    description = "Its title, icon, cover, style and place, and what the caller may do with it. \
                   A page in the trash shows only to those who could edit it.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = OK, description = "The page.", body = Page),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<Page>, ApiError> {
    Ok(Json(state.pages.page(&principal, id).await?.into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePage {
    #[serde(default)]
    pub title: Option<String>,
    /// Null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub icon: Option<Option<String>>,
    /// Null removes it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<Cover>)]
    pub cover: Option<Option<Cover>>,
    #[serde(default)]
    pub style: StyleChange,
}

#[utoipa::path(
    patch,
    path = "/pages/{id}",
    tag = "pages",
    summary = "Change a page",
    description = "Changes what the body names, for those who edit the page, out of the trash.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    request_body = UpdatePage,
    responses(
        (status = OK, description = "The page.", body = Page),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views it and does not edit it, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn update_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePage>,
) -> Result<Json<Page>, ApiError> {
    let update = PageUpdate {
        title: body.title,
        icon: body.icon,
        cover: body.cover.map(|cover| cover.map(Into::into)),
        font: body.style.font.map(Into::into),
        small_text: body.style.small_text,
        full_width: body.style.full_width,
    };
    Ok(Json(
        state
            .pages
            .update_page(&principal, id, update)
            .await?
            .into(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MovePage {
    /// The page to move it under, in the same workspace. Name this, `teamspace_id` or
    /// `private`.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    /// The teamspace of the workspace to move it to the top of.
    #[serde(default)]
    pub teamspace_id: Option<Uuid>,
    /// True to move it to the top of the caller's private pages.
    #[serde(default)]
    pub private: bool,
    /// `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
}

#[utoipa::path(
    post,
    path = "/pages/{id}/move",
    tag = "pages",
    summary = "Move a page",
    description = "Moves it, with the pages under it, under another page, to the top of a \
                   teamspace or to the caller's private pages, and places it among its new \
                   siblings; to reorder it, name the place it is in. The caller edits the page \
                   and where it goes. Out of its space, to another teamspace or to private \
                   pages, the caller must edit it through the space itself, being in its \
                   teamspace or owning it: a page shared with them moves only within its space. \
                   The pages moved take their access from their new place.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    request_body = MovePage,
    responses(
        (status = OK, description = "The page.", body = Page),
        (status = BAD_REQUEST, description = "invalid_input: no place or several, a place in another workspace, or a placement beside a page that is not a sibling.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller views the page or its new place and does not edit it, or edits the page through a share only and names another space; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: the page or its new place.", body = ErrorBody),
        (status = CONFLICT, description = "cycle: under itself or one of its subpages; or too_deep.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn move_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<MovePage>,
) -> Result<Json<Page>, ApiError> {
    let to = location(body.parent_id, body.teamspace_id, body.private)?;
    let placement = body.placement.map(Into::into).unwrap_or_default();
    Ok(Json(
        state
            .pages
            .move_page(&principal, id, to, placement)
            .await?
            .into(),
    ))
}

#[utoipa::path(
    post,
    path = "/pages/{id}/trash",
    tag = "pages",
    summary = "Move a page to the trash",
    description = "With the pages under it, for those who edit it. It stays there for \
                   `pages.trash_retention`, 30 days unless set otherwise, then is deleted for good.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = OK, description = "The page, in the trash.", body = Page),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller does not edit it, or it is in the trash already; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn trash_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<Page>, ApiError> {
    Ok(Json(state.pages.trash_page(&principal, id).await?.into()))
}

#[utoipa::path(
    post,
    path = "/pages/{id}/restore",
    tag = "pages",
    summary = "Restore a page from the trash",
    description = "With the pages trashed along with it. It goes back under its parent, unless \
                   that is in the trash too: then to the top of its space, which the caller must \
                   be allowed to add pages to.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = OK, description = "The page.", body = Page),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: it is not in the trash, or its parent is and the caller may not add pages at the top of its space; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn restore_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<Page>, ApiError> {
    Ok(Json(state.pages.restore_page(&principal, id).await?.into()))
}

#[utoipa::path(
    delete,
    path = "/pages/{id}",
    tag = "pages",
    summary = "Delete a page for good",
    description = "Only a page in the trash, with everything under it, for those who could edit \
                   it. It cannot be undone.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: it is not in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.pages.delete_page(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A page on the way down to another.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Crumb {
    pub id: Uuid,
    pub title: String,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Breadcrumbs {
    /// The highest page above it the caller sees first, the page itself last.
    pub pages: Vec<Crumb>,
}

#[utoipa::path(
    get,
    path = "/pages/{id}/breadcrumbs",
    tag = "pages",
    summary = "The way down to a page",
    description = "The pages above it that the caller sees, from the highest, then the page. A \
                   page shared with a guest starts their way down.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = OK, description = "The pages.", body = Breadcrumbs),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn page_breadcrumbs(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<Breadcrumbs>, ApiError> {
    let pages = state.pages.breadcrumbs(&principal, id).await?;
    Ok(Json(Breadcrumbs {
        pages: pages
            .into_iter()
            .map(|page| Crumb {
                id: page.id,
                title: page.title,
                icon: page.icon,
            })
            .collect(),
    }))
}

/// A page in a tree, as a sidebar shows it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TreePage {
    pub id: Uuid,
    /// The page it is under; absent at the top of its space, or of the pages shared with
    /// the caller.
    pub parent_id: Option<Uuid>,
    pub teamspace_id: Option<Uuid>,
    /// The member a private page belongs to; absent for a teamspace's page, and for
    /// guests, who know no other member.
    pub owner_id: Option<Uuid>,
    pub title: String,
    pub icon: Option<String>,
    /// How many pages are under it, out of the trash.
    pub child_count: i64,
    pub access: PageAccess,
}

impl From<AppTreePage> for TreePage {
    fn from(page: AppTreePage) -> Self {
        Self {
            id: page.id,
            parent_id: page.parent_id,
            teamspace_id: page.space.teamspace_id(),
            owner_id: page.space.owner_id().filter(|_| !page.members_hidden),
            title: page.title,
            icon: page.icon,
            child_count: page.child_count,
            access: page.access.into(),
        }
    }
}

/// A part of a tree.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PageTree {
    /// Each page, then the pages under it, siblings in order: a page's parent comes before
    /// it, in this part or an earlier one.
    pub pages: Vec<TreePage>,
    /// Pass it as `after` for the next part; absent on the last one.
    pub next: Option<String>,
}

impl From<Tree> for PageTree {
    fn from(tree: Tree) -> Self {
        Self {
            pages: tree.pages.into_iter().map(TreePage::from).collect(),
            next: tree.next,
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListPages {
    /// Pages in the part, 1 to 1000; 1000 when absent.
    pub limit: Option<i64>,
    /// The `next` of the part before.
    pub after: Option<String>,
}

#[utoipa::path(
    get,
    path = "/teamspaces/{id}/pages",
    tag = "pages",
    summary = "List a teamspace's pages",
    description = "Its pages out of the trash, as a tree, for everyone who may see the \
                   teamspace in full: a private one's for its members only. Guests get \
                   `not_found`; their pages are under `shared-pages`.",
    params(("id" = Uuid, Path, description = "The teamspace's id."), ListPages),
    responses(
        (status = OK, description = "The tree, or a part of it.", body = PageTree),
        (status = BAD_REQUEST, description = "invalid_input: a limit or cursor out of bounds.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn teamspace_pages(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPages>,
) -> Result<Json<PageTree>, ApiError> {
    let tree = state
        .pages
        .teamspace_tree(&principal, id, query.after.as_deref(), query.limit)
        .await?;
    Ok(Json(tree.into()))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/private-pages",
    tag = "pages",
    summary = "List the caller's private pages",
    description = "Their private pages in the workspace, out of the trash, as a tree. Guests \
                   have none, and get `forbidden`.",
    params(("id" = Uuid, Path, description = "The workspace's id."), ListPages),
    responses(
        (status = OK, description = "The tree, or a part of it.", body = PageTree),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: a guest; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn private_pages(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPages>,
) -> Result<Json<PageTree>, ApiError> {
    let tree = state
        .pages
        .private_tree(&principal, id, query.after.as_deref(), query.limit)
        .await?;
    Ok(Json(tree.into()))
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/shared-pages",
    tag = "pages",
    summary = "List the pages shared with the caller",
    description = "The pages of the workspace shared with the caller, out of the trash, each with \
                   the pages under it, as a tree: how guests find their pages. A shared page under \
                   another one shared with them shows under that one only. The pages shared are in \
                   the order they were made.",
    params(("id" = Uuid, Path, description = "The workspace's id."), ListPages),
    responses(
        (status = OK, description = "The tree, or a part of it.", body = PageTree),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn shared_pages(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPages>,
) -> Result<Json<PageTree>, ApiError> {
    let tree = state
        .pages
        .shared_tree(&principal, id, query.after.as_deref(), query.limit)
        .await?;
    Ok(Json(tree.into()))
}

/// A part of a workspace's trash.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrashList {
    /// Latest first. A page trashed with the pages under it stands for them.
    pub pages: Vec<Page>,
    /// Pass it as `after` for the next part; absent on the last one.
    pub next: Option<String>,
}

impl From<AppTrashList> for TrashList {
    fn from(list: AppTrashList) -> Self {
        Self {
            pages: list.pages.into_iter().map(Page::from).collect(),
            next: list.next,
        }
    }
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/trash",
    tag = "pages",
    summary = "List the workspace's trash",
    description = "The pages in the trash that the caller could edit, latest first: those trashed \
                   on their own, each standing for the pages that went with it.",
    params(("id" = Uuid, Path, description = "The workspace's id."), ListPages),
    responses(
        (status = OK, description = "The trash, or a part of it.", body = TrashList),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn workspace_trash(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPages>,
) -> Result<Json<TrashList>, ApiError> {
    let list = state
        .pages
        .trash(&principal, id, query.after.as_deref(), query.limit)
        .await?;
    Ok(Json(list.into()))
}
