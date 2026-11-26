//! The query on a large workspace: someone who sees a few of its twenty thousand matching
//! pages gets them, and its plan reads the pages they see, not every match.

use serde_json::Value as Json;
use sqlx::PgPool;

use super::*;

macro_rules! text_of {
    ($sql:literal) => {
        $sql
    };
}

const SEARCH_SQL: &str = search_sql!(text_of);

/// Fixed ids, made from a number.
fn id(n: u64) -> Uuid {
    Uuid::from_u64_pair(0x0000_0000_0000_7000, 0x8000_0000_0000_0000 | n)
}

const WORKSPACE: u64 = 1;
const ADA: u64 = 2;
const GUS: u64 = 3;
const SCOUT: u64 = 4;
const ARCHIVE: u64 = 5;
const SMALL: u64 = 6;

/// How many pages of the archive match.
const LARGE: i64 = 20_000;

/// Ada owns the workspace and its private Archive of twenty thousand reports, and a closed
/// teamspace of ten, which her agent Scout is in. Gus, a guest, has three reports shared
/// with him, each with two pages under it.
async fn seed(pool: &PgPool) {
    let statements = [
        format!(
            "INSERT INTO users (id, kind, email, email_normalized, display_name, password_hash,
                                role, created_at, updated_at)
             VALUES ('{ada}', 'human', 'ada@example.com', 'ada@example.com', 'Ada', 'hash',
                     'user', now(), now()),
                    ('{gus}', 'human', 'gus@example.com', 'gus@example.com', 'Gus', 'hash',
                     'user', now(), now())",
            ada = id(ADA),
            gus = id(GUS),
        ),
        format!(
            "INSERT INTO workspaces (id, name, slug, created_by, created_at, updated_at)
             VALUES ('{ws}', 'Big', 'big', '{ada}', now(), now())",
            ws = id(WORKSPACE),
            ada = id(ADA),
        ),
        format!(
            "INSERT INTO users (id, kind, display_name, role, workspace_id, owner_id, created_at,
                                updated_at)
             VALUES ('{scout}', 'agent', 'Scout', 'user', '{ws}', '{ada}', now(), now())",
            scout = id(SCOUT),
            ws = id(WORKSPACE),
            ada = id(ADA),
        ),
        format!(
            "INSERT INTO workspace_members (workspace_id, user_id, role, joined_at, kind)
             VALUES ('{ws}', '{ada}', 'owner', now(), 'human'),
                    ('{ws}', '{gus}', 'guest', now(), 'human'),
                    ('{ws}', '{scout}', 'member', now(), 'agent')",
            ws = id(WORKSPACE),
            ada = id(ADA),
            gus = id(GUS),
            scout = id(SCOUT),
        ),
        format!(
            "INSERT INTO teamspaces (id, workspace_id, name, access, created_at, updated_at)
             VALUES ('{archive}', '{ws}', 'Archive', 'private', now(), now()),
                    ('{small}', '{ws}', 'Small', 'closed', now(), now())",
            archive = id(ARCHIVE),
            small = id(SMALL),
            ws = id(WORKSPACE),
        ),
        format!(
            "INSERT INTO teamspace_members (teamspace_id, workspace_id, user_id, role, joined_at)
             VALUES ('{archive}', '{ws}', '{ada}', 'owner', now()),
                    ('{small}', '{ws}', '{ada}', 'owner', now()),
                    ('{small}', '{ws}', '{scout}', 'member', now())",
            archive = id(ARCHIVE),
            small = id(SMALL),
            ws = id(WORKSPACE),
            ada = id(ADA),
            scout = id(SCOUT),
        ),
        // Pages 1000 and on: the archive's reports, then the small teamspace's, then the
        // pages under the three shared with Gus.
        format!(
            "INSERT INTO pages (id, workspace_id, kind, teamspace_id, parent_id, position, title,
                                font, small_text, full_width, search_text, created_by,
                                created_at, updated_at, content_updated_at)
             SELECT ('00000000-0000-7000-8000-' || lpad(to_hex(1000 + n), 12, '0'))::uuid,
                    '{ws}', 'page', '{archive}', NULL, 'a' || n, 'Report ' || n, 'default',
                    false, false, 'The quarterly report, number ' || n || ', for the board',
                    '{ada}', now(), now(), now()
             FROM generate_series(1, {LARGE}) AS n",
            ws = id(WORKSPACE),
            archive = id(ARCHIVE),
            ada = id(ADA),
        ),
        format!(
            "INSERT INTO pages (id, workspace_id, kind, teamspace_id, parent_id, position, title,
                                font, small_text, full_width, search_text, created_by,
                                created_at, updated_at, content_updated_at)
             SELECT ('00000000-0000-7000-8000-' || lpad(to_hex(900 + n), 12, '0'))::uuid,
                    '{ws}', 'page', '{small}', NULL, 'a' || n, 'Small ' || n, 'default',
                    false, false, 'A quarterly note', '{ada}', now(), now(), now()
             FROM generate_series(1, 10) AS n",
            ws = id(WORKSPACE),
            small = id(SMALL),
            ada = id(ADA),
        ),
        format!(
            "INSERT INTO pages (id, workspace_id, kind, teamspace_id, parent_id, position, title,
                                font, small_text, full_width, search_text, created_by,
                                created_at, updated_at, content_updated_at)
             SELECT ('00000000-0000-7000-8000-' || lpad(to_hex(100 + n), 12, '0'))::uuid,
                    '{ws}', 'page', '{archive}',
                    ('00000000-0000-7000-8000-' || lpad(to_hex(1000 + (n + 1) / 2), 12, '0'))::uuid,
                    'b' || n, 'Under ' || n, 'default', false, false,
                    'Quarterly details', '{ada}', now(), now(), now()
             FROM generate_series(1, 6) AS n",
            ws = id(WORKSPACE),
            archive = id(ARCHIVE),
            ada = id(ADA),
        ),
        format!(
            "INSERT INTO page_shares (page_id, workspace_id, user_id, access, created_by,
                                      created_at, updated_at)
             SELECT ('00000000-0000-7000-8000-' || lpad(to_hex(1000 + n), 12, '0'))::uuid,
                    '{ws}', '{gus}', 'view', '{ada}', now(), now()
             FROM generate_series(1, 3) AS n",
            ws = id(WORKSPACE),
            gus = id(GUS),
            ada = id(ADA),
        ),
        "ANALYZE pages, page_shares, teamspaces, teamspace_members".to_owned(),
    ];
    for statement in statements {
        sqlx::query(sqlx::AssertSqlSafe(statement.as_str()))
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
}

/// The words of the searches below.
static QUARTERLY: std::sync::LazyLock<Vec<String>> =
    std::sync::LazyLock::new(|| vec!["quarterly".to_owned()]);
static REPORT_17: std::sync::LazyLock<Vec<String>> =
    std::sync::LazyLock::new(|| vec!["Report".to_owned(), "17".to_owned()]);

/// Long enough for any search here; reaching it means the search reads far too much.
const LIMIT: Duration = Duration::from_secs(5);

fn quarterly() -> Search<'static> {
    Search {
        workspace_id: id(WORKSPACE),
        text_search: Some("'quarterly':*"),
        fuzzy: Some("quarterly"),
        words: &QUARTERLY,
        limit: 50,
        ..Search::default()
    }
}

fn guest() -> Scope {
    Scope {
        mine: Side {
            shares: Some(id(GUS)),
            ..Side::default()
        },
        owner: None,
    }
}

fn scout() -> Scope {
    Scope {
        mine: Side {
            teamspaces: vec![id(SMALL)],
            private: None,
            shares: Some(id(SCOUT)),
        },
        owner: Some(Side {
            teamspaces: vec![id(ARCHIVE), id(SMALL)],
            private: Some(id(ADA)),
            shares: Some(id(ADA)),
        }),
    }
}

/// The plan of the search, run, as Postgres explains it.
async fn plan(pool: &PgPool, scope: &Scope, search: &Search<'_>) -> Json {
    let mut tx = pool.begin().await.expect("a transaction");
    prepare(&mut tx, LIMIT).await.expect("prepared");
    let owner = scope.owner.clone().unwrap_or_default();
    let explain = format!("EXPLAIN (ANALYZE, FORMAT JSON) {SEARCH_SQL}");
    let explained: Json = sqlx::query_scalar(sqlx::AssertSqlSafe(explain))
        .bind(search.workspace_id)
        .bind(search.text_search)
        .bind(search.fuzzy)
        .bind(&scope.mine.teamspaces)
        .bind(scope.mine.private)
        .bind(scope.mine.shares)
        .bind(scope.owner.is_some())
        .bind(&owner.teamspaces)
        .bind(owner.private)
        .bind(owner.shares)
        .bind(search.teamspace_id)
        .bind(search.kind)
        .bind(search.created_by)
        .bind(search.last_edited_by)
        .bind(search.edited_after)
        .bind(search.limit)
        .bind(search.offset)
        .bind(search.excluded)
        .bind(search.words)
        .bind(SNIPPET_CHARS)
        .bind(SNIPPET_LEAD)
        .fetch_one(&mut *tx)
        .await
        .expect("explained");
    tx.rollback().await.expect("rolled back");
    explained
}

/// Every node of a plan, and the rows each read of the pages, over all its loops.
fn pages_read(node: &Json, read: &mut Vec<(String, f64)>) {
    if node.get("Relation Name").and_then(Json::as_str) == Some("pages") {
        let rows = node["Actual Rows"].as_f64().unwrap_or(0.0)
            * node["Actual Loops"].as_f64().unwrap_or(1.0)
            + node
                .get("Rows Removed by Filter")
                .and_then(Json::as_f64)
                .unwrap_or(0.0)
            + node
                .get("Rows Removed by Index Recheck")
                .and_then(Json::as_f64)
                .unwrap_or(0.0);
        let kind = node["Node Type"].as_str().unwrap_or("?").to_owned();
        read.push((kind, rows));
    }
    for child in node
        .get("Plans")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        pages_read(child, read);
    }
}

async fn found(pool: &PgPool, scope: &Scope, search: &Search<'_>) -> Vec<Uuid> {
    let mut tx = pool.begin().await.expect("a transaction");
    prepare(&mut tx, LIMIT).await.expect("prepared");
    let found = super::search(&mut tx, scope, search)
        .await
        .expect("searched");
    tx.rollback().await.expect("rolled back");
    let mut ids: Vec<Uuid> = found.hits.into_iter().map(|hit| hit.id).collect();
    ids.sort_unstable();
    ids
}

fn ids(range: impl IntoIterator<Item = u64>) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = range.into_iter().map(id).collect();
    ids.sort_unstable();
    ids
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn who_sees_little_of_a_large_workspace_gets_it_without_reading_the_rest(pool: PgPool) {
    seed(&pool).await;

    // Gus gets his three reports and the six pages under them, and no other of the twenty
    // thousand that match.
    assert_eq!(
        found(&pool, &guest(), &quarterly()).await,
        ids((1001..=1003).chain(101..=106))
    );
    // Scout gets the small teamspace's ten, which it and Ada both see.
    assert_eq!(found(&pool, &scout(), &quarterly()).await, ids(901..=910));

    for (who, scope) in [("Gus", guest()), ("Scout", scout())] {
        let explained = plan(&pool, &scope, &quarterly()).await;
        let mut read = Vec::new();
        pages_read(&explained[0]["Plan"], &mut read);
        let total: f64 = read.iter().map(|(_, rows)| rows).sum();
        assert!(
            total < 500.0,
            "{who}'s search read {total} rows of pages: {read:?}"
        );
        assert!(
            read.iter().all(|(kind, _)| kind != "Seq Scan"),
            "{who}'s search scans the pages: {read:?}"
        );
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn who_sees_everything_finds_the_best_first_a_part_at_a_time(pool: PgPool) {
    seed(&pool).await;
    let ada = Scope {
        mine: Side {
            teamspaces: vec![id(ARCHIVE), id(SMALL)],
            private: Some(id(ADA)),
            shares: Some(id(ADA)),
        },
        owner: None,
    };
    let search = Search {
        text_search: Some("'report':* & '17':*"),
        fuzzy: Some("Report 17"),
        words: &REPORT_17,
        ..quarterly()
    };
    let mut tx = pool.begin().await.expect("a transaction");
    prepare(&mut tx, LIMIT).await.expect("prepared");
    let found = super::search(&mut tx, &ada, &search)
        .await
        .expect("searched");
    assert!(found.more, "more than fifty match");
    let hits = found.hits;
    assert_eq!(hits.len(), 50);
    // Report 17 itself first: its title is the query.
    assert_eq!(hits[0].id, id(1017));
    assert_eq!(
        hits[0].title.as_deref(),
        Some("\u{1}Report\u{2} \u{1}17\u{2}")
    );
    assert!(hits.windows(2).all(|pair| pair[0].rank >= pair[1].rank));
    let next = super::search(
        &mut tx,
        &ada,
        &Search {
            offset: 50,
            ..search.clone()
        },
    )
    .await
    .expect("searched")
    .hits;
    assert!(
        next.iter()
            .all(|hit| hits.iter().all(|seen| seen.id != hit.id))
    );
    tx.rollback().await.expect("rolled back");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_last_part_says_no_more_follow(pool: PgPool) {
    seed(&pool).await;
    let mut tx = pool.begin().await.expect("a transaction");
    prepare(&mut tx, LIMIT).await.expect("prepared");
    for (limit, offset, hits, more) in [(9, 0, 9, false), (8, 0, 8, true), (5, 5, 4, false)] {
        let found = super::search(
            &mut tx,
            &guest(),
            &Search {
                limit,
                offset,
                ..quarterly()
            },
        )
        .await
        .expect("searched");
        assert_eq!(
            (found.hits.len(), found.more),
            (hits, more),
            "{limit} from {offset}"
        );
        assert!(
            found.hits.iter().all(|hit| hit.title.is_some()),
            "all marked"
        );
    }
    tx.rollback().await.expect("rolled back");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_statement_running_past_the_limit_fails_as_timed_out(pool: PgPool) {
    let mut tx = pool.begin().await.expect("a transaction");
    prepare(&mut tx, Duration::from_millis(20))
        .await
        .expect("prepared");
    let error = sqlx::query("SELECT pg_sleep(2)")
        .execute(&mut *tx)
        .await
        .expect_err("cut off");
    assert!(timed_out(&error), "{error}");
    drop(tx);

    let mut tx = pool.begin().await.expect("a transaction");
    prepare(&mut tx, LIMIT).await.expect("prepared");
    sqlx::query("SELECT pg_sleep(0.05)")
        .execute(&mut *tx)
        .await
        .expect("within the limit");
    let other = sqlx::query("SELECT 1 / 0")
        .execute(&mut *tx)
        .await
        .expect_err("fails");
    assert!(!timed_out(&other), "{other}");
}
