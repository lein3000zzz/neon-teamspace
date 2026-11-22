//! `/api/v1/pages/{id}/shares`: who a page is shared with, to view or to edit it and every
//! page under it.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. Guests, who know no other member, neither list shares nor share.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::pages::ListedShare;
use neon_domain::pages as domain;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::dto::Person;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path};
use super::principal::Authenticated;

/// What a share gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShareAccess {
    CanView,
    CanEdit,
}

impl From<domain::PageAccess> for ShareAccess {
    fn from(access: domain::PageAccess) -> Self {
        match access {
            domain::PageAccess::View => Self::CanView,
            domain::PageAccess::Edit => Self::CanEdit,
        }
    }
}

impl From<ShareAccess> for domain::PageAccess {
    fn from(access: ShareAccess) -> Self {
        match access {
            ShareAccess::CanView => Self::View,
            ShareAccess::CanEdit => Self::Edit,
        }
    }
}

/// A page shared with a member or guest.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Share {
    pub user: Person,
    pub access: ShareAccess,
    /// The page it is set on: the one listed, or a page above it; absent when that is a
    /// page above that the caller may not see.
    pub page_id: Option<Uuid>,
    /// Set on a page above the one listed, and holding here too; it changes there.
    pub inherited: bool,
    pub created_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl From<ListedShare> for Share {
    fn from(listed: ListedShare) -> Self {
        Self {
            user: listed.person.into(),
            access: listed.share.access.into(),
            page_id: listed.set_on,
            inherited: listed.inherited,
            created_by: listed.share.created_by,
            created_at: listed.share.created_at,
            updated_at: listed.share.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ShareList {
    /// The page's own first, then those of each page above it, each page's by name.
    pub shares: Vec<Share>,
}

#[utoipa::path(
    get,
    path = "/pages/{id}/shares",
    tag = "pages",
    summary = "List who a page is shared with",
    description = "The shares on the page and on every page above it, which hold for it too. For \
                   everyone who sees the page but guests.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    responses(
        (status = OK, description = "The shares.", body = ShareList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: a guest; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_page_shares(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<Json<ShareList>, ApiError> {
    let shares = state.pages.shares(&principal, id).await?;
    Ok(Json(ShareList {
        shares: shares.into_iter().map(Share::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateShare {
    /// A member or guest of the page's workspace, other than the caller.
    pub user_id: Uuid,
    pub access: ShareAccess,
}

#[utoipa::path(
    post,
    path = "/pages/{id}/shares",
    tag = "pages",
    summary = "Share a page",
    description = "With a member or guest of its workspace, for the page and every page under it, \
                   by those who edit it, guests aside. It adds to what the person may do there \
                   already, never takes away.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    request_body = CreateShare,
    responses(
        (status = CREATED, description = "The share.", body = Share),
        (status = BAD_REQUEST, description = "invalid_input: shared with the caller.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller does not edit it, is a guest, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such page, or the person is not in its workspace.", body = ErrorBody),
        (status = CONFLICT, description = "already_shared: change that share instead.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn share_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateShare>,
) -> Result<(StatusCode, Json<Share>), ApiError> {
    let share = state
        .pages
        .share_page(&principal, id, body.user_id, body.access.into())
        .await?;
    Ok((StatusCode::CREATED, Json(share.into())))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeShare {
    pub access: ShareAccess,
}

#[utoipa::path(
    patch,
    path = "/pages/{id}/shares/{user_id}",
    tag = "pages",
    summary = "Change what a share gives",
    description = "A share set on this page; an inherited one changes on its own page.",
    params(
        ("id" = Uuid, Path, description = "The page's id."),
        ("user_id" = Uuid, Path, description = "The account it is shared with."),
    ),
    request_body = ChangeShare,
    responses(
        (status = OK, description = "The share.", body = Share),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such page, or no share of it with that account.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_page_share(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ChangeShare>,
) -> Result<Json<Share>, ApiError> {
    let share = state
        .pages
        .change_share(&principal, id, user_id, body.access.into())
        .await?;
    Ok(Json(share.into()))
}

#[utoipa::path(
    delete,
    path = "/pages/{id}/shares/{user_id}",
    tag = "pages",
    summary = "Remove a share",
    description = "A share set on this page; an inherited one goes from its own page. The person \
                   keeps what else lets them see the page.",
    params(
        ("id" = Uuid, Path, description = "The page's id."),
        ("user_id" = Uuid, Path, description = "The account it is shared with."),
    ),
    responses(
        (status = NO_CONTENT, description = "Removed."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn remove_page_share(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.pages.remove_share(&principal, id, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
