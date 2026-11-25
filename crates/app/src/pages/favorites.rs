//! Each account's favorite pages, in the order it keeps them. A favorite the account may
//! no longer see, or one in the trash, stays saved but is left out of the list, so it
//! comes back with the access. A token bound to one workspace keeps to that workspace's
//! favorites: it lists, places, moves and removes those alone, and the others are not
//! there for it.

use std::collections::HashMap;

use neon_domain::auth::Principal;
use neon_domain::pages::Placement;
use neon_domain::workspaces::Action;
use neon_store::favorites::{self as repo, ALREADY_FAVORITE};
use neon_store::is_unique_violation;
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{Facts, PageError, PageService, PageView, hide_parents, key_in, renumbered};

const NOT_FAVORITE: &str = "this page is not among your favorites";

impl PageService {
    /// The caller's favorite pages they may see and that are out of the trash, in their
    /// order, in every workspace or in one.
    pub async fn favorites(
        &self,
        principal: &Principal,
        workspace_id: Option<Uuid>,
    ) -> Result<Vec<PageView>, PageError> {
        self.observe("list_favorites", async {
            let user = principal.user.id;
            if workspace_id.is_some_and(|asked| !principal.reaches(asked)) {
                return Ok(Vec::new());
            }
            let mut conn = self.pool().acquire().await?;
            let scope = workspace_id.or(principal.workspace());
            let order = repo::positions(&mut *conn, user, scope, None).await?;
            let ids: Vec<Uuid> = order.iter().map(|(id, _)| *id).collect();
            let mut seen: HashMap<Uuid, Option<PageView>> = Facts::read(&mut conn, principal, &ids)
                .await?
                .seen(principal)?;
            let mut list = Vec::new();
            for id in ids {
                if let Some(view) = seen.remove(&id).flatten()
                    && !view.page.is_trashed()
                {
                    list.push(view);
                }
            }
            hide_parents(&mut conn, principal, &mut list).await?;
            Ok(list)
        })
        .await
    }

    /// Adds a page the caller sees to their favorites, placed among them.
    pub async fn add_favorite(
        &self,
        principal: &Principal,
        page_id: Uuid,
        placement: Placement,
    ) -> Result<PageView, PageError> {
        self.observe("add_favorite", async {
            let user = principal.user.id;
            let mut tx = self.pool().begin().await?;
            let context = self.context(&mut tx, principal, page_id).await?;
            context.check(Action::ViewPage)?;
            if context.page().is_trashed() {
                return Err(PageError::Forbidden(
                    "this page is in the trash: restore it first",
                ));
            }
            repo::lock(&mut *tx, user).await?;
            let position = self
                .favorite_position(&mut tx, principal, None, placement)
                .await?;
            let workspace_id = context.page().workspace_id;
            repo::insert(&mut *tx, user, page_id, workspace_id, &position, self.now())
                .await
                .map_err(|error| {
                    if is_unique_violation(&error, ALREADY_FAVORITE) {
                        PageError::AlreadyFavorite
                    } else {
                        PageError::Database(error)
                    }
                })?;
            tx.commit().await?;
            info!(user_id = %user, page_id = %page_id, "favorite added");
            context.view()
        })
        .await
    }

    /// Moves one of the caller's favorites among the others.
    pub async fn move_favorite(
        &self,
        principal: &Principal,
        page_id: Uuid,
        placement: Placement,
    ) -> Result<(), PageError> {
        self.observe("move_favorite", async {
            let user = principal.user.id;
            let mut tx = self.pool().begin().await?;
            repo::lock(&mut *tx, user).await?;
            let all = repo::positions(&mut *tx, user, principal.workspace(), None).await?;
            if !all.iter().any(|(id, _)| *id == page_id) {
                return Err(PageError::NotFound(NOT_FAVORITE));
            }
            let position = self
                .favorite_position(&mut tx, principal, Some(page_id), placement)
                .await?;
            repo::set_positions(&mut *tx, user, &[page_id], &[position]).await?;
            tx.commit().await?;
            info!(user_id = %user, page_id = %page_id, "favorite moved");
            Ok(())
        })
        .await
    }

    /// Takes a page off the caller's favorites.
    pub async fn remove_favorite(
        &self,
        principal: &Principal,
        page_id: Uuid,
    ) -> Result<(), PageError> {
        self.observe("remove_favorite", async {
            let user = principal.user.id;
            if !repo::delete(self.pool(), user, page_id, principal.workspace()).await? {
                return Err(PageError::NotFound(NOT_FAVORITE));
            }
            info!(user_id = %user, page_id = %page_id, "favorite removed");
            Ok(())
        })
        .await
    }

    /// The position for a favorite placed among the account's others but for `except`,
    /// those of its token's workspace alone for a bound token, after fresh keys for them
    /// all when the key would grow too long. Run it holding the account's favorites lock.
    async fn favorite_position(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        except: Option<Uuid>,
        placement: Placement,
    ) -> Result<String, PageError> {
        let user = principal.user.id;
        let list = repo::positions(&mut *conn, user, principal.workspace(), except).await?;
        if let Some(key) = key_in(&list, placement)? {
            return Ok(key);
        }
        let list = renumbered(&list);
        let (ids, keys): (Vec<Uuid>, Vec<String>) = list.iter().cloned().unzip();
        repo::set_positions(&mut *conn, user, &ids, &keys).await?;
        key_in(&list, placement)?
            .ok_or_else(|| PageError::Internal("fresh favorite positions are too long".to_owned()))
    }
}
