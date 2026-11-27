//! Search: a workspace's pages, databases and rows found by a query, among those the caller
//! may see, the best first, with their matches marked; and rebuilding the text search reads.
//!
//! - **What is read.** A page's title; a row's title and the values of its text and URL
//!   properties; the text of a page's or a row's document, which is stored with each
//!   snapshot of it, in the same transaction; and the words of its comments. Titles, values
//!   and comments change with their own writes. Nothing goes through the job queue. See
//!   [`neon_store::search`]. A comment is found through its page alone, which whoever finds
//!   may read its comments.
//! - **The query** is read by [`neon_domain::search`]: words, quoted phrases, words left
//!   out and `OR`. A query without words lists what the filters let through, the most
//!   recently changed first.
//! - **Who sees what.** The rules say, for the caller and for an agent's owner, which
//!   teamspaces show their pages, whether private pages and shares do
//!   ([`person_sees_page`]), and the database keeps to it, so whoever sees a few pages of a
//!   large workspace gets them quickly. Every page found is then asked of the rules again,
//!   as any page read is, and only those they let the caller see are answered, without
//!   the page above one they may not see. Pages in the trash are left out. Guests, and
//!   agents of guests, know no other member, and may not filter by who made or changed a
//!   page.
//! - **What it costs.** Each user's searches count against [`Bucket::Search`]; at most
//!   [`SearchServiceBuilder::parallel`] run at once, so searches never hold every connection
//!   of the pool, and one waits [`QUEUE_WAIT`] for its turn at most; and each statement of a
//!   search runs [`TIMEOUT`] at most. See [`neon_store::search`] for what one search reads.
//! - **Rebuilding.** [`SearchService::reindex`] reads pages' documents again and stores
//!   their text, for those with changes no snapshot took in yet or whose text was never
//!   stored, or every page.
//!
//! Every use case runs in a span named `search <operation>` and is measured as RED metrics
//! of [`SUBSYSTEM`]; a refusal is `ok`. Nothing logs a query, but for its length, nor what
//! a page holds.

mod error;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neon_domain::auth::Principal;
use neon_domain::pages::{PageAccess, PageKind};
use neon_domain::search::{self as syntax, Query};
use neon_domain::workspaces::{PageSpace, WorkspaceRole, person_sees_page};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::search::{self as repo, MATCH_END, MATCH_START, Scope, Search, Side};
use neon_store::{documents, teamspaces};
use sqlx::{PgConnection, PgPool};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{Instrument as _, Span, field, info, info_span, warn};
use uuid::Uuid;
use yrs::Transact as _;

use crate::clock::Clock;
use crate::pages::{Facts, PageView, hide_parents, in_workspace};
use crate::ratelimit::{Bucket, MemoryLimiter, RateLimiter};
use crate::sync::assemble;

pub use error::SearchError;

/// RED subsystem of the search use cases.
pub const SUBSYSTEM: &str = "search";

/// How many pages a part of the results holds unless asked otherwise, and at most.
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 50;

/// How far into the results one may go: the best this many.
pub const MAX_RESULTS: usize = 1000;

/// The longest ago `edited_within` reaches.
pub const MAX_EDITED_WITHIN: Duration = Duration::from_secs(10 * 365 * 24 * 3600);

/// The longest each statement of a search runs before it is given up.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The longest a search waits for its turn while as many as may run at once are running.
pub const QUEUE_WAIT: Duration = Duration::from_secs(2);

/// How many searches run at once unless told otherwise.
pub const DEFAULT_PARALLEL: usize = 4;

/// How many pages a round of rebuilding reads.
const REINDEX_ROUND: i64 = 200;

/// What to look for, and where.
#[derive(Debug, Clone, Default)]
pub struct SearchInput {
    /// The query, as [`neon_domain::search`] reads it; empty lists what the filters let
    /// through.
    pub query: String,
    /// Only in this teamspace.
    pub teamspace_id: Option<Uuid>,
    /// Only pages, databases or rows.
    pub kind: Option<PageKind>,
    /// Only those this member made.
    pub created_by: Option<Uuid>,
    /// Only those this member changed last.
    pub edited_by: Option<Uuid>,
    /// Only those changed this long ago at most.
    pub edited_within: Option<Duration>,
    /// Where the part starts: the `next` of the part before.
    pub after: Option<String>,
    pub limit: Option<usize>,
}

/// A run of text, a match or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// It is what the query matched.
    pub highlight: bool,
}

/// A page found.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub page: PageView,
    /// How well it matched: higher is better; 0 without a query.
    pub rank: f64,
    /// Its title, its matches marked; empty without a query.
    pub title: Vec<Segment>,
    /// A few words of its text around its matches, marked, its lines set apart by ` · `;
    /// empty without a query or text.
    pub snippet: Vec<Segment>,
}

/// A part of the results, the best first.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Where the next part starts; `None` on the last.
    pub next: Option<String>,
}

/// The search use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct SearchService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    clock: Clock,
    limiter: Arc<dyn RateLimiter>,
    /// The turns of the searches that may run at once.
    turns: Arc<Semaphore>,
    red: Red,
}

/// Puts a [`SearchService`] together. Without being told otherwise it reads the system
/// clock, limits rates in memory, and runs [`DEFAULT_PARALLEL`] searches at once.
pub struct SearchServiceBuilder {
    pool: PgPool,
    clock: Clock,
    limiter: Option<Arc<dyn RateLimiter>>,
    parallel: usize,
}

impl SearchServiceBuilder {
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// How many searches run at once, at least one: fewer than the pool's connections, so
    /// searching never leaves the other requests none.
    pub fn parallel(mut self, parallel: usize) -> Self {
        self.parallel = parallel.max(1);
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> SearchService {
        SearchService {
            inner: Arc::new(Inner {
                pool: self.pool,
                clock: self.clock,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                turns: Arc::new(Semaphore::new(self.parallel)),
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl SearchService {
    pub fn builder(pool: PgPool) -> SearchServiceBuilder {
        SearchServiceBuilder {
            pool,
            clock: Clock::system(),
            limiter: None,
            parallel: DEFAULT_PARALLEL,
        }
    }

    /// The pages of a workspace the caller may see that `input` finds: see the module docs.
    pub async fn search(
        &self,
        principal: &Principal,
        workspace_id: Uuid,
        input: SearchInput,
    ) -> Result<SearchResults, SearchError> {
        let span = info_span!(
            "search",
            otel.name = "search search",
            search.operation = "search",
            search.query_chars = input.query.chars().count(),
            search.results = field::Empty,
        );
        self.observe("search", span.clone(), async {
            let query = syntax::parse(&input.query).map_err(SearchError::Query)?;
            let limit = match input.limit {
                None => DEFAULT_LIMIT,
                Some(limit) if (1..=MAX_LIMIT).contains(&limit) => limit,
                Some(_) => {
                    return Err(SearchError::InvalidInput(format!(
                        "a part holds 1 to {MAX_LIMIT} results"
                    )));
                }
            };
            let offset = input
                .after
                .as_deref()
                .map(offset_of)
                .transpose()?
                .unwrap_or(0);
            let edited_after = match input.edited_within {
                Some(within) if within > MAX_EDITED_WITHIN || within.is_zero() => {
                    return Err(SearchError::InvalidInput(
                        "edited_within reaches back at least a second and at most ten years"
                            .to_owned(),
                    ));
                }
                Some(within) => Some(self.inner.clock.now() - within),
                None => None,
            };
            let user = principal.user.id.to_string();
            if let Err(limited) = self.inner.limiter.check(Bucket::Search, &user).await {
                warn!(user_id = %principal.user.id, "searches rate limited");
                return Err(SearchError::RateLimited {
                    retry_after: limited.retry_after,
                });
            }
            let _turn = self.turn().await?;
            let mut tx = self.inner.pool.begin().await?;
            repo::prepare(&mut tx, TIMEOUT).await?;
            let standing = in_workspace(&mut tx, principal, workspace_id).await?;
            if standing.members_hidden()
                && (input.created_by.is_some() || input.edited_by.is_some())
            {
                return Err(SearchError::Forbidden(
                    "guests know no other member, and cannot filter by who made or changed a page",
                ));
            }
            let mine = side(&mut tx, workspace_id, principal.user.id, standing.role()).await?;
            let owner = match standing.owner() {
                Some((owner, role)) => Some(side(&mut tx, workspace_id, owner, role).await?),
                None => None,
            };
            let found = repo::search(
                &mut tx,
                &Scope { mine, owner },
                &Search {
                    workspace_id,
                    text_search: query.text_search.as_deref(),
                    fuzzy: query.fuzzy.as_deref(),
                    excluded: query.excluded.as_deref(),
                    words: &query.words,
                    teamspace_id: input.teamspace_id,
                    kind: input.kind.map(PageKind::as_str),
                    created_by: input.created_by,
                    last_edited_by: input.edited_by,
                    edited_after,
                    limit: i64::try_from(limit).unwrap_or(i64::MAX),
                    offset: i64::try_from(offset).unwrap_or(i64::MAX),
                },
            )
            .await
            .map_err(|error| {
                if repo::timed_out(&error) {
                    warn!(timeout = ?TIMEOUT, "a search took too long");
                    SearchError::TimedOut(TIMEOUT)
                } else {
                    error.into()
                }
            })?;
            let more = found.more && offset + limit < MAX_RESULTS;
            let hits = self.seen(&mut tx, principal, found.hits, &query).await?;
            tx.commit().await?;
            Span::current().record("search.results", hits.len());
            Ok(SearchResults {
                hits,
                next: more.then(|| (offset + limit).to_string()),
            })
        })
        .await
    }

    /// A turn to search, held while the search runs, once fewer than may run at once are
    /// running; [`SearchError::Busy`] when none comes within [`QUEUE_WAIT`].
    async fn turn(&self) -> Result<OwnedSemaphorePermit, SearchError> {
        match tokio::time::timeout(QUEUE_WAIT, Arc::clone(&self.inner.turns).acquire_owned()).await
        {
            Ok(Ok(turn)) => Ok(turn),
            // The semaphore is never closed; a search waiting too long is told to come back.
            Ok(Err(_)) | Err(_) => {
                warn!("too many searches running at once");
                Err(SearchError::Busy)
            }
        }
    }

    /// Each page found that the rules let the principal see, as they see it, in order.
    async fn seen(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        found: Vec<repo::Hit>,
        query: &Query,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let ids: Vec<Uuid> = found.iter().map(|hit| hit.id).collect();
        let mut seen = Facts::read(&mut *conn, principal, &ids)
            .await?
            .seen(principal)?;
        let mut views = Vec::with_capacity(found.len());
        let mut kept = Vec::with_capacity(found.len());
        for hit in found {
            match seen.remove(&hit.id).flatten() {
                Some(view) => {
                    views.push(view);
                    kept.push(hit);
                }
                // The query keeps to what the rules allow; this would be a difference
                // between the two, which the rules win.
                None => warn!(page_id = %hit.id, "a page found is not one the caller may see"),
            }
        }
        hide_parents(&mut *conn, principal, &mut views).await?;
        Ok(views
            .into_iter()
            .zip(kept)
            .map(|(page, hit)| SearchHit {
                title: match (&hit.title, query.is_empty()) {
                    (Some(marked), false) => segments(marked),
                    _ => Vec::new(),
                },
                snippet: hit
                    .snippet
                    .as_deref()
                    .map(|snippet| segments(&one_line(snippet)))
                    .unwrap_or_default(),
                rank: hit.rank,
                page,
            })
            .collect())
    }

    /// Reads pages' documents again and stores their text for search: those with changes no
    /// snapshot took in yet, as a server stopped before it saved them leaves, and those whose
    /// text was never stored, or with `all`, every page with a document. Says how many
    /// pages' text it changed. Each page is read and written while its row is locked, so a
    /// snapshot saved meanwhile writes its own text after. A page whose document does not
    /// load is logged and left.
    pub async fn reindex(&self, all: bool) -> Result<u64, SearchError> {
        let span = info_span!(
            "search",
            otel.name = "search reindex",
            search.operation = "reindex",
            search.all = all,
        );
        self.observe("reindex", span, async {
            let mut after = None;
            let mut stored = 0u64;
            loop {
                let mut conn = self.inner.pool.acquire().await?;
                let round = repo::unindexed(&mut conn, all, after, REINDEX_ROUND).await?;
                drop(conn);
                for page_id in &round {
                    if self.reindex_page(*page_id).await? {
                        stored += 1;
                    }
                }
                if i64::try_from(round.len()).unwrap_or(i64::MAX) < REINDEX_ROUND {
                    break;
                }
                after = round.last().copied();
            }
            info!(pages = stored, all, "search text stored again");
            Ok(stored)
        })
        .await
    }

    /// Stores one page's text from its document as logged; `false` when it had that text
    /// already, the page is gone or its document does not load.
    async fn reindex_page(&self, page_id: Uuid) -> Result<bool, SearchError> {
        let mut tx = self.inner.pool.begin().await?;
        let Some(stored) = repo::lock_text(&mut tx, page_id).await? else {
            return Ok(false);
        };
        let snapshot = documents::snapshot(&mut *tx, page_id).await?;
        let after = snapshot.as_ref().map_or(0, |snapshot| snapshot.last_seq);
        let updates = documents::updates_after(&mut *tx, page_id, after).await?;
        let doc = match assemble(snapshot.as_ref(), &updates) {
            Ok(doc) => doc,
            Err(error) => {
                warn!(%page_id, %error, "a page's document does not load; its search text stays");
                return Ok(false);
            }
        };
        let text = {
            let fragment = neon_blocks::fragment(&doc);
            neon_blocks::read_lenient(&doc.transact(), &fragment)
                .document
                .plain_text()
        };
        if stored.as_deref() == Some(text.as_str()) {
            return Ok(false);
        }
        repo::set_text(&mut tx, page_id, &text).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        span: Span,
        work: impl Future<Output = Result<T, SearchError>>,
    ) -> Result<T, SearchError> {
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

/// What one person sees of a workspace's pages, as the rules say: the teamspaces whose
/// pages they see, their own private pages, and shares.
async fn side(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    role: Option<WorkspaceRole>,
) -> Result<Side, SearchError> {
    let teamspaces = teamspaces::list_with_membership(&mut *conn, workspace_id, user_id)
        .await?
        .into_iter()
        .filter(|(teamspace, membership, _)| {
            person_sees_page(
                role,
                PageSpace::Teamspace {
                    access: teamspace.access,
                    membership: *membership,
                },
                None,
            )
        })
        .map(|(teamspace, _, _)| teamspace.id)
        .collect();
    let private = person_sees_page(role, PageSpace::Private { own: true }, None);
    let shares = person_sees_page(
        role,
        PageSpace::Private { own: false },
        Some(PageAccess::View),
    );
    Ok(Side {
        teamspaces,
        private: private.then_some(user_id),
        shares: shares.then_some(user_id),
    })
}

const INVALID_CURSOR: &str =
    "this cursor comes from no search: pass back the one a search answered";

fn offset_of(cursor: &str) -> Result<usize, SearchError> {
    cursor
        .parse::<usize>()
        .ok()
        .filter(|offset| *offset < MAX_RESULTS)
        .ok_or_else(|| SearchError::InvalidInput(INVALID_CURSOR.to_owned()))
}

/// What stands between the lines of a snippet, such as a page's blocks, its list's items, or
/// a row's values, which would otherwise run together where it shows on one line.
const LINE_BREAK: &str = " · ";

/// A snippet on one line: its lines, trimmed, but for empty ones, joined by [`LINE_BREAK`].
fn one_line(snippet: &str) -> String {
    snippet
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(LINE_BREAK)
}

/// Text marked between [`MATCH_START`] and [`MATCH_END`], as runs.
fn segments(marked: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut highlight = false;
    let mut text = String::new();
    let mut push = |text: &mut String, highlight: bool| {
        if !text.is_empty() {
            segments.push(Segment {
                text: std::mem::take(text),
                highlight,
            });
        }
    };
    for c in marked.chars() {
        match c {
            MATCH_START if !highlight => {
                push(&mut text, false);
                highlight = true;
            }
            MATCH_END if highlight => {
                push(&mut text, true);
                highlight = false;
            }
            MATCH_START | MATCH_END => {}
            c => text.push(c),
        }
    }
    push(&mut text, highlight);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, highlight: bool) -> Segment {
        Segment {
            text: text.to_owned(),
            highlight,
        }
    }

    #[test]
    fn marks_make_runs() {
        assert_eq!(
            segments("The \u{1}road\u{2}map and \u{1}plan\u{2}"),
            [
                run("The ", false),
                run("road", true),
                run("map and ", false),
                run("plan", true)
            ]
        );
        assert_eq!(segments("plain"), [run("plain", false)]);
        assert_eq!(segments(""), []);
        // A mark left open runs to the end; one out of place is dropped.
        assert_eq!(
            segments("\u{2}a \u{1}b"),
            [run("a ", false), run("b", true)]
        );
    }

    #[test]
    fn a_snippet_shows_on_one_line_its_lines_set_apart() {
        assert_eq!(
            one_line("a sensor, a rule, an \u{1}alert\u{2}\n\nLive demo:\r\n  Water it \n"),
            "a sensor, a rule, an \u{1}alert\u{2} · Live demo: · Water it"
        );
        assert_eq!(one_line("one line"), "one line");
        assert_eq!(one_line("\n\n"), "");
    }

    #[test]
    fn cursors_are_offsets_within_the_results() {
        assert_eq!(offset_of("20").ok(), Some(20));
        for bad in ["", "-1", "x", "1000", "99999999999999999999"] {
            assert!(offset_of(bad).is_err(), "{bad}");
        }
    }
}
