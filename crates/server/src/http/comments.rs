//! Comments on pages: `/api/v1/pages/{id}/comments`, `/api/v1/threads/{id}`,
//! `/api/v1/comments/{id}`, following a page's comments, and what a mention may name.
//!
//! Every route needs a browser session or an API token: `read` for `GET`, `write` for the
//! rest. Reading a page's comments needs to see it; commenting needs edit access, or a
//! share to comment. A comment is changed by its author alone, and deleted by its author
//! or those who edit the page, who also delete whole threads. Listings come a part at a
//! time: a page's threads, each with its first comment and its latest, and the rest of a
//! thread's comments.

use axum::extract::State;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_app::comments::{
    Candidates, CommentView, PageThreads, Placing, Posted, ThreadComments, ThreadQuery, ThreadView,
};
use neon_blocks::Inline;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::dto::Person;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json as JsonBody, Path, Query};
use super::pages::Page;
use super::principal::Authenticated;

/// Where an inline thread is in the page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AnchorOut {
    /// The Yjs relative position of the start of its range, as Yjs encodes it, in base64.
    pub start: String,
    /// The Yjs relative position of its end, likewise.
    pub end: String,
    /// The text it was made on.
    pub quote: String,
    /// The block its text starts in now; absent once it is detached.
    pub block_id: Option<String>,
    /// Every character of its range was deleted: the thread stays, without a place.
    pub detached: bool,
}

/// A comment, its markdown and its inline content.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommentOut {
    pub id: Uuid,
    pub thread_id: Uuid,
    /// Who wrote it; absent once their account is gone.
    pub author: Option<Person>,
    /// Its markdown: inline content alone, mentions written `@[Name](user:<id>)` and
    /// `@[Title](page:<id>)`.
    pub markdown: String,
    /// The same as BlockNote's inline content: text with its styles, links and mentions.
    #[schema(value_type = Vec<Object>)]
    pub content: Json,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When its author last changed it, which shows it as edited.
    #[serde(with = "time::serde::rfc3339::option")]
    pub edited_at: Option<OffsetDateTime>,
}

/// A thread of comments: on the page as a whole, or on a range of its text.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ThreadOut {
    pub id: Uuid,
    pub page_id: Uuid,
    /// Where it is, for an inline thread; absent for one on the whole page.
    pub anchor: Option<AnchorOut>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    pub resolved_by: Option<Uuid>,
    pub created_by: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Its first comment, which starts it, and its latest replies after it, in order, at
    /// most 10 in all; `GET /threads/{id}/comments` reads the rest.
    pub comments: Vec<CommentOut>,
    /// How many comments it holds in all.
    pub comment_count: usize,
}

/// A part of a page's threads, oldest first, and what the caller may do with them.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ThreadList {
    pub threads: Vec<ThreadOut>,
    /// Pass as `after` for the next part; absent at the end.
    pub next: Option<Uuid>,
    /// How many threads of the page are open, in every part.
    pub open_threads: usize,
    /// The caller may start threads, reply, resolve and reopen them.
    pub may_comment: bool,
    /// The caller edits the page: they delete others' comments, and whole threads.
    pub may_moderate: bool,
    /// The caller follows the page's comments.
    pub following: bool,
}

/// A part of a thread's comments, in order.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommentList {
    pub comments: Vec<CommentOut>,
    /// Pass as `after` for the next part; absent at the end.
    pub next: Option<Uuid>,
}

/// A comment just made or changed, with its thread, and those it mentions who may not see
/// the page: they were told nothing, and sharing the page with them would let them.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PostedComment {
    pub comment_id: Uuid,
    pub thread: ThreadOut,
    pub unnotified: Vec<Person>,
}

fn person(person: neon_domain::workspaces::Person, members_hidden: bool) -> Person {
    let mut shown = Person::from(person);
    if members_hidden {
        shown.email = None;
    }
    shown
}

fn content_json(content: &[Inline]) -> Json {
    serde_json::to_value(content).unwrap_or(Json::Array(Vec::new()))
}

pub(crate) fn comment_out(view: CommentView, members_hidden: bool) -> CommentOut {
    CommentOut {
        id: view.comment.id,
        thread_id: view.comment.thread_id,
        author: view.author.map(|author| person(author, members_hidden)),
        content: content_json(&view.content),
        markdown: view.comment.body,
        created_at: view.comment.created_at,
        edited_at: view.comment.edited_at,
    }
}

pub(crate) fn thread_out(view: ThreadView, members_hidden: bool) -> ThreadOut {
    let anchored = view.anchored;
    ThreadOut {
        id: view.thread.id,
        page_id: view.thread.page_id,
        anchor: view.thread.anchor.map(|anchor| AnchorOut {
            start: STANDARD.encode(&anchor.start),
            end: STANDARD.encode(&anchor.end),
            quote: anchor.quote,
            block_id: anchored.as_ref().and_then(|at| at.block_id.clone()),
            detached: anchored.as_ref().is_none_or(|at| at.detached),
        }),
        resolved_at: view.thread.resolved_at,
        resolved_by: view.thread.resolved_by.filter(|_| !members_hidden),
        created_by: view.thread.created_by.filter(|_| !members_hidden),
        created_at: view.thread.created_at,
        comments: view
            .comments
            .into_iter()
            .map(|comment| comment_out(comment, members_hidden))
            .collect(),
        comment_count: view.count,
    }
}

impl From<PageThreads> for ThreadList {
    fn from(list: PageThreads) -> Self {
        let hidden = list.members_hidden;
        Self {
            threads: list
                .threads
                .into_iter()
                .map(|thread| thread_out(thread, hidden))
                .collect(),
            next: list.next,
            open_threads: list.open,
            may_comment: list.may_comment,
            may_moderate: list.may_moderate,
            following: list.following,
        }
    }
}

impl From<ThreadComments> for CommentList {
    fn from(part: ThreadComments) -> Self {
        let hidden = part.members_hidden;
        Self {
            comments: part
                .comments
                .into_iter()
                .map(|comment| comment_out(comment, hidden))
                .collect(),
            next: part.next,
        }
    }
}

impl From<Posted> for PostedComment {
    fn from(posted: Posted) -> Self {
        let hidden = posted.members_hidden;
        Self {
            comment_id: posted.comment_id,
            thread: thread_out(posted.thread, hidden),
            unnotified: posted
                .unnotified
                .into_iter()
                .map(|who| person(who, hidden))
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListThreads {
    /// `true` for the resolved threads alone, `false` for the open ones alone; both when
    /// absent.
    pub resolved: Option<bool>,
    /// The `next` of the part before.
    pub after: Option<Uuid>,
    /// Threads in the part, 1 to 100; 50 when absent.
    pub limit: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/pages/{id}/comments",
    tag = "comments",
    summary = "List a page's comments",
    description = "Its threads, oldest first, a part at a time, each with its first comment and \
                   its latest replies, how many it holds, and, for an inline thread, where its \
                   anchor is in the page now: the block its text starts in, or detached once \
                   that text is gone. For everyone who may see the page.",
    params(("id" = Uuid, Path, description = "The page's id."), ListThreads),
    responses(
        (status = OK, description = "The threads.", body = ThreadList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_comments(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListThreads>,
) -> Result<JsonBody<ThreadList>, ApiError> {
    let query = ThreadQuery {
        resolved: query.resolved,
        after: query.after,
        limit: query.limit,
    };
    let threads = state.comments.threads(&principal, id, query).await?;
    Ok(JsonBody(threads.into()))
}

#[utoipa::path(
    get,
    path = "/threads/{id}",
    tag = "comments",
    summary = "Get a thread",
    description = "With its first comment and its latest replies, as a page's list has it. For \
                   everyone who may see its page.",
    params(("id" = Uuid, Path, description = "The thread's id.")),
    responses(
        (status = OK, description = "The thread.", body = ThreadOut),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn get_thread(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<JsonBody<ThreadOut>, ApiError> {
    let (thread, members_hidden) = state.comments.thread(&principal, id).await?;
    Ok(JsonBody(thread_out(thread, members_hidden)))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListComments {
    /// The `next` of the part before.
    pub after: Option<Uuid>,
    /// Comments in the part, 1 to 100; 50 when absent.
    pub limit: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/threads/{id}/comments",
    tag = "comments",
    summary = "List a thread's comments",
    description = "Every comment of the thread, in order, a part at a time: what the page's list \
                   leaves out of a long thread. For everyone who may see its page.",
    params(("id" = Uuid, Path, description = "The thread's id."), ListComments),
    responses(
        (status = OK, description = "The comments.", body = CommentList),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn list_thread_comments(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<ListComments>,
) -> Result<JsonBody<CommentList>, ApiError> {
    let part = state
        .comments
        .comments(&principal, id, query.after, query.limit)
        .await?;
    Ok(JsonBody(part.into()))
}

/// An inline thread's range, as the editor makes it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AnchorIn {
    /// The Yjs relative position of its start, as Yjs encodes it, in base64.
    pub start: String,
    /// The Yjs relative position of its end, likewise.
    pub end: String,
    /// The text it is on, at most 1000 characters kept.
    pub quote: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewThread {
    /// The first comment: markdown limited to inline content, at most 10000 characters.
    pub markdown: String,
    /// For an inline thread, the range of text it is on.
    #[serde(default)]
    pub anchor: Option<AnchorIn>,
    /// For a thread on one block's whole text, the block's id; not with `anchor`.
    #[serde(default)]
    pub block_id: Option<String>,
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", message)
}

fn decode(field: &str, text: &str) -> Result<Vec<u8>, ApiError> {
    STANDARD
        .decode(text)
        .map_err(|_| invalid(format!("`{field}` is base64")))
}

#[utoipa::path(
    post,
    path = "/pages/{id}/comments",
    tag = "comments",
    summary = "Start a thread of comments",
    description = "On the whole page, on a range of its text (`anchor`), or on one block's text \
                   (`block_id`), with its first comment. For those who edit the page or may \
                   comment on it. Those it mentions who may see the page, and the page's \
                   followers, are told in their inboxes; those it mentions who may not are \
                   told nothing, and come back in `unnotified`.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    request_body = NewThread,
    responses(
        (status = CREATED, description = "The thread.", body = PostedComment),
        (status = BAD_REQUEST, description = "invalid_input: an empty or long comment, a bad anchor, or a block without text.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller may only view the page, or it is in the trash; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "too_many_threads, or too_many_comments.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited: too many comments at once; Retry-After says when to try again.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn start_thread(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    JsonBody(body): JsonBody<NewThread>,
) -> Result<(StatusCode, JsonBody<PostedComment>), ApiError> {
    let placing = match (body.anchor, body.block_id) {
        (Some(_), Some(_)) => {
            return Err(invalid("name `anchor` or `block_id`, not both"));
        }
        (Some(anchor), None) => Placing::Text {
            start: decode("anchor.start", &anchor.start)?,
            end: decode("anchor.end", &anchor.end)?,
            quote: anchor.quote,
        },
        (None, Some(block)) => Placing::Block(block),
        (None, None) => Placing::Page,
    };
    let posted = state
        .comments
        .start_thread(&principal, id, &body.markdown, placing)
        .await?;
    Ok((StatusCode::CREATED, JsonBody(posted.into())))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewComment {
    /// Markdown limited to inline content, at most 10000 characters.
    pub markdown: String,
}

#[utoipa::path(
    post,
    path = "/threads/{id}/comments",
    tag = "comments",
    summary = "Reply in a thread",
    description = "For those who may comment on its page. Those it mentions, those who wrote in \
                   the thread or were mentioned in it, and the page's followers are told, each \
                   once, while they may see the page.",
    params(("id" = Uuid, Path, description = "The thread's id.")),
    request_body = NewComment,
    responses(
        (status = CREATED, description = "The reply, with its thread.", body = PostedComment),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "too_many_replies, or too_many_comments.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn reply(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    JsonBody(body): JsonBody<NewComment>,
) -> Result<(StatusCode, JsonBody<PostedComment>), ApiError> {
    let posted = state.comments.reply(&principal, id, &body.markdown).await?;
    Ok((StatusCode::CREATED, JsonBody(posted.into())))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadChange {
    /// Resolves the thread, or reopens it.
    pub resolved: bool,
}

#[utoipa::path(
    patch,
    path = "/threads/{id}",
    tag = "comments",
    summary = "Resolve or reopen a thread",
    description = "For those who may comment on its page. Reopening one needs room on the page \
                   for one more open thread.",
    params(("id" = Uuid, Path, description = "The thread's id.")),
    request_body = ThreadChange,
    responses(
        (status = OK, description = "The thread.", body = ThreadOut),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = CONFLICT, description = "too_many_threads, to reopen one.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn change_thread(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    JsonBody(body): JsonBody<ThreadChange>,
) -> Result<JsonBody<ThreadOut>, ApiError> {
    let (thread, members_hidden) = state
        .comments
        .set_resolved(&principal, id, body.resolved)
        .await?;
    Ok(JsonBody(thread_out(thread, members_hidden)))
}

#[utoipa::path(
    delete,
    path = "/threads/{id}",
    tag = "comments",
    summary = "Delete a thread",
    description = "With every comment in it and their notifications, by those who edit its page.",
    params(("id" = Uuid, Path, description = "The thread's id.")),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: the caller does not edit the page; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_thread(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.comments.delete_thread(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/comments/{id}",
    tag = "comments",
    summary = "Change a comment",
    description = "Its text, by its author, who must still be allowed to comment. It shows as \
                   edited. Those it mentions anew are told; nobody else is told again.",
    params(("id" = Uuid, Path, description = "The comment's id.")),
    request_body = NewComment,
    responses(
        (status = OK, description = "The comment, with its thread.", body = PostedComment),
        (status = BAD_REQUEST, description = "invalid_input.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: not its author, or may not comment; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn edit_comment(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    JsonBody(body): JsonBody<NewComment>,
) -> Result<JsonBody<PostedComment>, ApiError> {
    let posted = state.comments.edit(&principal, id, &body.markdown).await?;
    Ok(JsonBody(posted.into()))
}

#[utoipa::path(
    delete,
    path = "/comments/{id}",
    tag = "comments",
    summary = "Delete a comment",
    description = "By its author, who must still be allowed to comment, or by those who edit \
                   the page, with the notifications of it. A thread left without comments goes \
                   too.",
    params(("id" = Uuid, Path, description = "The comment's id.")),
    responses(
        (status = NO_CONTENT, description = "Deleted."),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn delete_comment(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.comments.delete(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Following {
    /// Whether the caller hears of every comment on the page.
    pub following: bool,
}

#[utoipa::path(
    put,
    path = "/pages/{id}/follow",
    tag = "comments",
    summary = "Follow a page's comments, or stop",
    description = "Who follows a page hears of every comment on it in their inbox. Its maker, \
                   and whoever comments on it or is mentioned on it, follows it until they stop.",
    params(("id" = Uuid, Path, description = "The page's id.")),
    request_body = Following,
    responses(
        (status = OK, description = "Whether the caller follows it now.", body = Following),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn follow_page(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    JsonBody(body): JsonBody<Following>,
) -> Result<JsonBody<Following>, ApiError> {
    let following = state
        .comments
        .follow(&principal, id, body.following)
        .await?;
    Ok(JsonBody(Following { following }))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct MentionQuery {
    /// What their name or title holds, ignoring case; every one when empty.
    #[serde(default)]
    pub q: String,
}

/// Someone a mention may name, and whether they may see the page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MentionPerson {
    pub person: Person,
    /// Off, a mention of them tells them nothing: share the page with them first.
    pub sees_page: bool,
}

/// What a mention on a page may name.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MentionCandidates {
    /// People and agents, at most 10; none for a guest.
    pub people: Vec<MentionPerson>,
    /// Pages the caller sees, at most 10, the most recently changed first.
    pub pages: Vec<Page>,
}

impl From<Candidates> for MentionCandidates {
    fn from(found: Candidates) -> Self {
        Self {
            people: found
                .people
                .into_iter()
                .map(|candidate| MentionPerson {
                    person: candidate.person.into(),
                    sees_page: candidate.sees_page,
                })
                .collect(),
            pages: found.pages.into_iter().map(Page::from).collect(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/pages/{id}/mentions",
    tag = "comments",
    summary = "Find what a mention may name",
    description = "The members and agents, and the pages, a mention on the page may name, whose \
                   names or titles hold `q`, each person with whether they may see the page. For \
                   everyone who may see it; a guest finds pages alone.",
    params(("id" = Uuid, Path, description = "The page's id."), MentionQuery),
    responses(
        (status = OK, description = "What it may name.", body = MentionCandidates),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn mention_candidates(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(query): Query<MentionQuery>,
) -> Result<JsonBody<MentionCandidates>, ApiError> {
    let found = state.comments.candidates(&principal, id, &query.q).await?;
    Ok(JsonBody(found.into()))
}
