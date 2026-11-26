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
//!   changed and deleted by its author alone, while they may still comment. An agent or
//!   integration does so only as far as its owner may too.
//! - **Telling.** A new comment tells, in its transaction, those it mentions, those in its
//!   thread and the page's followers, each once and only who may see the page; one it
//!   mentions who may not is told nothing, and its writer hears so. An edit tells only
//!   those it mentions anew. See [`crate::notifications`].
//! - **Live.** Once a change commits, the page's room of comments hears which thread
//!   changed, and the open editors read the comments again.
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
use neon_domain::comments::{Anchor, MAX_REPLIES, MAX_THREADS, check_anchor, check_comment};
use neon_domain::workspaces::{Action, Person};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::comments::{self as repo, Comment, Thread};
use neon_store::{notifications as notifications_repo, pages as pages_repo, workspaces};
use sqlx::{PgConnection, PgPool};
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use crate::notifications::mentions::{self, Written};
use crate::notifications::notify::{Notifier, sees};
use crate::pages::{Context, Facts, PageService, PageView};
use crate::sync::{Room, SyncService, comments_changed};

pub use error::CommentError;

/// RED subsystem of the comment use cases.
pub const SUBSYSTEM: &str = "comments";

/// The most people and pages a search for mentions answers with, each.
pub const MAX_CANDIDATES: usize = 10;

const NO_THREAD: &str = "no such thread";
const NO_COMMENT: &str = "no such comment";
const AUTHOR_ONLY: &str = "only its author changes or deletes a comment";

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

/// A thread with its comments, in order, and where its anchor is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadView {
    pub thread: Thread,
    /// For an inline thread, the block its text is in and whether that text is gone.
    pub anchored: Option<Anchored>,
    pub comments: Vec<CommentView>,
}

/// A page's threads, oldest first, and what the caller may do with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageThreads {
    pub page_id: Uuid,
    pub threads: Vec<ThreadView>,
    /// The caller may comment: start threads, reply, resolve and reopen.
    pub may_comment: bool,
    /// The caller follows the page's comments.
    pub following: bool,
    /// The caller knows no other member: people show without their addresses.
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
    red: Red,
}

impl CommentService {
    /// The comments of `pages`' pages, whose anchors `sync`'s documents resolve. Call it
    /// after telemetry is installed, as it creates the RED metrics.
    pub fn new(pages: PageService, sync: SyncService) -> Self {
        Self {
            inner: Arc::new(Inner {
                pages,
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

    /// A page's threads with their comments, for whoever may see it.
    pub async fn threads(
        &self,
        principal: &Principal,
        page_id: Uuid,
    ) -> Result<PageThreads, CommentError> {
        self.observe("list", async {
            let mut conn = self.pool().acquire().await?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, page_id)
                .await?;
            check(&context, Action::ViewPage)?;
            let threads = repo::threads(&mut *conn, page_id).await?;
            let written = repo::comments_of_page(&mut *conn, page_id).await?;
            let following = following(&mut conn, context.page(), principal.user.id).await?;
            drop(conn);
            let threads = self.assemble(page_id, threads, written).await?;
            Ok(PageThreads {
                page_id,
                threads,
                may_comment: may_comment(principal, &context),
                following,
                members_hidden: context.members_hidden(),
            })
        })
        .await
    }

    /// One thread with its comments, for whoever may see its page, and whether they know no
    /// other member.
    pub async fn thread(
        &self,
        principal: &Principal,
        thread_id: Uuid,
    ) -> Result<(ThreadView, bool), CommentError> {
        self.observe("get", async {
            let mut conn = self.pool().acquire().await?;
            let thread = repo::thread(&mut *conn, thread_id)
                .await?
                .ok_or(CommentError::NotFound(NO_THREAD))?;
            let context = self
                .inner
                .pages
                .context(&mut conn, principal, thread.page_id)
                .await
                .map_err(|_| CommentError::NotFound(NO_THREAD))?;
            check(&context, Action::ViewPage).map_err(|_| CommentError::NotFound(NO_THREAD))?;
            drop(conn);
            Ok((self.view(thread_id).await?, context.members_hidden()))
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
            if repo::count_threads(&mut *tx, page_id).await? >= count(MAX_THREADS) {
                return Err(CommentError::TooManyThreads);
            }
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
            repo::insert_comment(&mut *tx, &comment).await?;
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
            let comment = new_comment(thread_id, principal, body, now);
            repo::insert_comment(&mut *tx, &comment).await?;
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
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let (thread, context, comment) = self.own(&mut tx, principal, comment_id).await?;
            let before = mentions::mentioned_in(&comment.body);
            repo::set_body(&mut *tx, comment.id, &body, now).await?;
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

    /// Deletes a comment, for its author while they may still comment, with its
    /// notifications; a thread left without comments goes too. Says whether the thread is
    /// still there.
    pub async fn delete(
        &self,
        principal: &Principal,
        comment_id: Uuid,
    ) -> Result<bool, CommentError> {
        self.observe("delete", async {
            let mut tx = self.pool().begin().await?;
            let (thread, _, comment) = self.own(&mut tx, principal, comment_id).await?;
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

    /// Resolves a thread, or reopens it, for whoever may comment on its page. Answers it,
    /// and whether the caller knows no other member.
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
    /// whether they may see the page. A guest, who knows no other member, finds pages
    /// alone.
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
            let ids = pages_repo::titled(
                &mut *conn,
                page.workspace_id,
                &query,
                i64::try_from(MAX_CANDIDATES * 5).unwrap_or(i64::MAX),
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

    /// The comment of the principal's own, with its thread and page, for a change: hidden
    /// when they may not see the page, refused when they may not comment or did not write
    /// it.
    async fn own(
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
            .await?;
        check(&context, Action::CommentPage).map_err(hide_as(NO_COMMENT))?;
        if comment.author_id != Some(principal.user.id) {
            return Err(CommentError::Forbidden(AUTHOR_ONLY));
        }
        Ok((thread, context, comment))
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

    /// A thread as it is now, with its comments and its anchor.
    async fn view(&self, thread_id: Uuid) -> Result<ThreadView, CommentError> {
        let mut conn = self.pool().acquire().await?;
        let thread = repo::thread(&mut *conn, thread_id)
            .await?
            .ok_or(CommentError::NotFound(NO_THREAD))?;
        let written: Vec<_> = repo::comments_of_page(&mut *conn, thread.page_id)
            .await?
            .into_iter()
            .filter(|written| written.comment.thread_id == thread_id)
            .collect();
        drop(conn);
        let page_id = thread.page_id;
        self.assemble(page_id, vec![thread], written)
            .await?
            .pop()
            .ok_or(CommentError::NotFound(NO_THREAD))
    }

    /// Threads with their comments, and their anchors resolved in the page's document.
    async fn assemble(
        &self,
        page_id: Uuid,
        threads: Vec<Thread>,
        written: Vec<repo::Written>,
    ) -> Result<Vec<ThreadView>, CommentError> {
        let anchors: Vec<(Vec<u8>, Vec<u8>)> = threads
            .iter()
            .filter_map(|thread| thread.anchor.as_ref())
            .map(|anchor| (anchor.start.clone(), anchor.end.clone()))
            .collect();
        let mut resolved = self.inner.sync.anchors(page_id, anchors).await?.into_iter();
        let mut by_thread: HashMap<Uuid, Vec<CommentView>> = HashMap::new();
        for repo::Written { comment, author } in written {
            let content = inline_from_markdown(&comment.body);
            by_thread
                .entry(comment.thread_id)
                .or_default()
                .push(CommentView {
                    comment,
                    author,
                    content,
                });
        }
        Ok(threads
            .into_iter()
            .map(|thread| ThreadView {
                anchored: thread.anchor.as_ref().and_then(|_| resolved.next()),
                comments: by_thread.remove(&thread.id).unwrap_or_default(),
                thread,
            })
            .collect())
    }
}

fn count(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
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

/// Whether the principal may comment on the page now, with the scope to write.
fn may_comment(principal: &Principal, context: &Context) -> bool {
    principal.can(Scope::Write) && context.check(Action::CommentPage).is_ok()
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
