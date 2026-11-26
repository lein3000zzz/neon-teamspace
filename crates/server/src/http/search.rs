//! `/api/v1/workspaces/{id}/search`: the pages, databases and rows of a workspace a query
//! finds, among those the caller may see, the best first, with their matches marked.
//!
//! Needs a session or a token with `read`. The query's syntax, what is read and how access
//! is kept to are [`neon_app::search`]'s; the MCP server's `search` tool takes the same
//! parameters and answers the same.

use axum::extract::State;
use axum::http::StatusCode;
use neon_app::search::{SearchHit, SearchInput, SearchResults as AppResults, Segment};
use neon_domain::duration;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::AppState;
use super::error::{ApiError, ErrorBody};
use super::extract::{Json, Path, Query};
use super::pages::{Page, PageKind};
use super::principal::Authenticated;

/// What to look for, and where.
#[derive(Debug, Clone, Default, Deserialize, IntoParams, JsonSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    /// Words, all of which must be there, each of two characters or more also finding the
    /// words it starts; "quoted words" in that order; -word to leave out what holds it; OR
    /// between two terms for either. Titles also match loosely, misspelled. At most 200
    /// characters. Empty or absent, the filters alone list what they let through, the most
    /// recently changed first.
    #[serde(default)]
    pub query: Option<String>,
    /// Only the pages of this teamspace.
    #[serde(default)]
    pub teamspace_id: Option<Uuid>,
    /// Only pages, databases or rows.
    #[serde(default)]
    pub kind: Option<PageKind>,
    /// Only those this member made. Guests cannot filter by member.
    #[serde(default)]
    pub created_by: Option<Uuid>,
    /// Only those this member changed last. Guests cannot filter by member.
    #[serde(default)]
    pub edited_by: Option<Uuid>,
    /// Only those changed within this long, such as `7d` or `12h`, at most ten years.
    #[serde(default)]
    #[param(example = "7d")]
    pub edited_within: Option<String>,
    /// Where the part starts: the `next` of the part before.
    #[serde(default)]
    pub after: Option<String>,
    /// Results in the part, 1 to 50; 20 when absent. At most the best 1000 are reached.
    #[serde(default)]
    #[schemars(range(min = 1, max = 50))]
    pub limit: Option<usize>,
}

impl SearchParams {
    pub fn input(self) -> Result<SearchInput, ApiError> {
        let edited_within = self
            .edited_within
            .as_deref()
            .map(|text| {
                duration::parse(text).ok_or_else(|| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "invalid_input",
                        "edited_within is a duration such as 7d or 12h",
                    )
                })
            })
            .transpose()?;
        Ok(SearchInput {
            query: self.query.unwrap_or_default(),
            teamspace_id: self.teamspace_id,
            kind: self.kind.map(Into::into),
            created_by: self.created_by,
            edited_by: self.edited_by,
            edited_within,
            after: self.after,
            limit: self.limit,
        })
    }
}

/// A run of text: a match of the query, or what is between.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TextRun {
    pub text: String,
    /// It is what the query matched.
    pub highlight: bool,
}

fn runs(segments: Vec<Segment>) -> Vec<TextRun> {
    segments
        .into_iter()
        .map(|segment| TextRun {
            text: segment.text,
            highlight: segment.highlight,
        })
        .collect()
}

/// A page, database or row found.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SearchResult {
    pub page: Page,
    /// Its title, in runs that mark what matched; empty without a query.
    pub title: Vec<TextRun>,
    /// A few words of a row's property values and the text around the matches, in runs
    /// that mark them, on one line: ` · ` stands between its lines, such as a page's blocks.
    /// Empty without a query, or without text.
    pub snippet: Vec<TextRun>,
    /// How well it matched, higher being better, as the results are ordered: worth
    /// comparing within one search only. 0 without a query.
    pub rank: f64,
}

impl From<SearchHit> for SearchResult {
    fn from(hit: SearchHit) -> Self {
        Self {
            page: hit.page.into(),
            title: runs(hit.title),
            snippet: runs(hit.snippet),
            rank: hit.rank,
        }
    }
}

/// A part of the results, the best first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    /// Pass it as `after` for the next part; absent on the last one.
    pub next: Option<String>,
}

impl From<AppResults> for SearchResults {
    fn from(results: AppResults) -> Self {
        Self {
            results: results.hits.into_iter().map(SearchResult::from).collect(),
            next: results.next,
        }
    }
}

#[utoipa::path(
    get,
    path = "/workspaces/{id}/search",
    tag = "search",
    summary = "Search a workspace",
    description = "The pages, databases and rows of the workspace that the query finds, among \
                   those the caller may see, out of the trash, the best first: a match in a \
                   title ranks above one in a row's property values, which ranks above one in \
                   the text, which ranks above one in its comments. Each comes with its title \
                   and a snippet, in runs that mark the matches. Read are titles, rows' text \
                   and URL values, the text of pages and rows, as the server last saved it, \
                   which is a moment after an edit, and the text of their comments, as they \
                   are written. \
                   Filters narrow it to a teamspace, a kind, who made or last changed it, and \
                   how recently it changed. A snippet is drawn from the 5000 characters around \
                   the first place the query's words are. Each user's searches are rate \
                   limited, a few run at once, and each gives up after 5 seconds.",
    params(("id" = Uuid, Path, description = "The workspace's id."), SearchParams),
    responses(
        (status = OK, description = "The results, or a part of them.", body = SearchResults),
        (status = BAD_REQUEST, description = "invalid_search: a query too long, with more than 16 terms, with only words to leave out, or with a control character; invalid_input: a limit out of bounds, a cursor from no search, or edited_within no duration or out of bounds; invalid_query: a parameter that does not read.", body = ErrorBody),
        (status = UNAUTHORIZED, description = "unauthenticated.", body = ErrorBody),
        (status = FORBIDDEN, description = "forbidden: a guest filtering by member; or insufficient_scope.", body = ErrorBody),
        (status = NOT_FOUND, description = "not_found: no such workspace, or one the caller is not in.", body = ErrorBody),
        (status = TOO_MANY_REQUESTS, description = "rate_limited: too many searches; Retry-After says when to try again.", body = ErrorBody),
        (status = SERVICE_UNAVAILABLE, description = "busy: too many searches are running, Retry-After says when to try again; timeout: the search took longer than 5 seconds, so narrow it.", body = ErrorBody),
        (status = "default", description = "An error.", body = ErrorBody),
    ),
    security(("session_cookie" = []), ("api_token" = []))
)]
pub(super) async fn search_workspace(
    State(state): State<AppState>,
    Authenticated(principal): Authenticated,
    Path(id): Path<Uuid>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResults>, ApiError> {
    let results = state.search.search(&principal, id, params.input()?).await?;
    Ok(Json(results.into()))
}
