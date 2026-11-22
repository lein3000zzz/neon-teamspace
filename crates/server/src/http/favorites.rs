//! `/api/v1/account/favorites`: the caller's favorite pages, in the order they keep them.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. A favorite the caller may no longer see, or one in the trash, stays saved but is
//! left out of the list, and comes back with the access.

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::pages::{Page, Placement};
use super::principal::Authenticated;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FavoriteList {
    /// In the caller's order.
    pub pages: Vec<Page>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListFavorites {
    /// Only the favorites of this workspace; those of every workspace when absent.
    pub workspace_id: Option<Uuid>,
}

#[utoipa::path(
    get,
    path = "/account/favorites",
    tag = "account",
    summary = "List the caller's favorite pages",
    description = "Those they may see and that are out of the trash, in their order.",
    params(ListFavorites),
    responses(
        (status = OK, description = "The pages.", body = FavoriteList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_favorites(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Query(query): Query<ListFavorites>,
) -> Result<Json<FavoriteList>, ApiError> {
    let pages = state
        .pages
        .favorites(&principal, query.workspace_id)
        .await?;
    Ok(Json(FavoriteList {
        pages: pages.into_iter().map(Page::from).collect(),
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddFavorite {
    pub page_id: Uuid,
    /// Among the caller's favorites, `after` and `before` naming another favorite page;
    /// `"last"` when absent.
    #[serde(default)]
    pub placement: Option<Placement>,
}

#[utoipa::path(
    post,
    path = "/account/favorites",
    tag = "account",
    summary = "Add a favorite page",
    description = "A page the caller sees, out of the trash, placed among their favorites.",
    request_body = AddFavorite,
    responses(
        (status = CREATED, description = "The page.", body = Page),
        (status = BAD_REQUEST, description = "invalid_input: a placement beside a page that is not a favorite.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "already_favorite.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn add_favorite(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Json(body): Json<AddFavorite>,
) -> Result<(StatusCode, Json<Page>), ApiError> {
    let placement = body.placement.map(Into::into).unwrap_or_default();
    let page = state
        .pages
        .add_favorite(&principal, body.page_id, placement)
        .await?;
    Ok((StatusCode::CREATED, Json(page.into())))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveFavorite {
    /// Among the caller's other favorites, `after` and `before` naming one of them.
    pub placement: Placement,
}

#[utoipa::path(
    post,
    path = "/account/favorites/{page_id}/move",
    tag = "account",
    summary = "Reorder a favorite",
    params(("page_id" = Uuid, Path, description = "The favorite page's id.")),
    request_body = MoveFavorite,
    responses(
        (status = NO_CONTENT, description = "Moved."),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: not among the caller's favorites.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn move_favorite(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(page_id): Path<Uuid>,
    Json(body): Json<MoveFavorite>,
) -> Result<StatusCode, ApiError> {
    state
        .pages
        .move_favorite(&principal, page_id, body.placement.into())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/account/favorites/{page_id}",
    tag = "account",
    summary = "Remove a favorite",
    params(("page_id" = Uuid, Path, description = "The favorite page's id.")),
    responses(
        (status = NO_CONTENT, description = "Removed."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: not among the caller's favorites.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn remove_favorite(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(page_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.pages.remove_favorite(&principal, page_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
