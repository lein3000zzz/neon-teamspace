//! Each account's favorite pages, in the order it keeps them. A favorite the account may
//! no longer see, or one in the trash, stays saved but is left out of the list, so it
//! comes back with the access.

use std::collections::HashMap;

use neon_domain::auth::Principal;
use neon_domain::pages::Placement;
use neon_domain::workspaces::Action;
use neon_store::favorites::{self as repo, ALREADY_FAVORITE};
use neon_store::is_unique_violation;
use neon_store::pages;
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{PageError, PageService, PageView, key_in, renumbered, seen};

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
            let mut conn = self.pool().acquire().await?;
            let order = repo::positions(&mut *conn, user, workspace_id, None).await?;
            let ids: Vec<Uuid> = order.iter().map(|(id, _)| *id).collect();
            let facts: HashMap<_, _> = pages::facts(&mut *conn, &ids, user)
                .await?
                .into_iter()
                .map(|facts| (facts.page.id, facts))
                .collect();
            let mut list = Vec::new();
            for id in ids {
                let Some(facts) = facts.get(&id) else {
                    continue;
                };
                if let Some(view) = seen(principal, facts)?
                    && !view.page.is_trashed()
                {
                    list.push(view);
                }
            }
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
                .favorite_position(&mut tx, user, None, placement)
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
            let all = repo::positions(&mut *tx, user, None, None).await?;
            if !all.iter().any(|(id, _)| *id == page_id) {
                return Err(PageError::NotFound(NOT_FAVORITE));
            }
            let position = self
                .favorite_position(&mut tx, user, Some(page_id), placement)
                .await?;
            repo::set_positions(&mut *tx, user, &[page_id], &[position]).await?;
            tx.commit().await?;
            info!(user_id = %user, page_id = %page_id, "favorite moved");
            Ok(())
        })
        .await
    }

    pub async fn remove_favorite(
        &self,
        principal: &Principal,
        page_id: Uuid,
    ) -> Result<(), PageError> {
        self.observe("remove_favorite", async {
            let user = principal.user.id;
            if !repo::delete(self.pool(), user, page_id).await? {
                return Err(PageError::NotFound(NOT_FAVORITE));
            }
            info!(user_id = %user, page_id = %page_id, "favorite removed");
            Ok(())
        })
        .await
    }

    /// The position for a favorite placed among the account's others but for `except`,
    /// after fresh keys for them all when the key would grow too long. Run it holding the
    /// account's favorites lock.
    async fn favorite_position(
        &self,
        conn: &mut PgConnection,
        user: Uuid,
        except: Option<Uuid>,
        placement: Placement,
    ) -> Result<String, PageError> {
        let list = repo::positions(&mut *conn, user, None, except).await?;
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
