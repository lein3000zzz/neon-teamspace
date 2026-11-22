//! Pages: creating, changing, moving, trashing, restoring and deleting them, the trees they
//! make, sharing them, favorites, and the trash's hourly purge.
//!
//! Every use case asks [`authorize`] before it acts, so who may do what is decided in one
//! place, the domain. A change to a workspace's tree locks the tree first, in the
//! transaction that makes it, so such changes run one at a time: positions come from
//! siblings nothing else is moving, and no two moves make a cycle. Every change of the
//! tree or of a share happens in one transaction, and once it commits, the
//! [`AccessChanges`] port hears whose access it may have changed.
//!
//! Every public use case runs in a span named `pages <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`]. An expected error, such as a refusal of the rules, is recorded
//! on the span without failing it and counts as `ok`; only failures of the server count as
//! `error`.

mod access;
mod error;
mod favorites;
mod operations;
mod shares;
mod trees;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neon_domain::auth::Principal;
use neon_domain::pages::{Page, PageAccess, Placement, position};
use neon_domain::workspaces::{Action, Actor, PageSpace, Resource, WorkspaceRole, authorize};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::pages::{self as repo, PageFacts, Siblings};
use neon_store::{teamspaces, workspaces};
use sqlx::{PgConnection, PgPool};
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use crate::clock::Clock;
use crate::settings::{SettingsService, pages as keys};

pub use access::{AccessChange, AccessChanges, NoListener};
pub use error::PageError;
pub use operations::{Location, NewPage, PageUpdate};
pub use shares::ListedShare;
pub use trees::{MAX_LIST, TrashList, Tree, TreePage};

/// RED subsystem of the page use cases.
pub const SUBSYSTEM: &str = "pages";

const NO_PAGE: &str = "no such page";
const NO_WORKSPACE: &str = "no such workspace";
const NO_TEAMSPACE: &str = "no such teamspace";

/// A page as the caller sees it, with what they may do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView {
    pub page: Page,
    pub access: PageAccess,
}

/// The page use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct PageService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    access: Arc<dyn AccessChanges>,
    clock: Clock,
    red: Red,
}

/// Puts a [`PageService`] together. Without being told otherwise it tells nobody of access
/// changes and reads the system clock.
pub struct PageServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    access: Arc<dyn AccessChanges>,
    clock: Clock,
}

impl PageServiceBuilder {
    /// Who hears of access changes, as the workspace service's changes tell it too.
    pub fn access_changes(mut self, access: Arc<dyn AccessChanges>) -> Self {
        self.access = access;
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> PageService {
        PageService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                access: self.access,
                clock: self.clock,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl PageService {
    pub fn builder(pool: PgPool, settings: Arc<SettingsService>) -> PageServiceBuilder {
        PageServiceBuilder {
            pool,
            settings,
            access: Arc::new(NoListener),
            clock: Clock::system(),
        }
    }

    fn now(&self) -> time::OffsetDateTime {
        self.inner.clock.now()
    }

    fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    fn trash_retention(&self) -> Duration {
        *self.inner.settings.current().get(keys::TRASH_RETENTION)
    }

    /// Tells the port, once the change committed.
    fn changed(&self, change: AccessChange) {
        self.inner.access.access_changed(change);
    }

    /// Deletes for good the pages in the trash longer than `pages.trash_retention`, with
    /// the pages under them. The hourly cleanup runs it.
    pub async fn purge_trash(&self) -> Result<u64, PageError> {
        self.observe("purge_trash", async {
            let before = self.now() - self.trash_retention();
            let purged = repo::purge_trash(self.pool(), before).await?;
            if purged > 0 {
                info!(pages = purged, "pages deleted from the trash");
            }
            Ok(purged)
        })
        .await
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, PageError>>,
    ) -> Result<T, PageError> {
        let span = info_span!(
            "pages",
            otel.name = %format!("pages {operation}"),
            pages.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }

    /// Reads a page, the pages above it and what the rules need to know of them for the
    /// principal. Answers [`NO_PAGE`] when there is no such page; whether the principal may
    /// know of it is for the caller to ask.
    async fn context(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Context, PageError> {
        let user = principal.user.id;
        let found = repo::chain(&mut *conn, id, user).await?;
        let Some((page, _)) = found.first() else {
            return Err(PageError::NotFound(NO_PAGE));
        };
        let role = workspaces::member_role(&mut *conn, page.workspace_id, user).await?;
        let space = page_space(&mut *conn, page, user).await?;
        // What a share gives holds for every page under it: each page gets the most of
        // its own share and those above it.
        let mut chain: Vec<Link> = Vec::with_capacity(found.len());
        let mut above = None;
        for (page, shared) in found.into_iter().rev() {
            above = above.max(shared);
            chain.push(Link {
                page,
                shared: above,
            });
        }
        chain.reverse();
        Ok(Context {
            actor: actor(principal, role),
            space,
            chain,
        })
    }

    /// The position for a page placed among `siblings` but for `except`, which moves. When
    /// the key would grow past [`position::MAX_LEN`], the siblings get fresh keys first.
    async fn position(
        &self,
        conn: &mut PgConnection,
        siblings: Siblings,
        except: Option<Uuid>,
        placement: Placement,
    ) -> Result<String, PageError> {
        let list = repo::sibling_positions(&mut *conn, siblings, except).await?;
        if let Some(key) = key_in(&list, placement)? {
            return Ok(key);
        }
        let list = renumbered(&list);
        let (ids, keys): (Vec<Uuid>, Vec<String>) = list.iter().cloned().unzip();
        repo::set_positions(&mut *conn, &ids, &keys).await?;
        key_in(&list, placement)?
            .ok_or_else(|| PageError::Internal("fresh page positions are too long".to_owned()))
    }
}

/// The principal as the rules see them in a workspace where their role is `role`.
fn actor(principal: &Principal, role: Option<WorkspaceRole>) -> Actor {
    Actor {
        instance_admin: principal.is_admin(),
        via_session: principal.via_session(),
        role,
    }
}

/// Where a page is, as the rules see it for `user`.
async fn page_space(
    conn: &mut PgConnection,
    page: &Page,
    user: Uuid,
) -> Result<PageSpace, PageError> {
    use neon_domain::pages::Space;

    Ok(match page.space {
        Space::Teamspace(id) => {
            let teamspace = teamspaces::get(&mut *conn, page.workspace_id, id)
                .await?
                .ok_or_else(|| PageError::Internal(format!("page {} has no teamspace", page.id)))?;
            PageSpace::Teamspace {
                access: teamspace.access,
                membership: teamspaces::member_role(&mut *conn, id, user).await?,
            }
        }
        Space::Private { owner } => PageSpace::Private { own: owner == user },
    })
}

/// What a page gives an actor, as the rules say: edit, view or nothing. Asked as for a page
/// out of the trash, so it is the same in the trash, where only editors see it.
fn access_of(actor: Actor, space: PageSpace, shared: Option<PageAccess>) -> Option<PageAccess> {
    let allowed = |action| {
        let page = Resource::Page {
            space,
            shared,
            trashed: false,
        };
        authorize(actor, action, page).is_ok()
    };
    if allowed(Action::EditPage) {
        Some(PageAccess::Edit)
    } else if allowed(Action::ViewPage) {
        Some(PageAccess::View)
    } else {
        None
    }
}

/// The page of `facts` as the principal sees it, or `None` when they may not see it.
fn seen(principal: &Principal, facts: &PageFacts) -> Result<Option<PageView>, PageError> {
    use neon_domain::pages::Space;

    let actor = actor(principal, facts.workspace_role);
    let space = match facts.page.space {
        Space::Teamspace(_) => PageSpace::Teamspace {
            access: facts.teamspace_access.ok_or_else(|| {
                PageError::Internal(format!("page {} has no teamspace", facts.page.id))
            })?,
            membership: facts.teamspace_role,
        },
        Space::Private { owner } => PageSpace::Private {
            own: owner == principal.user.id,
        },
    };
    let page = Resource::Page {
        space,
        shared: facts.shared,
        trashed: facts.page.is_trashed(),
    };
    if authorize(actor, Action::ViewPage, page).is_err() {
        return Ok(None);
    }
    Ok(
        access_of(actor, space, facts.shared).map(|access| PageView {
            page: facts.page.clone(),
            access,
        }),
    )
}

/// Asks the rules about an action on a space's pages.
fn check_space(
    actor: Actor,
    space: PageSpace,
    action: Action,
    missing: &'static str,
) -> Result<(), PageError> {
    authorize(actor, action, Resource::Pages { space })
        .map_err(|denied| PageError::denied(denied, missing))
}

/// A page above or at the one a context is for, with the most it or a page above it is
/// shared with the principal for.
struct Link {
    page: Page,
    shared: Option<PageAccess>,
}

/// A page, the pages above it, and what the rules need to know of them.
struct Context {
    actor: Actor,
    space: PageSpace,
    /// The page first, then each page above it.
    chain: Vec<Link>,
}

impl Context {
    fn page(&self) -> &Page {
        &self.chain[0].page
    }

    fn resource(&self, link: &Link) -> Resource {
        Resource::Page {
            space: self.space,
            shared: link.shared,
            trashed: link.page.is_trashed(),
        }
    }

    /// Asks the rules about an action on the page.
    fn check(&self, action: Action) -> Result<(), PageError> {
        authorize(self.actor, action, self.resource(&self.chain[0]))
            .map_err(|denied| PageError::denied(denied, NO_PAGE))
    }

    /// How deep the page is: 1 at the top of its space.
    fn depth(&self) -> usize {
        self.chain.len()
    }

    /// The page as the principal sees it. Ask the rules first.
    fn view(self) -> Result<PageView, PageError> {
        let access = access_of(self.actor, self.space, self.chain[0].shared)
            .ok_or(PageError::NotFound(NO_PAGE))?;
        let page = self
            .chain
            .into_iter()
            .next()
            .map(|link| link.page)
            .ok_or(PageError::NotFound(NO_PAGE))?;
        Ok(PageView { page, access })
    }

    /// The pages from the highest one the principal sees down to the page.
    fn breadcrumbs(&self) -> Vec<Page> {
        let mut crumbs: Vec<Page> = self
            .chain
            .iter()
            .take_while(|link| authorize(self.actor, Action::ViewPage, self.resource(link)).is_ok())
            .map(|link| link.page.clone())
            .collect();
        crumbs.reverse();
        crumbs
    }
}

/// The key for an entry of `list`, a list of ids and keys in order, placed by
/// `placement`; `None` when it would be longer than [`position::MAX_LEN`], and the list
/// wants fresh keys first.
fn key_in(list: &[(Uuid, String)], placement: Placement) -> Result<Option<String>, PageError> {
    let index_of = |id: Uuid| {
        list.iter()
            .position(|(entry, _)| *entry == id)
            .ok_or_else(|| {
                PageError::InvalidInput(
                    "the page to place it beside is not among its new neighbours".to_owned(),
                )
            })
    };
    let key = |i: usize| list.get(i).map(|(_, key)| key.as_str());
    let (before, after) = match placement {
        Placement::First => (None, key(0)),
        Placement::Last => (list.len().checked_sub(1).and_then(key), None),
        Placement::After(id) => {
            let at = index_of(id)?;
            (key(at), key(at + 1))
        }
        Placement::Before(id) => {
            let at = index_of(id)?;
            (at.checked_sub(1).and_then(key), key(at))
        }
    };
    let new = position::between(before, after)?;
    Ok((new.len() <= position::MAX_LEN).then_some(new))
}

/// The same entries with fresh keys, in the same order.
fn renumbered(list: &[(Uuid, String)]) -> Vec<(Uuid, String)> {
    list.iter()
        .map(|(id, _)| *id)
        .zip(position::sequence(list.len()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(keys: &[&str]) -> Vec<(Uuid, String)> {
        keys.iter()
            .enumerate()
            .map(|(i, key)| (Uuid::from_u128(i as u128 + 1), (*key).to_owned()))
            .collect()
    }

    #[test]
    fn placements_find_their_neighbours() {
        let siblings = list(&["a0", "a1", "a2"]);
        let at = |placement| key_in(&siblings, placement).expect("placed");

        assert_eq!(at(Placement::First), Some("Zz".to_owned()));
        assert_eq!(at(Placement::Last), Some("a3".to_owned()));
        assert_eq!(
            at(Placement::After(Uuid::from_u128(1))),
            Some("a0V".to_owned())
        );
        assert_eq!(
            at(Placement::After(Uuid::from_u128(3))),
            Some("a3".to_owned())
        );
        assert_eq!(
            at(Placement::Before(Uuid::from_u128(1))),
            Some("Zz".to_owned())
        );
        assert_eq!(
            at(Placement::Before(Uuid::from_u128(3))),
            Some("a1V".to_owned())
        );
        assert!(matches!(
            key_in(&siblings, Placement::After(Uuid::from_u128(9))),
            Err(PageError::InvalidInput(_))
        ));
        assert_eq!(key_in(&[], Placement::After(Uuid::nil())).ok(), None);
        assert_eq!(
            key_in(&[], Placement::First).expect("placed"),
            Some(position::START.to_owned())
        );
    }

    #[test]
    fn keys_that_grew_too_long_ask_for_fresh_ones() {
        let long_a = format!("a0{}", "V".repeat(position::MAX_LEN));
        let long_b = format!("a0{}W", "V".repeat(position::MAX_LEN));
        let siblings = list(&[&long_a, &long_b]);

        assert_eq!(
            key_in(&siblings, Placement::After(Uuid::from_u128(1))).expect("placed"),
            None
        );
        let fresh = renumbered(&siblings);
        assert_eq!(
            fresh,
            vec![
                (Uuid::from_u128(1), "a0".to_owned()),
                (Uuid::from_u128(2), "a1".to_owned())
            ]
        );
        assert_eq!(
            key_in(&fresh, Placement::After(Uuid::from_u128(1))).expect("placed"),
            Some("a0V".to_owned())
        );
    }
}
