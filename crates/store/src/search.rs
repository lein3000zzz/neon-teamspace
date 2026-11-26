//! Search: the pages of a workspace whose title, text, rows' property values or comments
//! match a query, among those someone may see, ranked, with their matches marked.
//!
//! Who may see what is the rules' to say, and the search service asks them before it runs
//! this: which teamspaces show their pages to the searcher, whether their own private pages
//! do, whether shares do, and the same for an agent's owner ([`Scope`]). The query keeps to
//! that in the database, so whoever sees little of a large workspace gets the few pages they
//! see, found through them rather than through every match: each way of seeing a page is a
//! branch of its own, which its index finds, and the pages under someone's shares are walked
//! from the shares down.
//!
//! The matches are full-text search with the `simple` configuration over
//! `pages.search_vector`, the title weighted most, then a row's property values, then the
//! document's text, then the words of the page's comments, and a trigram match of titles
//! typed loosely, with
//! `pg_trgm.word_similarity_threshold` at [`FUZZY_THRESHOLD`]. Pages in the trash are left
//! out.
//!
//! What a search costs stays bounded however large the pages are: only the pages answered
//! get their matches marked, the one read past them to tell whether more follow does not;
//! a snippet is marked in [`SNIPPET_CHARS`] characters of the text, from a little before
//! the first place one of the query's words is; and [`prepare`] gives the statements a
//! time limit.

use std::time::Duration;

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

/// How alike a query's words and a title must be, by trigrams, for a loose match:
/// `quaterly` matches `Quarterly roadmap`.
pub const FUZZY_THRESHOLD: &str = "0.5";

/// The text that marks where a match starts in a snippet, and where it ends. Neither is
/// ever in what search reads: they are taken out of it first.
pub const MATCH_START: char = '\u{1}';
pub const MATCH_END: char = '\u{2}';

/// How many characters of a page's text its snippet is marked in: from
/// [`SNIPPET_LEAD`] characters before the first of the query's words, or from the start.
/// Marking costs more than in proportion to the text it reads.
pub const SNIPPET_CHARS: i32 = 5000;
pub const SNIPPET_LEAD: i32 = 1000;

/// What a statement that ran out of the time [`prepare`] gave it fails with:
/// `query_canceled`.
const CANCELED: &str = "57014";

/// Who sees what, as the rules said: for the searcher, and for an agent's owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub mine: Side,
    /// For an agent or integration: its owner's, which bounds what it sees.
    pub owner: Option<Side>,
}

/// What one person sees of a workspace's pages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Side {
    /// The teamspaces whose every page they see.
    pub teamspaces: Vec<Uuid>,
    /// Whose private pages they see: their own, when they have any.
    pub private: Option<Uuid>,
    /// Whose shares show them pages: theirs, when shares do.
    pub shares: Option<Uuid>,
}

/// What to look for, and where.
#[derive(Debug, Clone, Default)]
pub struct Search<'a> {
    pub workspace_id: Uuid,
    /// For `to_tsquery('simple', …)`; `None` lists what the filters let through, the most
    /// recently changed first.
    pub text_search: Option<&'a str>,
    /// The words to match titles loosely by.
    pub fuzzy: Option<&'a str>,
    /// What a loose match must hold too, for `to_tsquery('simple', …)`: none of the words
    /// left out.
    pub excluded: Option<&'a str>,
    /// The query's words: a title that is them, but for case, ranks first, and a snippet is
    /// marked around the first place one of them is.
    pub words: &'a [String],
    pub teamspace_id: Option<Uuid>,
    /// `page`, `database` or `row`.
    pub kind: Option<&'a str>,
    pub created_by: Option<Uuid>,
    pub last_edited_by: Option<Uuid>,
    /// Changed, its title, place, style or content, at or after this.
    pub edited_after: Option<OffsetDateTime>,
    /// How many pages to answer, at most, from `offset` on.
    pub limit: i64,
    pub offset: i64,
}

/// The pages found, the best first, and whether more follow them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub hits: Vec<Hit>,
    pub more: bool,
}

/// A page found.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: Uuid,
    pub rank: f64,
    /// The title with its matches between [`MATCH_START`] and [`MATCH_END`]; `None`
    /// without a query.
    pub title: Option<String>,
    /// A few words of its property values, text and comments around its matches, marked
    /// so; `None` without a query or any of them.
    pub snippet: Option<String>,
}

/// The query: one text for the code and the tests that look at its plan.
macro_rules! search_sql {
    ($then:ident) => {
        $then!(
            r#"WITH RECURSIVE
               mine_shared (id) AS (
                   SELECT s.page_id FROM page_shares s
                   WHERE s.workspace_id = $1 AND s.user_id = $6
                 UNION
                   SELECT c.id FROM mine_shared d JOIN pages c ON c.parent_id = d.id
               ),
               owner_shared (id) AS (
                   SELECT s.page_id FROM page_shares s
                   WHERE $7 AND s.workspace_id = $1 AND s.user_id = $10
                 UNION
                   SELECT c.id FROM owner_shared d JOIN pages c ON c.parent_id = d.id
               ),
               found (id) AS (
                   SELECT p.id FROM pages p
                   WHERE p.workspace_id = $1 AND p.teamspace_id = ANY($4)
                     AND p.trashed_at IS NULL
                     AND ($11::uuid IS NULL OR p.teamspace_id = $11)
                     AND ($12::text IS NULL OR p.kind = $12)
                     AND ($13::uuid IS NULL OR p.created_by = $13)
                     AND ($14::uuid IS NULL OR p.last_edited_by = $14)
                     AND ($15::timestamptz IS NULL
                          OR GREATEST(p.updated_at, p.content_updated_at) >= $15)
                     AND ($2::text IS NULL OR p.search_vector @@ to_tsquery('simple', $2)
                          OR ($3::text IS NOT NULL AND $3 <% p.title
                              AND ($18::text IS NULL
                                   OR p.search_vector @@ to_tsquery('simple', $18))))
                     AND (NOT $7 OR p.teamspace_id = ANY($8) OR p.owner_id = $9
                          OR p.id IN (SELECT id FROM owner_shared))
                 UNION
                   SELECT p.id FROM pages p
                   WHERE p.workspace_id = $1 AND p.owner_id = $5
                     AND p.trashed_at IS NULL
                     AND ($11::uuid IS NULL OR p.teamspace_id = $11)
                     AND ($12::text IS NULL OR p.kind = $12)
                     AND ($13::uuid IS NULL OR p.created_by = $13)
                     AND ($14::uuid IS NULL OR p.last_edited_by = $14)
                     AND ($15::timestamptz IS NULL
                          OR GREATEST(p.updated_at, p.content_updated_at) >= $15)
                     AND ($2::text IS NULL OR p.search_vector @@ to_tsquery('simple', $2)
                          OR ($3::text IS NOT NULL AND $3 <% p.title
                              AND ($18::text IS NULL
                                   OR p.search_vector @@ to_tsquery('simple', $18))))
                     AND (NOT $7 OR p.teamspace_id = ANY($8) OR p.owner_id = $9
                          OR p.id IN (SELECT id FROM owner_shared))
                 UNION
                   SELECT p.id FROM mine_shared m JOIN pages p ON p.id = m.id
                   WHERE p.workspace_id = $1 AND p.trashed_at IS NULL
                     AND ($11::uuid IS NULL OR p.teamspace_id = $11)
                     AND ($12::text IS NULL OR p.kind = $12)
                     AND ($13::uuid IS NULL OR p.created_by = $13)
                     AND ($14::uuid IS NULL OR p.last_edited_by = $14)
                     AND ($15::timestamptz IS NULL
                          OR GREATEST(p.updated_at, p.content_updated_at) >= $15)
                     AND ($2::text IS NULL OR p.search_vector @@ to_tsquery('simple', $2)
                          OR ($3::text IS NOT NULL AND $3 <% p.title
                              AND ($18::text IS NULL
                                   OR p.search_vector @@ to_tsquery('simple', $18))))
                     AND (NOT $7 OR p.teamspace_id = ANY($8) OR p.owner_id = $9
                          OR p.id IN (SELECT id FROM owner_shared))
               ),
               ranked AS (
                   SELECT p.id, p.title, p.property_text, p.search_text, p.comment_text,
                          (CASE WHEN $2::text IS NULL THEN 0
                                ELSE ts_rank(p.search_vector, to_tsquery('simple', $2)) END
                           + CASE WHEN $3::text IS NULL THEN 0
                                  ELSE word_similarity($3, p.title) END
                           + CASE WHEN $2::text IS NOT NULL
                                       AND lower(p.title) = lower(array_to_string($19::text[], ' '))
                                  THEN 1 ELSE 0 END
                          )::float8 AS rank,
                          GREATEST(p.updated_at, p.content_updated_at) AS edited_at
                   FROM found f JOIN pages p ON p.id = f.id
                   ORDER BY rank DESC, edited_at DESC, p.id
                   LIMIT $16::bigint + 1 OFFSET $17
               ),
               numbered AS (
                   SELECT r.*, row_number() OVER (ORDER BY r.rank DESC, r.edited_at DESC, r.id)
                               AS n
                   FROM ranked r
               )
               -- The page past the part tells that more follow, and is not marked. A
               -- snippet is marked in $20 characters of the values, text and comments, from
               -- $21 before the first place one of the query's words is, or from the start.
               -- ts_headline drops what reads as an HTML tag, such as the <id> of
               -- plants/<id>/moisture, so each < stands as U+0003 while it marks.
               SELECT r.id AS "id!", r.rank AS "rank!", r.n AS "n!",
                      CASE WHEN $2::text IS NOT NULL AND r.n <= $16 THEN
                          translate(ts_headline('simple',
                                                translate(r.title, E'<\x01\x02\x03', E'\x03'),
                                                to_tsquery('simple', $2),
                                                E'StartSel=\x01, StopSel=\x02, HighlightAll=true'),
                                    E'\x03', '<')
                      END AS title,
                      CASE WHEN t.body IS NOT NULL THEN
                          translate(ts_headline('simple',
                                                translate(substr(t.body,
                                                                 greatest(1, COALESCE(m.at, 1) - $21),
                                                                 $20),
                                                          E'<\x01\x02\x03', E'\x03'),
                                                to_tsquery('simple', $2),
                                                E'StartSel=\x01, StopSel=\x02, MaxWords=30, MinWords=12, ShortWord=2, MaxFragments=2, FragmentDelimiter=" … "'),
                                    E'\x03', '<')
                      END AS snippet
               FROM numbered r
               LEFT JOIN LATERAL (
                   SELECT b.body, lower(b.body) AS low
                   FROM (SELECT left(concat_ws(E'\n', r.property_text, r.search_text,
                                               r.comment_text), 100000) AS body) b
                   WHERE $2::text IS NOT NULL AND r.n <= $16
                     AND (r.property_text IS NOT NULL OR r.search_text IS NOT NULL
                          OR r.comment_text IS NOT NULL)
                   OFFSET 0
               ) t ON true
               LEFT JOIN LATERAL (
                   SELECT min(a.at) AS at
                   FROM (SELECT strpos(t.low, lower(word)) AS at
                         FROM unnest($19::text[]) AS word) a
                   WHERE a.at > 0
               ) m ON true
               ORDER BY r.n"#
        )
    };
}

/// Readies the connection's transaction for [`search`]: each search is planned for the
/// values it is given, rather than with a plan made for any, which cannot tell a guest
/// with three shares from an owner of every teamspace; the loose match of titles takes
/// [`FUZZY_THRESHOLD`]; and each statement after, until the transaction ends, fails once it
/// runs longer than `limit`, as [`timed_out`] tells.
pub async fn prepare(conn: &mut PgConnection, limit: Duration) -> sqlx::Result<()> {
    sqlx::query!(
        r#"SELECT set_config('plan_cache_mode', 'force_custom_plan', true) AS "plans!",
                  set_config('pg_trgm.word_similarity_threshold', $1, true) AS "threshold!",
                  set_config('statement_timeout', $2, true) AS "timeout!""#,
        FUZZY_THRESHOLD,
        format!("{}ms", limit.as_millis().max(1)),
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(())
}

/// Whether a statement failed for running longer than [`prepare`] let it.
pub fn timed_out(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == CANCELED)
}

/// The pages found, the best first, and whether more follow: see the module docs. Run it in
/// a transaction that [`prepare`] readied.
pub async fn search(
    conn: &mut PgConnection,
    scope: &Scope,
    search: &Search<'_>,
) -> sqlx::Result<Found> {
    let owner = scope.owner.clone().unwrap_or_default();
    macro_rules! run {
        ($sql:literal) => {
            sqlx::query!(
                $sql,
                search.workspace_id,
                search.text_search,
                search.fuzzy,
                &scope.mine.teamspaces,
                scope.mine.private,
                scope.mine.shares,
                scope.owner.is_some(),
                &owner.teamspaces,
                owner.private,
                owner.shares,
                search.teamspace_id,
                search.kind,
                search.created_by,
                search.last_edited_by,
                search.edited_after,
                search.limit,
                search.offset,
                search.excluded,
                search.words,
                SNIPPET_CHARS,
                SNIPPET_LEAD,
            )
        };
    }
    let rows = search_sql!(run).fetch_all(&mut *conn).await?;
    let limit = usize::try_from(search.limit).unwrap_or_default();
    let more = rows.len() > limit;
    Ok(Found {
        hits: rows
            .into_iter()
            .filter(|row| row.n <= search.limit)
            .map(|row| Hit {
                id: row.id,
                rank: row.rank,
                title: row.title,
                snippet: row.snippet,
            })
            .collect(),
        more,
    })
}

/// The ids of the pages after `after`, in the order of their ids, whose documents may
/// hold text search lacks: those with updates no snapshot took in yet, as a server stopped
/// before it saved them leaves, and those with a snapshot whose text was never stored, or
/// with `all`, every page with a document; at most `limit`.
pub async fn unindexed(
    conn: &mut PgConnection,
    all: bool,
    after: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!(
        "SELECT p.id FROM pages p
         WHERE p.kind <> 'database'
           AND ($2::uuid IS NULL OR p.id > $2)
           AND (EXISTS (SELECT 1 FROM page_updates u WHERE u.page_id = p.id)
                OR (($1 OR p.search_text IS NULL)
                    AND EXISTS (SELECT 1 FROM page_snapshots s WHERE s.page_id = p.id)))
         ORDER BY p.id
         LIMIT $3",
        all,
        after,
        limit,
    )
    .fetch_all(&mut *conn)
    .await
}

/// Locks a page's row until the transaction ends, so a snapshot saved meanwhile waits for
/// the text read from the document as it was, and writes its own after, and reads the text
/// it has: `None` when the page is gone.
pub async fn lock_text(
    conn: &mut PgConnection,
    page_id: Uuid,
) -> sqlx::Result<Option<Option<String>>> {
    sqlx::query_scalar!(
        "SELECT search_text FROM pages WHERE id = $1 FOR UPDATE",
        page_id
    )
    .fetch_optional(&mut *conn)
    .await
}

/// Stores a page's text, as its snapshot would.
pub async fn set_text(conn: &mut PgConnection, page_id: Uuid, text: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE pages SET search_text = $2 WHERE id = $1",
        page_id,
        text
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
