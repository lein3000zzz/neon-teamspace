//! Pages: creating, changing, moving, trashing, restoring and deleting them, the trees they
//! make, sharing them, favorites, importing trees of them and outlining them for exports,
//! and the trash's hourly purge.
//!
//! Every use case asks [`authorize`] before it acts, so who may do what is decided in one
//! place, the domain. A change to a workspace's tree locks the tree first, in the
//! transaction that makes it, so such changes run one at a time: positions come from
//! siblings nothing else is moving, and no two moves make a cycle. Every change of the
//! tree or of a share happens in one transaction, and once it commits, the
//! [`AccessChanges`] port hears whose access it may have changed.
//!
//! Databases and their rows are pages too, of their own kinds. A database's subpages are its
//! rows, which only the database service adds and moves; no other page goes under a
//! database or a row. Rows are left out of trees, and once a row is trashed, restored or
//! renamed, those following its database hear of it.
//!
//! An agent or integration sees and changes a page only as far as its owner may too: every
//! check here asks the rules with the page as both stand to it, the owner's role,
//! teamspace and shares beside the agent's own.
//!
//! Every public use case runs in a span named `pages <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`]. An expected error, such as a refusal of the rules, is recorded
//! on the span without failing it and counts as `ok`; only failures of the server count as
//! `error`.

mod access;
mod error;
mod favorites;
mod imports;
mod operations;
mod outlines;
mod shares;
mod trees;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use std::collections::HashMap;

use neon_domain::auth::Principal;
use neon_domain::pages::{Page, PageAccess, PageKind, Placement, Space, position};
use neon_domain::workspaces::{Action, Denied, PageSpace, Resource, WorkspaceRole};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::pages::{self as repo, PageFacts, Siblings};
use neon_store::teamspaces;
use sqlx::{PgConnection, PgPool};
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use crate::clock::Clock;
use crate::settings::{SettingsService, pages as keys};
use crate::standing::{Standing, owner_of};
use crate::sync::{Broadcaster, LocalBroadcaster};

pub use access::{AccessChange, AccessChanges, Fanout, NoListener};
pub use error::PageError;
pub use imports::{Imported, ImportedContent, ImportedPage};
pub use operations::{Location, NewPage, PageUpdate};
pub(crate) use operations::{Target, in_workspace};
pub use outlines::{OutlinePage, TeamspaceOutline};
pub use shares::ListedShare;
pub use trees::{MAX_LIST, MAX_RECENT, TrashList, Tree, TreePage};

/// RED subsystem of the page use cases.
pub const SUBSYSTEM: &str = "pages";

const NO_PAGE: &str = "no such page";
const NO_WORKSPACE: &str = "no such workspace";
const NO_TEAMSPACE: &str = "no such teamspace";

/// A page as the caller sees it, with what they may do with it. The page it is under is
/// left out, `parent_id` being `None`, when the caller may not see that page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView {
    pub page: Page,
    pub access: PageAccess,
    /// The caller is a guest, who knows no other member: what is shown of the page leaves
    /// out who made, changed, trashed or owns it.
    pub members_hidden: bool,
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
    broadcaster: Arc<dyn Broadcaster>,
    clock: Clock,
    red: Red,
}

/// Puts a [`PageService`] together. Without being told otherwise it tells nobody of access
/// changes, tells changes to rows to a broadcaster of its own, and reads the system clock.
pub struct PageServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    access: Arc<dyn AccessChanges>,
    broadcaster: Arc<dyn Broadcaster>,
    clock: Clock,
}

impl PageServiceBuilder {
    /// Who hears of access changes, as the workspace service's changes tell it too.
    pub fn access_changes(mut self, access: Arc<dyn AccessChanges>) -> Self {
        self.access = access;
        self
    }

    /// Where changes to databases and their rows go, to those following them: the
    /// broadcaster the database feeds listen on.
    pub fn broadcaster(mut self, broadcaster: Arc<dyn Broadcaster>) -> Self {
        self.broadcaster = broadcaster;
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
                broadcaster: self.broadcaster,
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
            broadcaster: Arc::new(LocalBroadcaster::new()),
            clock: Clock::system(),
        }
    }

    pub(crate) fn now(&self) -> time::OffsetDateTime {
        self.inner.clock.now()
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    /// The runtime settings, as they are now.
    pub(crate) fn settings(&self) -> Arc<neon_domain::settings::Snapshot> {
        self.inner.settings.current()
    }

    /// The broadcaster that changes to databases and their rows go to.
    pub(crate) fn broadcaster(&self) -> &Arc<dyn Broadcaster> {
        &self.inner.broadcaster
    }

    fn trash_retention(&self) -> Duration {
        *self.inner.settings.current().get(keys::TRASH_RETENTION)
    }

    /// Tells the port, once the change committed.
    fn changed(&self, change: AccessChange) {
        self.inner.access.access_changed(change);
    }

    /// Deletes for good the pages in the trash longer than `pages.trash_retention`, with
    /// the pages under them, and says how many. The hourly cleanup runs it.
    pub async fn purge_trash(&self) -> Result<u64, PageError> {
        self.observe("purge_trash", async {
            let before = self.now() - self.trash_retention();
            let purged = repo::purge_trash(self.pool(), before).await?;
            if !purged.is_empty() {
                info!(pages = purged.len(), "pages deleted from the trash");
            }
            for page in purged.iter().filter(|page| page.on_its_own) {
                self.changed(AccessChange::Pages {
                    workspace_id: page.workspace_id,
                    page_id: page.id,
                    user_id: None,
                });
            }
            Ok(u64::try_from(purged.len()).unwrap_or(u64::MAX))
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
    /// principal, and for an agent's owner. Answers [`NO_PAGE`] when there is no such page;
    /// whether the principal may know of it is for the caller to ask.
    pub(crate) async fn context(
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
        let standing = Standing::read(&mut *conn, principal, page.workspace_id).await?;
        let space = page_space(&mut *conn, page, user).await?;
        let (owner_space, owner_shares) = match standing.owner_id() {
            Some(owner) => {
                let shares: HashMap<Uuid, Option<PageAccess>> = repo::chain(&mut *conn, id, owner)
                    .await?
                    .into_iter()
                    .map(|(page, shared)| (page.id, shared))
                    .collect();
                (page_space(&mut *conn, page, owner).await?, Some(shares))
            }
            None => (space, None),
        };
        // What a share gives holds for every page under it: each page gets the most of
        // its own share and those above it.
        let mut chain: Vec<Link> = Vec::with_capacity(found.len());
        let (mut above, mut owner_above) = (None, None);
        for (page, shared) in found.into_iter().rev() {
            above = above.max(shared);
            owner_above = match &owner_shares {
                Some(shares) => owner_above.max(shares.get(&page.id).copied().flatten()),
                None => above,
            };
            chain.push(Link {
                page,
                shared: above,
                owner_shared: owner_above,
            });
        }
        chain.reverse();
        Ok(Context {
            standing,
            space,
            owner_space,
            chain,
        })
    }

    /// The position for a page placed among `siblings` but for `except`, which moves. When
    /// the key would grow past [`position::MAX_LEN`], the siblings get fresh keys first.
    pub(crate) async fn position(
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

/// Where a page is, as the rules see it for `user`.
async fn page_space(
    conn: &mut PgConnection,
    page: &Page,
    user: Uuid,
) -> Result<PageSpace, PageError> {
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

/// A page's space and what it or a page above it is shared for, as one stands to it.
pub(crate) type Seat = (PageSpace, Option<PageAccess>);

/// Where nobody stands: what an agent's owner sees of a page they cannot know of.
const NOWHERE: Seat = (PageSpace::Private { own: false }, None);

fn page_resource((space, shared): Seat, trashed: bool) -> Resource {
    Resource::Page {
        space,
        shared,
        trashed,
    }
}

/// What a page gives the principal, standing to it as `mine` and its owner as `theirs`, as
/// the rules say: edit, view or nothing. Asked as for a page out of the trash, so it is the
/// same in the trash, where only editors see it.
pub(crate) fn access_of(standing: &Standing, mine: Seat, theirs: Seat) -> Option<PageAccess> {
    let allowed = |action| {
        standing
            .authorize(
                action,
                page_resource(mine, false),
                page_resource(theirs, false),
            )
            .is_ok()
    };
    if allowed(Action::EditPage) {
        Some(PageAccess::Edit)
    } else if allowed(Action::ViewPage) {
        Some(PageAccess::View)
    } else {
        None
    }
}

/// The space of the page of `facts` as `user` stands to it.
fn space_of(facts: &PageFacts, user: Uuid) -> Result<PageSpace, PageError> {
    Ok(match facts.page.space {
        Space::Teamspace(_) => PageSpace::Teamspace {
            access: facts.teamspace_access.ok_or_else(|| {
                PageError::Internal(format!("page {} has no teamspace", facts.page.id))
            })?,
            membership: facts.teamspace_role,
        },
        Space::Private { owner } => PageSpace::Private { own: owner == user },
    })
}

/// What the rules need to know of pages for the principal, and for an agent's owner: each
/// page's facts as the principal stands to it, with the owner's, by page id.
pub(crate) struct Facts {
    mine: Vec<PageFacts>,
    owner: Option<(Uuid, HashMap<Uuid, PageFacts>)>,
}

impl Facts {
    /// The facts of each of `ids` that exists.
    pub(crate) async fn read(
        conn: &mut PgConnection,
        principal: &Principal,
        ids: &[Uuid],
    ) -> Result<Self, PageError> {
        let mine = repo::facts(&mut *conn, ids, principal.user.id).await?;
        let owner = match owner_of(principal) {
            Some(owner) => Some((
                owner,
                repo::facts(&mut *conn, ids, owner)
                    .await?
                    .into_iter()
                    .map(|facts| (facts.page.id, facts))
                    .collect(),
            )),
            None => None,
        };
        Ok(Self { mine, owner })
    }

    /// Each page, as the principal sees it, or `None` when they may not.
    pub(crate) fn seen(
        &self,
        principal: &Principal,
    ) -> Result<HashMap<Uuid, Option<PageView>>, PageError> {
        self.mine
            .iter()
            .map(|facts| Ok((facts.page.id, self.view(principal, facts)?)))
            .collect()
    }

    /// For an agent or integration: its owner's role in the page's workspace and where
    /// they stand to the page.
    fn owners(
        &self,
        facts: &PageFacts,
    ) -> Result<Option<(Option<WorkspaceRole>, Seat)>, PageError> {
        let Some((owner, by_page)) = &self.owner else {
            return Ok(None);
        };
        Ok(Some(match by_page.get(&facts.page.id) {
            Some(theirs) => (
                theirs.workspace_role,
                (space_of(theirs, *owner)?, theirs.shared),
            ),
            None => (None, NOWHERE),
        }))
    }

    fn view(
        &self,
        principal: &Principal,
        facts: &PageFacts,
    ) -> Result<Option<PageView>, PageError> {
        let owner = self.owners(facts)?;
        let standing = Standing::new(
            principal,
            facts.page.workspace_id,
            facts.workspace_role,
            owner.and_then(|(role, _)| role),
        );
        let mine = (space_of(facts, principal.user.id)?, facts.shared);
        let theirs = owner.map_or(mine, |(_, seat)| seat);
        let trashed = facts.page.is_trashed();
        if standing
            .authorize(
                Action::ViewPage,
                page_resource(mine, trashed),
                page_resource(theirs, trashed),
            )
            .is_err()
        {
            return Ok(None);
        }
        Ok(access_of(&standing, mine, theirs).map(|access| PageView {
            page: facts.page.clone(),
            access,
            members_hidden: standing.members_hidden(),
        }))
    }
}

/// Leaves out of each view the page it is under, when the principal may not see that page.
pub(crate) async fn hide_parents(
    conn: &mut PgConnection,
    principal: &Principal,
    views: &mut [PageView],
) -> Result<(), PageError> {
    let parents: Vec<Uuid> = views
        .iter()
        .filter_map(|view| view.page.parent_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    if parents.is_empty() {
        return Ok(());
    }
    let seen_parents: std::collections::HashSet<Uuid> = Facts::read(conn, principal, &parents)
        .await?
        .seen(principal)?
        .into_iter()
        .filter_map(|(id, view)| view.map(|_| id))
        .collect();
    for view in views {
        if view
            .page
            .parent_id
            .is_some_and(|parent| !seen_parents.contains(&parent))
        {
            view.page.parent_id = None;
        }
    }
    Ok(())
}

/// Asks the rules about an action on a space's pages, as the principal and its owner
/// stand to them.
fn check_space(
    standing: &Standing,
    mine: PageSpace,
    theirs: PageSpace,
    action: Action,
    missing: &'static str,
) -> Result<(), PageError> {
    standing
        .authorize(
            action,
            Resource::Pages { space: mine },
            Resource::Pages { space: theirs },
        )
        .map_err(|denied| PageError::denied(denied, missing))
}

/// A page above or at the one a context is for, with the most it or a page above it is
/// shared with the principal for, and with an agent's owner.
pub(crate) struct Link {
    pub(crate) page: Page,
    pub(crate) shared: Option<PageAccess>,
    pub(crate) owner_shared: Option<PageAccess>,
}

/// A page, the pages above it, and what the rules need to know of them.
pub(crate) struct Context {
    pub(crate) standing: Standing,
    pub(crate) space: PageSpace,
    /// The space as an agent's owner stands to it; the principal's own for a person.
    pub(crate) owner_space: PageSpace,
    /// The page first, then each page above it.
    pub(crate) chain: Vec<Link>,
}

impl Context {
    pub(crate) fn page(&self) -> &Page {
        &self.chain[0].page
    }

    /// What the page gives the principal: edit, view or nothing.
    pub(crate) fn access(&self) -> Option<PageAccess> {
        let link = &self.chain[0];
        access_of(
            &self.standing,
            (self.space, link.shared),
            (self.owner_space, link.owner_shared),
        )
    }

    /// The principal is a guest, or an agent of one, who knows no other member.
    pub(crate) fn members_hidden(&self) -> bool {
        self.standing.members_hidden()
    }

    fn check_link(&self, action: Action, link: &Link) -> Result<(), PageError> {
        let trashed = link.page.is_trashed();
        self.standing
            .authorize(
                action,
                page_resource((self.space, link.shared), trashed),
                page_resource((self.owner_space, link.owner_shared), trashed),
            )
            .map_err(|denied| PageError::denied(denied, NO_PAGE))
    }

    /// Asks the rules about an action on the page.
    pub(crate) fn check(&self, action: Action) -> Result<(), PageError> {
        self.check_link(action, &self.chain[0])
    }

    /// Asks the rules about an action on the top of the page's space.
    pub(crate) fn check_space(&self, action: Action) -> Result<(), Denied> {
        self.standing.authorize(
            action,
            Resource::Pages { space: self.space },
            Resource::Pages {
                space: self.owner_space,
            },
        )
    }

    /// How deep the page is: 1 at the top of its space.
    fn depth(&self) -> usize {
        self.chain.len()
    }

    /// The page as the principal sees it, without the page above it when they may not see
    /// that one. Ask the rules first.
    pub(crate) fn view(self) -> Result<PageView, PageError> {
        let access = self.access().ok_or(PageError::NotFound(NO_PAGE))?;
        let parent_seen = self.parent_seen();
        let members_hidden = self.members_hidden();
        let mut page = self
            .chain
            .into_iter()
            .next()
            .map(|link| link.page)
            .ok_or(PageError::NotFound(NO_PAGE))?;
        if !parent_seen {
            page.parent_id = None;
        }
        Ok(PageView {
            page,
            access,
            members_hidden,
        })
    }

    /// Whether the principal may see the page above the page, which its view names only
    /// then.
    pub(crate) fn parent_seen(&self) -> bool {
        self.chain.get(1).is_some_and(|parent| self.sees(parent))
    }

    /// Whether the principal may see a page of the chain.
    fn sees(&self, link: &Link) -> bool {
        self.check_link(Action::ViewPage, link).is_ok()
    }

    /// The pages from the highest one the principal sees down to the page.
    fn breadcrumbs(&self) -> Vec<Page> {
        let mut crumbs: Vec<Page> = self
            .chain
            .iter()
            .take_while(|link| self.sees(link))
            .map(|link| link.page.clone())
            .collect();
        crumbs.reverse();
        crumbs
    }
}

/// Refuses to put a page under `parent` unless it is a page of blocks: a database's
/// subpages are its rows, added as rows, and a row has none.
pub(crate) fn check_parent(parent: &Page) -> Result<(), PageError> {
    match parent.kind {
        PageKind::Page => Ok(()),
        PageKind::Database => Err(PageError::InvalidInput(
            "a database's subpages are its rows: add a row to it instead".to_owned(),
        )),
        PageKind::Row => Err(PageError::InvalidInput(
            "a database's row has no subpages".to_owned(),
        )),
    }
}

/// The key for an entry of `list`, a list of ids and keys in order, placed by
/// `placement`; `None` when it would be longer than [`position::MAX_LEN`], and the list
/// wants fresh keys first.
fn key_in(list: &[(Uuid, String)], placement: Placement) -> Result<Option<String>, PageError> {
    key_among(
        list,
        placement,
        "the page to place it beside is not among its new neighbours",
    )
}

/// The key for an entry of `list` placed by `placement`, as [`key_in`] has it; `missing`
/// says what is wrong when the placement names an entry the list lacks.
pub(crate) fn key_among(
    list: &[(Uuid, String)],
    placement: Placement,
    missing: &'static str,
) -> Result<Option<String>, PageError> {
    let index_of = |id: Uuid| {
        list.iter()
            .position(|(entry, _)| *entry == id)
            .ok_or_else(|| PageError::InvalidInput(missing.to_owned()))
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
pub(crate) fn renumbered(list: &[(Uuid, String)]) -> Vec<(Uuid, String)> {
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
