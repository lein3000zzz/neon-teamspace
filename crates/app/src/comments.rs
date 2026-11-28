//! Comments: threads on a page, on the page as a whole or anchored to a range of its text,
//! their replies, edits and deletions, resolving and reopening them, following a page's
//! comments, and the people and pages a comment or the page may mention.
//!
//! Comments are plain Postgres rows beside the page's Yjs document, never in it. An inline
//! thread keeps the two Yjs relative positions of its range and the text it was made on;
//! where that range is now is read from the page's live document, or its saved one, as
//! the comments are listed: the block it is in, or that it is detached, once its text is
//! gone.
//!
//! - **Who may.** Whoever may see a page reads its comments. Whoever may comment on it, as
//!   [`Action::CommentPage`] says, starts threads, replies, and resolves and reopens them:
//!   those who edit it, and those it is shared with to comment, guests too. A comment is
//!   changed by its author alone, while they may still comment, and deleted by them or by
//!   those who edit the page, who also delete whole threads. An agent or integration does
//!   so only as far as its owner may too.
//! - **Bounds.** A page's threads are read a part at a time, each with its first comment
//!   and its latest, the rest of a thread's comments a part at a time too. A page holds at
//!   most [`MAX_THREADS`] open threads and [`MAX_PAGE_COMMENTS`] comments, a thread
//!   [`MAX_REPLIES`], each checked under a lock of the page's comments, and each identity
//!   writes comments at the rate [`Bucket::Comment`] allows.
//! - **Telling.** A new comment tells, in its transaction, those it mentions, those in its
//!   thread and the page's followers, each once and only who may see the page; one it
//!   mentions who may not is told nothing, and its writer hears so. An edit tells only
//!   those it mentions anew. See [`crate::notifications`].
//! - **Live.** Once a change commits, the page's room of comments hears which thread
//!   changed, and the open editors read the comments again.
//! - **Search.** Each comment is kept with its words, which search finds its page by, as
//!   whoever finds the page may read its comments.
//!
//! Every use case runs in a span named `comments <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`].

mod error;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use neon_blocks::Inline;
use neon_blocks::anchors::Anchored;
use neon_blocks::markdown::inline_from_markdown;
use neon_domain::auth::{Principal, Scope, UserKind};
use neon_domain::comments::{
    Anchor, DEFAULT_PART, LISTED_COMMENTS, MAX_PAGE_COMMENTS, MAX_PART, MAX_REPLIES, MAX_THREADS,
    check_anchor, check_comment,
};
use neon_domain::workspaces::{Action, Person};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::comments::{self as repo, Comment, Thread};
use neon_store::{notifications as notifications_repo, pages as pages_repo, workspaces};
use sqlx::{PgConnection, PgPool};
use tracing::{Instrument as _, info, info_span, warn};
use uuid::Uuid;

use crate::notifications::mentions::{self, Written};
use crate::notifications::notify::{Notifier, sees};
use crate::pages::{Context, Facts, PageService, PageView};
use crate::ratelimit::{Bucket, RateLimiter};
use crate::sync::{Room, SyncService, comments_changed};

pub use error::CommentError;

/// RED subsystem of the comment use cases.
pub const SUBSYSTEM: &str = "comments";

/// The most people and pages a search for mentions answers with, each.
pub const MAX_CANDIDATES: usize = 10;

const NO_THREAD: &str = "no such thread";
const NO_COMMENT: &str = "no such comment";
const AUTHOR_ONLY: &str = "only its author changes a comment";
const NOT_YOURS: &str = "only its author, or those who edit the page, delete a comment";
const EDITORS_ONLY: &str = "only those who edit the page delete a thread";

/// Where a new thread goes on its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placing {
    /// On the page as a whole.
    Page,
    /// On a range of its text, between two Yjs relative positions as the editor made them,
    /// with the text it holds.
    Text {
        start: Vec<u8>,
        end: Vec<u8>,
        quote: String,
    },
    /// On the whole text of one of its blocks, by the block's id.
    Block(String),
}

/// A comment as its readers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentView {
    pub comment: Comment,
    /// Who wrote it, while their account is there.
    pub author: Option<Person>,
    /// Its markdown read as inline content: text, styles, links and mentions.
    pub content: Vec<Inline>,
}

/// A thread with the comments a listing shows of it, in order, and where its anchor is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadView {
    pub thread: Thread,
    /// For an inline thread, the block its text is in and whether that text is gone.
    pub anchored: Option<Anchored>,
    /// Its first comment and its latest, at most [`LISTED_COMMENTS`] in all.
    pub comments: Vec<CommentView>,
    /// How many comments it holds in all.
    pub count: usize,
}

/// Which of a page's threads to list, oldest first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadQuery {
    /// The resolved ones alone, `Some(true)`, the open ones alone, `Some(false)`, or both.
    pub resolved: Option<bool>,
    /// The threads after this one.
    pub after: Option<Uuid>,
    /// At most this many, up to [`MAX_PART`]; [`DEFAULT_PART`] unless given.
    pub limit: Option<usize>,
}

/// A part of a page's threads, oldest first, and what the caller may do with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageThreads {
    pub page_id: Uuid,
    pub threads: Vec<ThreadView>,
    /// Where the next part starts: the last thread of this one, while there may be more.
    pub next: Option<Uuid>,
    /// How many threads of the page are open.
    pub open: usize,
    /// The caller may comment: start threads, reply, resolve and reopen.
    pub may_comment: bool,
    /// The caller edits the page: they delete others' comments, and whole threads.
    pub may_moderate: bool,
    /// The caller follows the page's comments.
    pub following: bool,
    /// The caller knows no other member: people show without their addresses.
    pub members_hidden: bool,
}

/// A part of a thread's comments, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadComments {
    pub comments: Vec<CommentView>,
    /// Where the next part starts: the last comment of this one, while there may be more.
    pub next: Option<Uuid>,
    pub members_hidden: bool,
}

/// A comment just made or changed, its thread as it is then, and those it mentions who
/// may not see the page, whom it told nothing. A guest, who knows no other member, is told
/// none of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub thread: ThreadView,
    pub comment_id: Uuid,
    pub unnotified: Vec<Person>,
    pub members_hidden: bool,
}

/// Someone a comment or the page's text may mention, and whether they may see the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub person: Person,
    pub sees_page: bool,
}

/// What a mention may name, by the text typed after `@`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidates {
    pub people: Vec<Candidate>,
    pub pages: Vec<PageView>,
}

/// The comment use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct CommentService {
    inner: Arc<Inner>,
}

struct Inner {
    pages: PageService,
    sync: SyncService,
    limiter: Arc<dyn RateLimiter>,
    red: Red,
}

impl CommentService {
    /// The comments of `pages`' pages, whose anchors `sync`'s documents resolve, counted
    /// against [`Bucket::Comment`] where `sync` counts edits. Call it after telemetry is
    /// installed, as it creates the RED metrics.
    pub fn new(pages: PageService, sync: SyncService) -> Self {
        Self {
            inner: Arc::new(Inner {
                pages,
                limiter: Arc::clone(sync.limiter()),
                sync,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }

    fn pool(&self) -> &PgPool {
        self.inner.pages.pool()
    }

    fn now(&self) -> time::OffsetDateTime {
        self.inner.pages.now()
    }

    /// Runs a use case in its span and records its outcome.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, CommentError>>,
    ) -> Result<T, CommentError> {
        let span = info_span!(
            "comments",
            otel.name = %format!("comments {operation}"),
            comments.operation = operation,
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

    /// A part of a page's threads, each with its first comment and its latest, for whoever
    /// may see it.
    pub async fn threads(
        &self,
        principal: &Principal,
        page_id: Uuid,
        query: ThreadQuery,
    ) -> Result<PageThreads, CommentError> {
        self.observe("list", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, page_id)
                .await?;
            check(&context, Action::ViewPage)?;
            let limit = part(query.limit);
            let threads = repo::threads(
                &mut *conn,
                page_id,
                query.resolved,
                query.after,
                count(limit),
            )
            .await?;
            let next = (threads.len() == limit)
                .then(|| threads.last().map(|thread| thread.id))
                .flatten();
            let open = repo::count_open_threads(&mut *conn, page_id).await?;
            let following = following(&mut conn, context.page(), principal.user.id).await?;
            let listed = listed(&mut conn, &threads).await?;
            drop(conn);
            let threads = self.assemble(page_id, threads, listed).await?;
            Ok(PageThreads {
                page_id,
                threads,
                next,
                open: usize::try_from(open).unwrap_or_default(),
                may_comment: may(principal, &context, Action::CommentPage),
                may_moderate: may(principal, &context, Action::EditPage),
                following,
                members_hidden: context.members_hidden(),
            })
        })
        .await
    }

    /// One thread with its first comment and its latest, for whoever may see its page, and
    /// whether they know no other member.
    pub async fn thread(
        &self,
        principal: &Principal,
        thread_id: Uuid,
    ) -> Result<(ThreadView, bool), CommentError> {
        self.observe("get", async {
            let mut conn = self.pool().acquire().await?;
            let (_, context) = readable(&self.inner.pages, &mut conn, principal, thread_id).await?;
            drop(conn);
            Ok((self.view(thread_id).await?, context.members_hidden()))
        })
        .await
    }

    /// A part of a thread's comments, in order, for whoever may see its page: those after
    /// `after`, at most `limit`, up to [`MAX_PART`].
    pub async fn comments(
        &self,
        principal: &Principal,
        thread_id: Uuid,
        after: Option<Uuid>,
        limit: Option<usize>,
    ) -> Result<ThreadComments, CommentError> {
        self.observe("list_thread", async {
            let mut conn = self.pool().acquire().await?;
            let (_, context) = readable(&self.inner.pages, &mut conn, principal, thread_id).await?;
            let limit = part(limit);
            let written = repo::comments_after(&mut *conn, thread_id, after, count(limit)).await?;
            let next = (written.len() == limit)
                .then(|| written.last().map(|last| last.comment.id))
                .flatten();
            Ok(ThreadComments {
                comments: written.into_iter().map(view_of).collect(),
                next,
                members_hidden: context.members_hidden(),
            })
        })
        .await
    }

    /// Starts a thread on a page with its first comment, for whoever may comment on it.
    pub async fn start_thread(
        &self,
        principal: &Principal,
        page_id: Uuid,
        markdown: &str,
        placing: Placing,
    ) -> Result<Posted, CommentError> {
        self.observe("start_thread", async {
            let body = check_comment(markdown)?;
            self.limit(principal).await?;
            let anchor = match placing {
                Placing::Page => None,
                Placing::Text { start, end, quote } => Some(check_anchor(start, end, &quote)?),
                Placing::Block(block_id) => {
                    Some(self.block_anchor(principal, page_id, block_id).await?)
                }
            };
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let context = self
                .inner
                .pages
                .context(&mut tx, principal, page_id)
                .await?;
            check(&context, Action::CommentPage)?;
            repo::lock_page(&mut *tx, page_id).await?;
            if repo::count_open_threads(&mut *tx, page_id).await? >= count(MAX_THREADS) {
                return Err(CommentError::TooManyThreads);
            }
            room_for_a_comment(&mut tx, page_id).await?;
            let page = context.page();
            let thread = Thread {
                id: Uuid::now_v7(),
                workspace_id: page.workspace_id,
                page_id,
                anchor,
                resolved_at: None,
                resolved_by: None,
                created_by: Some(principal.user.id),
                created_at: now,
            };
            repo::insert_thread(&mut *tx, &thread).await?;
            let comment = new_comment(thread.id, principal, body, now);
            repo::insert_comment(&mut *tx, &comment, &words(&comment.body)).await?;
            let mut notifier = Notifier::of(&self.inner.pages);
            let told = mentions::comment(
                &mut tx,
                &self.inner.pages,
                &mut notifier,
                Written {
                    thread: &thread,
                    comment: &comment,
                    earlier: &[],
                    mentioned_before: &[],
                    edit: false,
                },
            )
            .await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                page_id = %page_id,
                thread_id = %thread.id,
                comment_id = %comment.id,
                inline = thread.anchor.is_some(),
                "thread started"
            );
            self.posted(&context, notifier, &thread, comment.id, told.unseen)
                .await
        })
        .await
    }

    /// Replies in a thread, for whoever may comment on its page.
    pub async fn reply(
        &self,
        principal: &Principal,
        thread_id: Uuid,
        markdown: &str,
    ) -> Result<Posted, CommentError> {
        self.observe("reply", async {
            let body = check_comment(markdown)?;
            self.limit(principal).await?;
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let thread = repo::lock_thread(&mut *tx, thread_id)
                .await?
                .ok_or(CommentError::NotFound(NO_THREAD))?;
            let context = self
                .inner
                .pages
                .context(&mut tx, principal, thread.page_id)
                .await?;
            check(&context, Action::CommentPage).map_err(hide_as(NO_THREAD))?;
            let earlier = repo::comments_of_thread(&mut *tx, thread_id).await?;
            if earlier.len() >= MAX_REPLIES {
                return Err(CommentError::TooManyReplies);
            }
            repo::lock_page(&mut *tx, thread.page_id).await?;
            room_for_a_comment(&mut tx, thread.page_id).await?;
            let comment = new_comment(thread_id, principal, body, now);
            repo::insert_comment(&mut *tx, &comment, &words(&comment.body)).await?;
            let mut notifier = Notifier::of(&self.inner.pages);
            let told = mentions::comment(
                &mut tx,
                &self.inner.pages,
                &mut notifier,
                Written {
                    thread: &thread,
                    comment: &comment,
                    earlier: &earlier,
                    mentioned_before: &[],
                    edit: false,
                },
            )
            .await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                thread_id = %thread_id,
                comment_id = %comment.id,
                "comment replied"
            );
            self.posted(&context, notifier, &thread, comment.id, told.unseen)
                .await
        })
        .await
    }

    /// Changes a comment's text, for its author while they may still comment. Those it
    /// mentions anew are told; nobody else is told again.
    pub async fn edit(
        &self,
        principal: &Principal,
        comment_id: Uuid,
        markdown: &str,
    ) -> Result<Posted, CommentError> {
        self.observe("edit", async {
            let body = check_comment(markdown)?;
            self.limit(principal).await?;
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let (thread, context, comment) = self.found(&mut tx, principal, comment_id).await?;
            check(&context, Action::CommentPage).map_err(hide_as(NO_COMMENT))?;
            if comment.author_id != Some(principal.user.id) {
                return Err(CommentError::Forbidden(AUTHOR_ONLY));
            }
            let before = mentions::mentioned_in(&comment.body);
            repo::set_body(&mut *tx, comment.id, &body, &words(&body), now).await?;
            let edited = Comment {
                body,
                edited_at: Some(now),
                ..comment
            };
            let mut notifier = Notifier::of(&self.inner.pages);
            let told = mentions::comment(
                &mut tx,
                &self.inner.pages,
                &mut notifier,
                Written {
                    thread: &thread,
                    comment: &edited,
                    earlier: &[],
                    mentioned_before: &before,
                    edit: true,
                },
            )
            .await?;
            tx.commit().await?;
            info!(actor_id = %principal.user.id, comment_id = %comment_id, "comment edited");
            self.posted(&context, notifier, &thread, comment_id, told.unseen)
                .await
        })
        .await
    }

    /// Deletes a comment, for its author while they may still comment, or for whoever edits
    /// its page, with its notifications; a thread left without comments goes too. Says
    /// whether the thread is still there.
    pub async fn delete(
        &self,
        principal: &Principal,
        comment_id: Uuid,
    ) -> Result<bool, CommentError> {
        self.observe("delete", async {
            let mut tx = self.pool().begin().await?;
            let (thread, context, comment) = self.found(&mut tx, principal, comment_id).await?;
            if comment.author_id == Some(principal.user.id) {
                check(&context, Action::CommentPage).map_err(hide_as(NO_COMMENT))?;
            } else {
                check(&context, Action::EditPage)
                    .map_err(hide_as(NO_COMMENT))
                    .map_err(|error| match error {
                        CommentError::Forbidden(_) => CommentError::Forbidden(NOT_YOURS),
                        other => other,
                    })?;
            }
            // Its notifications go with it: their inboxes hear so.
            let mut notifier = Notifier::of(&self.inner.pages);
            for user in notifications_repo::told_of_comment(&mut *tx, comment.id).await? {
                notifier.touched(user);
            }
            repo::delete_comment(&mut *tx, comment.id).await?;
            let gone = repo::delete_thread_if_empty(&mut *tx, thread.id).await?;
            tx.commit().await?;
            info!(
                actor_id = %principal.user.id,
                comment_id = %comment_id,
                thread_deleted = gone,
                "comment deleted"
            );
            notifier.tell();
            self.changed(thread.page_id, thread.id);
            Ok(!gone)
        })
        .await
    }

    /// Deletes a thread with every comment in it and their notifications, for whoever edits
    /// its page.
    pub async fn delete_thread(
        &self,
        principal: &Principal,
        thread_id: Uuid,
    ) -> Result<(), CommentError> {
        self.observe("delete_thread", async {
            let mut tx = self.pool().begin().await?;
            let thread = repo::lock_thread(&mut *tx, thread_id)
                .await?
                .ok_or(CommentError::NotFound(NO_THREAD))?;
            let context = self
                .inner
                .pages
                .context(&mut tx, principal, thread.page_id)
                .await
                .map_err(CommentError::from)
                .map_err(hide_as(NO_THREAD))?;
            check(&context, Action::EditPage)
                .map_err(hide_as(NO_THREAD))
                .map_err(|error| match error {
                    CommentError::Forbidden(_) => CommentError::Forbidden(EDITORS_ONLY),
                    other => other,
                })?;
            let mut notifier = Notifier::of(&self.inner.pages);
            for user in notifications_repo::told_of_thread(&mut *tx, thread_id).await? {
                notifier.touched(user);
            }
            repo::delete_thread(&mut *tx, thread_id).await?;
            tx.commit().await?;
            info!(actor_id = %principal.user.id, thread_id = %thread_id, "thread deleted");
            notifier.tell();
            self.changed(thread.page_id, thread_id);
            Ok(())
        })
        .await
    }

    /// Resolves a thread, or reopens it, for whoever may comment on its page, while the
    /// page has room for one more open thread. Answers it, and whether the caller knows no
    /// other member.
    pub async fn set_resolved(
        &self,
        principal: &Principal,
        thread_id: Uuid,
        resolved: bool,
    ) -> Result<(ThreadView, bool), CommentError> {
        self.observe(if resolved { "resolve" } else { "reopen" }, async {
            let mut tx = self.pool().begin().await?;
            let thread = repo::lock_thread(&mut *tx, thread_id)
                .await?
                .ok_or(CommentError::NotFound(NO_THREAD))?;
            let context = self
                .inner
                .pages
                .context(&mut tx, principal, thread.page_id)
                .await?;
            check(&context, Action::CommentPage).map_err(hide_as(NO_THREAD))?;
            if thread.resolved_at.is_some() != resolved {
                if !resolved {
                    repo::lock_page(&mut *tx, thread.page_id).await?;
                    if repo::count_open_threads(&mut *tx, thread.page_id).await?
                        >= count(MAX_THREADS)
                    {
                        return Err(CommentError::TooManyThreads);
                    }
                }
                let at = resolved.then(|| (self.now(), principal.user.id));
                repo::set_resolved(&mut *tx, thread_id, at).await?;
            }
            tx.commit().await?;
            info!(actor_id = %principal.user.id, thread_id = %thread_id, resolved, "thread resolved or reopened");
            self.changed(thread.page_id, thread_id);
            Ok((self.view(thread_id).await?, context.members_hidden()))
        })
        .await
    }

    /// Follows a page's comments, or stops, for whoever may see it.
    pub async fn follow(
        &self,
        principal: &Principal,
        page_id: Uuid,
        follows: bool,
    ) -> Result<bool, CommentError> {
        self.observe("follow", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, page_id)
                .await?;
            check(&context, Action::ViewPage)?;
            let page = context.page();
            repo::set_follow(
                &mut *conn,
                page.workspace_id,
                page_id,
                principal.user.id,
                follows,
                self.now(),
            )
            .await?;
            Ok(follows)
        })
        .await
    }

    /// The people, agents and pages a mention on a page may name, whose names or titles
    /// hold `query`, ignoring case: at most [`MAX_CANDIDATES`] of each, people each with
    /// whether they may see the page, and pages among those the caller may see. A guest,
    /// who knows no other member, finds pages alone, of those shared with them.
    pub async fn candidates(
        &self,
        principal: &Principal,
        page_id: Uuid,
        query: &str,
    ) -> Result<Candidates, CommentError> {
        self.observe("candidates", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, page_id)
                .await?;
            check(&context, Action::ViewPage)?;
            let page = context.page().clone();
            let query = query.trim().to_lowercase();
            let mut people = Vec::new();
            if !context.members_hidden() {
                let members = workspaces::list_members(&mut *conn, page.workspace_id).await?;
                for member in members {
                    if people.len() >= MAX_CANDIDATES {
                        break;
                    }
                    let person = member.person;
                    if member.suspended
                        || person.kind == UserKind::Integration
                        || !person.display_name.to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    let sees_page = sees(
                        &self.inner.pages,
                        &mut conn,
                        person.id,
                        page.workspace_id,
                        page.id,
                    )
                    .await?;
                    people.push(Candidate { person, sees_page });
                }
            }
            // Among the pages the caller may see alone, so hidden ones neither crowd out
            // those they may find nor show in how long it takes.
            let ids = pages_repo::titled_for(
                &mut *conn,
                page.workspace_id,
                principal.user.id,
                context.is_guest(),
                &query,
                count(MAX_CANDIDATES * 2),
            )
            .await?;
            let seen = Facts::read(&mut conn, principal, &ids)
                .await?
                .seen(principal)?;
            let pages = ids
                .iter()
                .filter_map(|id| seen.get(id).cloned().flatten())
                .take(MAX_CANDIDATES)
                .collect();
            Ok(Candidates { people, pages })
        })
        .await
    }

    /// A comment with its thread, locked, and its page, for a change: hidden when the
    /// principal may not see the page. What they may do is for the caller to ask.
    async fn found(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        comment_id: Uuid,
    ) -> Result<(Thread, Context, Comment), CommentError> {
        let comment = repo::comment(&mut *conn, comment_id)
            .await?
            .ok_or(CommentError::NotFound(NO_COMMENT))?;
        let thread = repo::lock_thread(&mut *conn, comment.thread_id)
            .await?
            .ok_or(CommentError::NotFound(NO_COMMENT))?;
        let context = self
            .inner
            .pages
            .context(&mut *conn, principal, thread.page_id)
            .await
            .map_err(CommentError::from)
            .map_err(hide_as(NO_COMMENT))?;
        check(&context, Action::ViewPage).map_err(hide_as(NO_COMMENT))?;
        Ok((thread, context, comment))
    }

    /// Counts a comment the principal writes against their rate.
    async fn limit(&self, principal: &Principal) -> Result<(), CommentError> {
        self.inner
            .limiter
            .check(Bucket::Comment, &principal.user.id.to_string())
            .await
            .map_err(|limited| {
                warn!(user_id = %principal.user.id, "comments rate limited");
                CommentError::RateLimited {
                    retry_after: limited.retry_after,
                }
            })
    }

    /// An anchor over a block's text, for a thread on the block, read from the page's
    /// document once the principal may see the page.
    async fn block_anchor(
        &self,
        principal: &Principal,
        page_id: Uuid,
        block_id: String,
    ) -> Result<Anchor, CommentError> {
        {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, page_id)
                .await?;
            check(&context, Action::ViewPage)?;
        }
        let found = self
            .inner
            .sync
            .block_anchor(page_id, block_id)
            .await?
            .ok_or_else(|| {
                CommentError::InvalidInput(
                    "the page has no block of that id with text to comment on: comment on the \
                     page instead"
                        .to_owned(),
                )
            })?;
        Ok(check_anchor(found.start, found.end, &found.quote)?)
    }

    /// The answer to a comment made or changed, once it committed: the page's open
    /// editors and the inboxes it told are told.
    async fn posted(
        &self,
        context: &Context,
        notifier: Notifier,
        thread: &Thread,
        comment_id: Uuid,
        unseen: Vec<Uuid>,
    ) -> Result<Posted, CommentError> {
        notifier.tell();
        self.changed(thread.page_id, thread.id);
        let members_hidden = context.members_hidden();
        let unnotified = if members_hidden || unseen.is_empty() {
            Vec::new()
        } else {
            let mut conn = self.pool().acquire().await?;
            let mut people = Vec::new();
            for user in unseen {
                if let Some(member) =
                    workspaces::member(&mut *conn, thread.workspace_id, user).await?
                {
                    people.push(member.person);
                }
            }
            people
        };
        Ok(Posted {
            thread: self.view(thread.id).await?,
            comment_id,
            unnotified,
            members_hidden,
        })
    }

    /// Tells the page's open editors that a thread changed.
    fn changed(&self, page_id: Uuid, thread_id: Uuid) {
        self.inner.pages.broadcaster().broadcast(
            Room::Comments(page_id),
            comments_changed(thread_id),
            None,
        );
    }

    /// A thread as it is now, with its first comment and its latest, and its anchor.
    async fn view(&self, thread_id: Uuid) -> Result<ThreadView, CommentError> {
        let mut conn = self.pool().acquire().await?;
        let thread = repo::thread(&mut *conn, thread_id)
            .await?
            .ok_or(CommentError::NotFound(NO_THREAD))?;
        let threads = vec![thread];
        let listed = listed(&mut conn, &threads).await?;
        drop(conn);
        let page_id = threads[0].page_id;
        self.assemble(page_id, threads, listed)
            .await?
            .pop()
            .ok_or(CommentError::NotFound(NO_THREAD))
    }

    /// Threads with the comments listed of them, and their anchors resolved in the page's
    /// document.
    async fn assemble(
        &self,
        page_id: Uuid,
        threads: Vec<Thread>,
        listed: Listed,
    ) -> Result<Vec<ThreadView>, CommentError> {
        let anchors: Vec<(Vec<u8>, Vec<u8>)> = threads
            .iter()
            .filter_map(|thread| thread.anchor.as_ref())
            .map(|anchor| (anchor.start.clone(), anchor.end.clone()))
            .collect();
        let mut resolved = self.inner.sync.anchors(page_id, anchors).await?.into_iter();
        let Listed {
            written,
            mut counts,
        } = listed;
        let mut by_thread: HashMap<Uuid, Vec<CommentView>> = HashMap::new();
        for written in written {
            by_thread
                .entry(written.comment.thread_id)
                .or_default()
                .push(view_of(written));
        }
        Ok(threads
            .into_iter()
            .map(|thread| ThreadView {
                anchored: thread.anchor.as_ref().and_then(|_| resolved.next()),
                comments: by_thread.remove(&thread.id).unwrap_or_default(),
                count: counts.remove(&thread.id).unwrap_or_default(),
                thread,
            })
            .collect())
    }
}

/// The comments a listing shows of some threads, and how many each holds.
struct Listed {
    written: Vec<repo::Written>,
    counts: HashMap<Uuid, usize>,
}

async fn listed(conn: &mut PgConnection, threads: &[Thread]) -> Result<Listed, CommentError> {
    let ids: Vec<Uuid> = threads.iter().map(|thread| thread.id).collect();
    let latest = count(LISTED_COMMENTS - 1);
    Ok(Listed {
        written: repo::listed_comments(&mut *conn, &ids, latest).await?,
        counts: repo::comment_counts(&mut *conn, &ids)
            .await?
            .into_iter()
            .map(|(thread, count)| (thread, usize::try_from(count).unwrap_or_default()))
            .collect(),
    })
}

/// A comment as its readers see it.
fn view_of(written: repo::Written) -> CommentView {
    let content = inline_from_markdown(&written.comment.body);
    CommentView {
        comment: written.comment,
        author: written.author,
        content,
    }
}

/// A thread and its page, for whoever may see the page; hidden as not found otherwise.
async fn readable(
    pages: &PageService,
    conn: &mut PgConnection,
    principal: &Principal,
    thread_id: Uuid,
) -> Result<(Thread, Context), CommentError> {
    let thread = repo::thread(&mut *conn, thread_id)
        .await?
        .ok_or(CommentError::NotFound(NO_THREAD))?;
    let context = pages
        .context(&mut *conn, principal, thread.page_id)
        .await
        .map_err(CommentError::from)
        .map_err(hide_as(NO_THREAD))?;
    check(&context, Action::ViewPage).map_err(hide_as(NO_THREAD))?;
    Ok((thread, context))
}

/// Refuses a comment more on a page that holds [`MAX_PAGE_COMMENTS`], under its lock.
async fn room_for_a_comment(conn: &mut PgConnection, page_id: Uuid) -> Result<(), CommentError> {
    if repo::count_page_comments(&mut *conn, page_id).await? >= count(MAX_PAGE_COMMENTS) {
        return Err(CommentError::TooManyComments);
    }
    Ok(())
}

/// How many of a listing's part to read.
fn part(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PART).clamp(1, MAX_PART)
}

fn count(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// A comment's words, for search: its text without styles or link targets, a mention as
/// `@` and its label, as a page's text reads.
fn words(body: &str) -> String {
    Inline::plain_text(&inline_from_markdown(body))
}

fn new_comment(
    thread_id: Uuid,
    principal: &Principal,
    body: String,
    at: time::OffsetDateTime,
) -> Comment {
    Comment {
        id: Uuid::now_v7(),
        thread_id,
        author_id: Some(principal.user.id),
        body,
        created_at: at,
        edited_at: None,
    }
}

/// Asks the rules about an action on the page.
fn check(context: &Context, action: Action) -> Result<(), CommentError> {
    Ok(context.check(action)?)
}

/// A refusal of the rules, the page hidden reading as the thing on it not found.
fn hide_as(missing: &'static str) -> impl Fn(CommentError) -> CommentError {
    move |error| match error {
        CommentError::NotFound(_) => CommentError::NotFound(missing),
        other => other,
    }
}

/// Whether the principal may do `action` with the page now, with the scope to write.
fn may(principal: &Principal, context: &Context, action: Action) -> bool {
    principal.can(Scope::Write) && context.check(action).is_ok()
}

/// Whether someone follows a page's comments: by choice, or as its maker unless they
/// stopped.
async fn following(
    conn: &mut PgConnection,
    page: &neon_domain::pages::Page,
    user_id: Uuid,
) -> Result<bool, CommentError> {
    Ok(
        match repo::follow_choice(&mut *conn, page.id, user_id).await? {
            Some(choice) => choice,
            None => page.created_by == Some(user_id),
        },
    )
}
