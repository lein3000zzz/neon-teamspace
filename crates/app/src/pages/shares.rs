//! Sharing a page with a member or guest of its workspace, to view or to edit it and every
//! page under it.

use neon_domain::auth::Principal;
use neon_domain::pages::{PageAccess, Share};
use neon_domain::workspaces::{Action, Person};
use neon_store::page_shares::{self as repo, ALREADY_SHARED};
use neon_store::{is_unique_violation, workspaces};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::{AccessChange, Context, PageError, PageService};

const NO_MEMBER: &str = "no such member: a page is shared with people in its workspace";
const NO_SHARE: &str = "the page itself is not shared with this person";

/// A share as a page's list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedShare {
    pub share: Share,
    pub person: Person,
    /// Set on a page above the one listed, so it holds here too; changed or removed there.
    pub inherited: bool,
    /// The page it is set on, unless that is a page above that the caller may not see:
    /// then only that it is inherited shows.
    pub set_on: Option<Uuid>,
}

impl PageService {
    /// Who a page is shared with: on the page itself first, then on each page above it.
    /// For everyone who sees the page but guests, who know no other member. A share set on
    /// a page above that the caller may not see does not say which page.
    pub async fn shares(
        &self,
        principal: &Principal,
        page_id: Uuid,
    ) -> Result<Vec<ListedShare>, PageError> {
        self.observe("list_shares", async {
            let mut conn = self.pool().acquire().await?;
            let context = self.context(&mut conn, principal, page_id).await?;
            context.check(Action::ListShares)?;
            Ok(repo::on_chain(&mut *conn, page_id)
                .await?
                .into_iter()
                .map(|listed| {
                    let seen = usize::try_from(listed.depth)
                        .ok()
                        .and_then(|depth| context.chain.get(depth))
                        .is_some_and(|link| context.sees(link));
                    ListedShare {
                        inherited: listed.depth > 0,
                        set_on: seen.then_some(listed.share.page_id),
                        share: listed.share,
                        person: listed.person,
                    }
                })
                .collect())
        })
        .await
    }

    /// Shares a page with a member or guest of its workspace, for those who edit it, but
    /// guests.
    pub async fn share_page(
        &self,
        principal: &Principal,
        page_id: Uuid,
        user_id: Uuid,
        access: PageAccess,
    ) -> Result<ListedShare, PageError> {
        self.observe("share_page", async {
            let mut tx = self.pool().begin().await?;
            let context = self.context(&mut tx, principal, page_id).await?;
            context.check(Action::SharePage)?;
            let workspace_id = context.page().workspace_id;
            if user_id == principal.user.id {
                return Err(PageError::InvalidInput(
                    "a page is shared with others, not with yourself".to_owned(),
                ));
            }
            let member = workspaces::member(&mut *tx, workspace_id, user_id)
                .await?
                .ok_or(PageError::NotFound(NO_MEMBER))?;
            let (share, change) = self
                .insert_share(&mut tx, principal, &context, user_id, access)
                .await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                page_id = %page_id,
                user_id = %user_id,
                access = %access,
                "page shared"
            );
            self.changed(change);
            Ok(ListedShare {
                set_on: Some(share.page_id),
                share,
                person: member.person,
                inherited: false,
            })
        })
        .await
    }

    /// Shares the page of `context`, read on `conn`, with `user_id` of its workspace, as the
    /// principal, whom the caller found may share it: for this use case, and for those that
    /// share a page as part of their own change. Answers the share, and the access change to
    /// report with [`report_access`](Self::report_access) once the transaction commits.
    pub(crate) async fn insert_share(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        context: &Context,
        user_id: Uuid,
        access: PageAccess,
    ) -> Result<(Share, AccessChange), PageError> {
        let now = self.now();
        let page = context.page();
        let share = Share {
            page_id: page.id,
            workspace_id: page.workspace_id,
            user_id,
            access,
            created_by: Some(principal.user.id),
            created_at: now,
            updated_at: now,
        };
        repo::insert(&mut *conn, &share).await.map_err(|error| {
            if is_unique_violation(&error, ALREADY_SHARED) {
                PageError::AlreadyShared
            } else {
                PageError::Database(error)
            }
        })?;
        let change = AccessChange::Pages {
            workspace_id: page.workspace_id,
            page_id: page.id,
            user_id: Some(user_id),
        };
        Ok((share, change))
    }

    /// Tells the port of a change to who may see pages that another use case made on its
    /// own transaction, once that committed.
    pub(crate) fn report_access(&self, change: AccessChange) {
        self.changed(change);
    }

    /// Changes what a share on the page itself gives.
    pub async fn change_share(
        &self,
        principal: &Principal,
        page_id: Uuid,
        user_id: Uuid,
        access: PageAccess,
    ) -> Result<ListedShare, PageError> {
        self.observe("change_share", async {
            let mut tx = self.pool().begin().await?;
            let context = self.context(&mut tx, principal, page_id).await?;
            context.check(Action::SharePage)?;
            let workspace_id = context.page().workspace_id;
            let mut share = repo::get(&mut *tx, page_id, user_id)
                .await?
                .ok_or(PageError::NotFound(NO_SHARE))?;
            share.access = access;
            share.updated_at = self.now();
            repo::set_access(&mut *tx, page_id, user_id, access, share.updated_at).await?;
            let member = workspaces::member(&mut *tx, workspace_id, user_id)
                .await?
                .ok_or(PageError::NotFound(NO_MEMBER))?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                page_id = %page_id,
                user_id = %user_id,
                access = %access,
                "page share changed"
            );
            self.changed(AccessChange::Pages {
                workspace_id,
                page_id,
                user_id: Some(user_id),
            });
            Ok(ListedShare {
                set_on: Some(share.page_id),
                share,
                person: member.person,
                inherited: false,
            })
        })
        .await
    }

    /// Removes a share on the page itself.
    pub async fn remove_share(
        &self,
        principal: &Principal,
        page_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), PageError> {
        self.observe("remove_share", async {
            let mut tx = self.pool().begin().await?;
            let context = self.context(&mut tx, principal, page_id).await?;
            context.check(Action::SharePage)?;
            let workspace_id = context.page().workspace_id;
            if !repo::delete(&mut *tx, page_id, user_id).await? {
                return Err(PageError::NotFound(NO_SHARE));
            }
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                page_id = %page_id,
                user_id = %user_id,
                "page share removed"
            );
            self.changed(AccessChange::Pages {
                workspace_id,
                page_id,
                user_id: Some(user_id),
            });
            Ok(())
        })
        .await
    }
}
