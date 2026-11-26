//! Page history: the versions a page's snapshots leave, listing them, reading one, comparing
//! one with the page as it is now, restoring one, and thinning them out.
//!
//! - **Versions.** Each time the live document saves a snapshot, or a page starts with
//!   content, its blocks are kept as a version when they differ from the latest one, in the
//!   same transaction, with who changed the page since and how many blocks they added,
//!   changed and removed. A database's rows are pages, so their bodies have versions too; a
//!   database itself has no body, and no history.
//! - **Reading and comparing** need to see the page, as reading it does. Comparing tells
//!   block by block what changed from the version to the page as it is now, live document
//!   included, and the revision it is at.
//! - **Restoring** needs to edit the page, as editing it does, and names the revision the
//!   restorer compared against: once the page is at another, the restore is refused, so
//!   nothing written meanwhile is replaced unseen. It goes through the page's live document
//!   as one change, made by the restorer, which every editor sees at once: see
//!   [`SyncService::restore`]. The page as it was is kept as a version first, so a restore
//!   is undone by restoring that one.
//! - **Thinning.** The hourly cleanup keeps what the `history.*` settings say: see
//!   [`neon_domain::history`].
//!
//! Guests, and agents of guests, know no other member: the authors of the versions they
//! read are left out. Every use case runs in a span named `history <operation>` and is
//! measured as RED metrics of [`SUBSYSTEM`]; a refusal is `ok`. Nothing logs a page's
//! content.

mod error;
pub(crate) mod record;

use std::future::Future;
use std::sync::Arc;

use neon_blocks::Document;
use neon_blocks::diff::{Line, Summary, diff};
use neon_domain::auth::Principal;
use neon_domain::history::{Retention, to_delete};
use neon_domain::pages::PageKind;
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::versions::{self, VersionInfo};
use sqlx::PgPool;
use time::OffsetDateTime;
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use crate::clock::Clock;
use crate::pages::{PageService, PageView};
use crate::settings::{SettingsService, history as keys};
use crate::sync::{ContentError, SyncService, revision};

pub use error::HistoryError;

/// RED subsystem of the history use cases.
pub const SUBSYSTEM: &str = "history";

/// The most versions a list holds at once.
pub const MAX_LIST: i64 = 100;

/// How many pages' versions one round of thinning reads.
const THIN_ROUND: i64 = 200;

/// A version as the caller sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: Uuid,
    pub page_id: Uuid,
    /// When its latest change was made.
    pub created_at: OffsetDateTime,
    /// Who changed the page since the version before; empty for a guest.
    pub authors: Vec<Uuid>,
    /// How many blocks it added, changed and removed since the version before.
    pub summary: Summary,
    /// The version a restore made this one from, while it is kept, and when that one was
    /// made.
    pub restored_from: Option<Uuid>,
    pub restored_from_at: Option<OffsetDateTime>,
}

impl Version {
    fn seen_by(info: VersionInfo, members_hidden: bool) -> Self {
        let count = |n: i32| usize::try_from(n).unwrap_or_default();
        Self {
            id: info.id,
            page_id: info.page_id,
            created_at: info.created_at,
            authors: if members_hidden {
                Vec::new()
            } else {
                info.authors
            },
            summary: Summary {
                added: count(info.added),
                changed: count(info.changed),
                removed: count(info.removed),
            },
            restored_from: info.restored_from,
            restored_from_at: info.restored_from_at,
        }
    }
}

/// A part of a page's history, latest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionList {
    pub page: PageView,
    pub versions: Vec<Version>,
    /// Where the next part starts; `None` on the last.
    pub next: Option<String>,
}

/// A version with its blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionContent {
    pub page: PageView,
    pub version: Version,
    pub document: Document,
}

/// A version compared with the page as it is now.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub page: PageView,
    pub version: Version,
    /// From the version to now, block by block: see [`neon_blocks::diff`].
    pub lines: Vec<Line>,
    pub summary: Summary,
    /// The revision the page is at, which a restore names as its base.
    pub revision: String,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub page: PageView,
    pub version: Version,
    /// The page's revision once restored.
    pub revision: Option<String>,
}

/// The history use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct HistoryService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    pages: PageService,
    sync: SyncService,
    clock: Clock,
    red: Red,
}

/// Puts a [`HistoryService`] together, reading the system clock unless told otherwise.
pub struct HistoryServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    pages: PageService,
    sync: SyncService,
    clock: Clock,
}

impl HistoryServiceBuilder {
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> HistoryService {
        HistoryService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                pages: self.pages,
                sync: self.sync,
                clock: self.clock,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl HistoryService {
    pub fn builder(
        pool: PgPool,
        settings: Arc<SettingsService>,
        pages: PageService,
        sync: SyncService,
    ) -> HistoryServiceBuilder {
        HistoryServiceBuilder {
            pool,
            settings,
            pages,
            sync,
            clock: Clock::system(),
        }
    }

    /// A page's versions the caller may see the page of, latest first: at most `limit`, up
    /// to [`MAX_LIST`], after the cursor a list answered, if given.
    pub async fn versions(
        &self,
        principal: &Principal,
        page_id: Uuid,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> Result<VersionList, HistoryError> {
        self.observe("list", async {
            let limit = match limit {
                None => MAX_LIST,
                Some(limit) if (1..=MAX_LIST).contains(&limit) => limit,
                Some(_) => {
                    return Err(HistoryError::InvalidInput(format!(
                        "a list holds 1 to {MAX_LIST} versions at a time"
                    )));
                }
            };
            let after = after.map(cursor).transpose()?;
            let page = self.body(principal, page_id).await?;
            let mut found = versions::list(&self.inner.pool, page_id, after, limit + 1).await?;
            let more = found.len() > usize::try_from(limit).unwrap_or(usize::MAX);
            found.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            let next = more
                .then(|| found.last().map(|last| position(last.created_at, last.id)))
                .flatten();
            let hidden = page.members_hidden;
            Ok(VersionList {
                versions: found
                    .into_iter()
                    .map(|info| Version::seen_by(info, hidden))
                    .collect(),
                page,
                next,
            })
        })
        .await
    }

    /// A version of a page the caller may see, with its blocks.
    pub async fn version(
        &self,
        principal: &Principal,
        page_id: Uuid,
        version_id: Uuid,
    ) -> Result<VersionContent, HistoryError> {
        self.observe("get", async {
            let page = self.body(principal, page_id).await?;
            let (version, document) = self.read(&page, version_id).await?;
            Ok(VersionContent {
                page,
                version,
                document,
            })
        })
        .await
    }

    /// A version compared with the page as it is now, and the revision it is at.
    pub async fn compare(
        &self,
        principal: &Principal,
        page_id: Uuid,
        version_id: Uuid,
    ) -> Result<Comparison, HistoryError> {
        self.observe("compare", async {
            let page = self.body(principal, page_id).await?;
            let (version, document) = self.read(&page, version_id).await?;
            let now = self
                .inner
                .sync
                .content(page_id)
                .await
                .map_err(ContentError::from)?;
            let lines = diff(&document, &now);
            Ok(Comparison {
                summary: Summary::of(&lines),
                revision: revision(&now),
                page,
                version,
                lines,
            })
        })
        .await
    }

    /// Makes the page hold a version of it again, for those who may edit it, as one change
    /// through its live document, on the revision `base` the caller compared against: see the
    /// module docs.
    pub async fn restore(
        &self,
        principal: &Principal,
        page_id: Uuid,
        version_id: Uuid,
        base: String,
    ) -> Result<Restored, HistoryError> {
        self.observe("restore", async {
            let page = self.body(principal, page_id).await?;
            let (version, document) = self.read(&page, version_id).await?;
            let restored = self
                .inner
                .sync
                .restore(
                    principal,
                    page_id,
                    document.blocks,
                    (version.id, version.created_at),
                    base,
                )
                .await?;
            info!(
                user_id = %principal.user.id,
                page_id = %page_id,
                version_id = %version.id,
                "version restored"
            );
            Ok(Restored {
                page: restored.page,
                version,
                revision: restored.revision,
            })
        })
        .await
    }

    /// Deletes the versions the `history.*` settings no longer keep, as of now, and says how
    /// many. The hourly cleanup runs it.
    pub async fn thin(&self) -> Result<u64, HistoryError> {
        self.observe("thin", async {
            let settings = self.inner.settings.current();
            let retention = Retention {
                keep_all: *settings.get(keys::KEEP_ALL),
                keep_daily: *settings.get(keys::KEEP_DAILY),
                max_versions: usize::try_from(*settings.get(keys::MAX_VERSIONS))
                    .unwrap_or(usize::MAX),
            };
            let now = self.inner.clock.now();
            let before = now - retention.keep_all;
            let max = i64::try_from(retention.max_versions).unwrap_or(i64::MAX);
            let (mut after, mut deleted) = (None, 0);
            loop {
                let round =
                    versions::to_thin(&self.inner.pool, after, before, max, THIN_ROUND).await?;
                for (_, dated) in &round {
                    let doomed = to_delete(dated, now, retention);
                    if !doomed.is_empty() {
                        deleted += versions::delete(&self.inner.pool, &doomed).await?;
                    }
                }
                if i64::try_from(round.len()).unwrap_or(i64::MAX) < THIN_ROUND {
                    break;
                }
                after = round.last().map(|(page, _)| *page);
            }
            if deleted > 0 {
                info!(versions = deleted, "page versions thinned out");
            }
            Ok(deleted)
        })
        .await
    }

    /// A page the caller may see that has a body: a page or a row, not a database.
    async fn body(&self, principal: &Principal, page_id: Uuid) -> Result<PageView, HistoryError> {
        let page = self.inner.pages.page(principal, page_id).await?;
        if page.page.kind == PageKind::Database {
            return Err(ContentError::Database.into());
        }
        Ok(page)
    }

    /// A version of `page`, as the caller sees it, and its blocks.
    async fn read(
        &self,
        page: &PageView,
        version_id: Uuid,
    ) -> Result<(Version, Document), HistoryError> {
        let found = versions::get(&self.inner.pool, page.page.id, version_id)
            .await?
            .ok_or(HistoryError::NoVersion)?;
        let document = serde_json::from_value(found.content).map_err(|error| {
            HistoryError::Internal(format!("version {version_id} does not read: {error}"))
        })?;
        Ok((Version::seen_by(found.info, page.members_hidden), document))
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, HistoryError>>,
    ) -> Result<T, HistoryError> {
        let span = info_span!(
            "history",
            otel.name = %format!("history {operation}"),
            history.operation = operation,
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
}

const INVALID_CURSOR: &str = "this cursor comes from no list: pass back the one a list answered";

/// Where a list goes on from: when the last version listed was made, in microseconds, and
/// its id.
fn position(at: OffsetDateTime, id: Uuid) -> String {
    format!("{}.{id}", at.unix_timestamp_nanos() / 1000)
}

fn cursor(text: &str) -> Result<(OffsetDateTime, Uuid), HistoryError> {
    let invalid = || HistoryError::InvalidInput(INVALID_CURSOR.to_owned());
    let (micros, id) = text.split_once('.').ok_or_else(invalid)?;
    let micros: i128 = micros.parse().map_err(|_| invalid())?;
    let at = OffsetDateTime::from_unix_timestamp_nanos(micros * 1000).map_err(|_| invalid())?;
    Ok((at, Uuid::try_parse(id).map_err(|_| invalid())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_go_round() {
        let at =
            OffsetDateTime::from_unix_timestamp_nanos(1_795_348_800_123_456_000).expect("a time");
        let id = Uuid::from_u128(7);
        assert_eq!(cursor(&position(at, id)).ok(), Some((at, id)));
        for bad in ["", "12", "x.y", "12.not-a-uuid"] {
            assert!(cursor(bad).is_err(), "{bad}");
        }
    }
}
