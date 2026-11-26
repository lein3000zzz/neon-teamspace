//! Search: what is indexed and when, in the transaction of the write it comes from; each
//! filter; access, against the rules for every page and every kind of caller; ranking,
//! snippets, unicode, short queries and loose titles; parts of the results; refusals; and
//! rebuilding the text.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use neon_app::databases::{NewDatabase, NewProperty, NewRow, PropertyChange, PropertySpec};
use neon_app::pages::{ImportedContent, Location, NewPage, PageUpdate};
use neon_app::search::{SearchInput, SearchResults, SearchService};
use neon_app::workspaces::NewTeamspace;
use neon_blocks::markdown::from_markdown;
use neon_blocks::{Block, SequentialIds};
use neon_domain::auth::Principal;
use neon_domain::databases::PropertyKind;
use neon_domain::pages::{PageAccess, PageKind, Placement};
use neon_domain::workspaces::{TeamspaceAccess, TeamspaceRole, WorkspaceRole};
use neon_store::documents::{self, NewSnapshot, NewUpdate};
use serde_json::{Map, Value as Json, json};
use sqlx::PgPool;
use support::Harness;
use uuid::Uuid;
use yrs::updates::encoder::Encode as _;
use yrs::{Doc, ReadTxn as _, StateVector, Transact as _};

/// Ada owns Design, with Bob as an admin, Cy and Dee as members and Gus as a guest; Eve is
/// in no workspace. Cy made Open and Secret, which is private and has Dee in it; Cy's agent
/// Scout is in Secret too, and Ada's agent Ace only in General.
struct Team {
    h: Harness,
    search: SearchService,
    ws: Uuid,
    general: Uuid,
    open: Uuid,
    secret: Uuid,
    ada: Principal,
    bob: Principal,
    cy: Principal,
    dee: Principal,
    gus: Principal,
    eve: Principal,
    scout: Principal,
    ace: Principal,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bob = h.person("bob@example.com").await;
    let cy = h.person("cy@example.com").await;
    let dee = h.person("dee@example.com").await;
    let gus = h.person("gus@example.com").await;
    let eve = h.person("eve@example.com").await;
    let ws = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, ws, &bob, WorkspaceRole::Admin).await;
    h.bring_in(&ada, ws, &cy, WorkspaceRole::Member).await;
    h.bring_in(&ada, ws, &dee, WorkspaceRole::Member).await;
    h.bring_in(&ada, ws, &gus, WorkspaceRole::Guest).await;
    let general = h
        .workspaces
        .list_teamspaces(&ada, ws)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let mut made = Vec::new();
    for (name, access) in [
        ("Open", TeamspaceAccess::Open),
        ("Secret", TeamspaceAccess::Private),
    ] {
        let entry = h
            .workspaces
            .create_teamspace(
                &cy,
                ws,
                NewTeamspace {
                    name: name.to_owned(),
                    icon: None,
                    hue: None,
                    access,
                },
            )
            .await
            .expect("created");
        made.push(entry.teamspace.id);
    }
    let (open, secret) = (made[0], made[1]);
    let scout = h.agent(&cy, ws, "Scout").await;
    let ace = h.agent(&ada, ws, "Ace").await;
    for user in [dee.user.id, scout.id] {
        h.workspaces
            .add_teamspace_member(&cy, ws, secret, user, TeamspaceRole::Member)
            .await
            .expect("added");
    }
    let (_, scout) = h.agent_token(&cy, &scout, &["read"]).await;
    let (_, ace) = h.agent_token(&ada, &ace, &["read"]).await;
    let search = SearchService::builder(h.pool.clone())
        .clock(h.time.clock())
        .build();
    Team {
        h,
        search,
        ws,
        general,
        open,
        secret,
        ada,
        bob,
        cy,
        dee,
        gus,
        eve,
        scout,
        ace,
    }
}

impl Team {
    /// A page `by` makes at `at`, with the blocks of `markdown`.
    async fn page(&self, by: &Principal, at: Location, title: &str, markdown: &str) -> Uuid {
        self.h
            .pages
            .create_page(
                by,
                self.ws,
                at,
                Placement::Last,
                NewPage {
                    title: title.to_owned(),
                    content: ImportedContent::of(&from_markdown(markdown)),
                    ..NewPage::default()
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{title}: {error}"))
            .page
            .id
    }

    async fn general_page(&self, title: &str, markdown: &str) -> Uuid {
        self.page(
            &self.ada,
            Location::Teamspace(self.general),
            title,
            markdown,
        )
        .await
    }

    async fn find(&self, who: &Principal, query: &str) -> SearchResults {
        self.find_with(
            who,
            SearchInput {
                query: query.to_owned(),
                ..SearchInput::default()
            },
        )
        .await
    }

    async fn find_with(&self, who: &Principal, input: SearchInput) -> SearchResults {
        self.search
            .search(who, self.ws, input)
            .await
            .expect("searched")
    }

    /// The titles found, best first.
    async fn titles(&self, who: &Principal, query: &str) -> Vec<String> {
        titles(&self.find(who, query).await)
    }

    async fn titles_with(&self, who: &Principal, input: SearchInput) -> Vec<String> {
        titles(&self.find_with(who, input).await)
    }

    /// Every page `who` finds with an empty query, through every part.
    async fn everything(&self, who: &Principal) -> BTreeSet<Uuid> {
        let mut seen = BTreeSet::new();
        let mut after = None;
        loop {
            let part = self
                .find_with(
                    who,
                    SearchInput {
                        after: after.clone(),
                        limit: Some(5),
                        ..SearchInput::default()
                    },
                )
                .await;
            for hit in &part.hits {
                assert!(seen.insert(hit.page.page.id), "found twice");
            }
            match part.next {
                Some(next) => after = Some(next),
                None => return seen,
            }
        }
    }

    async fn database(&self, title: &str) -> Uuid {
        self.h
            .databases
            .create_database(
                &self.ada,
                self.ws,
                Location::Teamspace(self.general),
                Placement::Last,
                NewDatabase {
                    title: title.to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("created")
            .page
            .page
            .id
    }

    async fn property(&self, database: Uuid, name: &str, kind: PropertyKind) -> Uuid {
        self.h
            .databases
            .add_property(
                &self.ada,
                database,
                NewProperty {
                    name: name.to_owned(),
                    spec: PropertySpec {
                        kind,
                        number_format: None,
                        currency: None,
                        prefix: None,
                    },
                    options: None,
                    placement: Placement::Last,
                },
            )
            .await
            .expect("added")
            .id
    }

    async fn row(&self, database: Uuid, values: Json) -> Uuid {
        self.h
            .databases
            .create_row(
                &self.ada,
                database,
                NewRow {
                    values: map(values),
                    ..NewRow::default()
                },
            )
            .await
            .expect("created")
            .id
    }
}

fn map(values: Json) -> Map<String, Json> {
    serde_json::from_value(values).expect("an object")
}

fn titles(results: &SearchResults) -> Vec<String> {
    results
        .hits
        .iter()
        .map(|hit| hit.page.page.title.clone())
        .collect()
}

fn refused(result: Result<SearchResults, neon_app::search::SearchError>) -> &'static str {
    result.expect_err("refused").code()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn titles_text_and_rows_values_are_found_as_they_are_written(pool: PgPool) {
    let t = team(pool).await;
    let page = t
        .general_page("Quarterly roadmap", "We ship the *harbour* bridge\n")
        .await;
    assert_eq!(t.titles(&t.dee, "harbour").await, ["Quarterly roadmap"]);
    assert_eq!(t.titles(&t.dee, "roadmap").await, ["Quarterly roadmap"]);

    // A new title is found at once, and the old one no more.
    t.h.pages
        .update_page(
            &t.ada,
            page,
            PageUpdate {
                title: Some("Annual plan".to_owned()),
                ..PageUpdate::default()
            },
        )
        .await
        .expect("renamed");
    assert_eq!(t.titles(&t.dee, "annual").await, ["Annual plan"]);
    assert!(t.titles(&t.dee, "roadmap").await.is_empty());

    // A row by its title and its text and URL values, not by its other values.
    let tasks = t.database("Tasks").await;
    let notes = t.property(tasks, "Notes", PropertyKind::Text).await;
    t.property(tasks, "Link", PropertyKind::Url).await;
    let estimate = t.property(tasks, "Estimate", PropertyKind::Number).await;
    let row = t
        .row(
            tasks,
            json!({"Name": "Fix login", "Notes": "flaky on mobile", "Link": "https://example.com/zebra", "Estimate": 31337}),
        )
        .await;
    assert_eq!(t.titles(&t.dee, "flaky").await, ["Fix login"]);
    assert_eq!(t.titles(&t.dee, "zebra").await, ["Fix login"]);
    assert!(t.titles(&t.dee, "31337").await.is_empty());
    assert_eq!(t.titles(&t.dee, "tasks").await, ["Tasks"]);

    // Changed, its old value is not found and its new one is.
    t.h.databases
        .update_row(&t.ada, row, map(json!({"Notes": "steady now"})))
        .await
        .expect("updated");
    assert!(t.titles(&t.dee, "flaky").await.is_empty());
    assert_eq!(t.titles(&t.dee, "steady").await, ["Fix login"]);
    // A property that stops being text stops being read, and one deleted too.
    t.h.databases
        .update_property(
            &t.ada,
            tasks,
            estimate,
            PropertyChange {
                spec: Some(PropertySpec {
                    kind: PropertyKind::Text,
                    number_format: None,
                    currency: None,
                    prefix: None,
                }),
                confirm: true,
                ..PropertyChange::default()
            },
        )
        .await
        .expect("changed");
    assert_eq!(t.titles(&t.dee, "31337").await, ["Fix login"]);
    t.h.databases
        .delete_property(&t.ada, tasks, notes)
        .await
        .expect("deleted");
    assert!(t.titles(&t.dee, "steady").await.is_empty());
    assert_eq!(t.titles(&t.dee, "zebra").await, ["Fix login"]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_document_s_text_is_indexed_in_the_transaction_that_stores_its_snapshot(pool: PgPool) {
    let t = team(pool).await;
    let page = t.general_page("Plan", "").await;
    let doc = Doc::new();
    let fragment = neon_blocks::fragment(&doc);
    neon_blocks::replace(
        &mut doc.transact_mut(),
        &fragment,
        &from_markdown("The lighthouse keeper\n"),
        &mut SequentialIds::new("b"),
    );
    let (state, vector) = {
        let txn = doc.transact();
        (
            txn.encode_state_as_update_v1(&StateVector::default()),
            txn.state_vector().encode_v1(),
        )
    };
    let snapshot = NewSnapshot {
        state: &state,
        state_vector: &vector,
        last_seq: 0,
        at: t.h.time.clock().now(),
    };

    // Inside the transaction that saves the snapshot, the page is found by its text; once
    // it rolls back, it is not.
    let mut tx = t.h.pool.begin().await.expect("a transaction");
    documents::save_snapshot(&mut *tx, page, snapshot)
        .await
        .expect("saved");
    documents::set_content(&mut *tx, page, Some("The lighthouse keeper"), None)
        .await
        .expect("stored");
    let inside: bool = sqlx::query_scalar(
        "SELECT search_vector @@ to_tsquery('simple', 'lighthouse') FROM pages WHERE id = $1",
    )
    .bind(page)
    .fetch_one(&mut *tx)
    .await
    .expect("read");
    assert!(inside);
    tx.rollback().await.expect("rolled back");
    assert!(t.titles(&t.dee, "lighthouse").await.is_empty());

    // Committed, it is found at once, and nothing waits in the job queue for it.
    let mut tx = t.h.pool.begin().await.expect("a transaction");
    documents::save_snapshot(&mut *tx, page, snapshot)
        .await
        .expect("saved");
    documents::set_content(&mut *tx, page, Some("The lighthouse keeper"), None)
        .await
        .expect("stored");
    tx.commit().await.expect("committed");
    assert_eq!(t.titles(&t.dee, "lighthouse").await, ["Plan"]);
    let jobs: i64 = t.h.scalar("SELECT count(*) FROM jobs").await;
    assert_eq!(jobs, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_filter_narrows_what_is_found(pool: PgPool) {
    let t = team(pool).await;
    let in_general = t.general_page("Alpha notes", "ledger\n").await;
    t.page(&t.cy, Location::Teamspace(t.open), "Beta notes", "ledger\n")
        .await;
    let tasks = t.database("Ledger tasks").await;
    t.row(tasks, json!({"Name": "Ledger row"})).await;
    let by = |query: &str| SearchInput {
        query: query.to_owned(),
        ..SearchInput::default()
    };
    let sorted = |mut titles: Vec<String>| {
        titles.sort();
        titles
    };

    assert_eq!(
        sorted(t.titles(&t.dee, "ledger").await),
        ["Alpha notes", "Beta notes", "Ledger row", "Ledger tasks"]
    );
    let only = |input: SearchInput| async { sorted(t.titles_with(&t.dee, input).await) };
    assert_eq!(
        only(SearchInput {
            teamspace_id: Some(t.open),
            ..by("ledger")
        })
        .await,
        ["Beta notes"]
    );
    for (kind, title) in [
        (PageKind::Page, vec!["Alpha notes", "Beta notes"]),
        (PageKind::Database, vec!["Ledger tasks"]),
        (PageKind::Row, vec!["Ledger row"]),
    ] {
        assert_eq!(
            only(SearchInput {
                kind: Some(kind),
                ..by("ledger")
            })
            .await,
            title
        );
    }
    assert_eq!(
        only(SearchInput {
            created_by: Some(t.cy.user.id),
            ..by("ledger")
        })
        .await,
        ["Beta notes"]
    );
    // Dee changes Alpha last.
    t.h.time.advance(Duration::from_secs(10 * 24 * 3600));
    t.h.pages
        .update_page(
            &t.dee,
            in_general,
            PageUpdate {
                title: Some("Alpha notes, edited".to_owned()),
                ..PageUpdate::default()
            },
        )
        .await
        .expect("renamed");
    assert_eq!(
        only(SearchInput {
            edited_by: Some(t.dee.user.id),
            ..by("ledger")
        })
        .await,
        ["Alpha notes, edited"]
    );
    assert_eq!(
        only(SearchInput {
            edited_within: Some(Duration::from_secs(24 * 3600)),
            ..by("ledger")
        })
        .await,
        ["Alpha notes, edited"]
    );
    assert_eq!(
        only(SearchInput {
            edited_within: Some(Duration::from_secs(30 * 24 * 3600)),
            ..by("ledger")
        })
        .await
        .len(),
        4
    );
    // The filters alone, without words, list what they let through, latest first.
    assert_eq!(
        t.titles_with(
            &t.dee,
            SearchInput {
                teamspace_id: Some(t.general),
                ..SearchInput::default()
            }
        )
        .await,
        ["Alpha notes, edited", "Ledger tasks", "Ledger row"]
    );
    // A teamspace the caller may not see finds nothing, as one that does not exist.
    t.page(&t.cy, Location::Teamspace(t.secret), "Secret ledger", "")
        .await;
    let secret = SearchInput {
        teamspace_id: Some(t.secret),
        ..by("ledger")
    };
    assert_eq!(
        t.titles_with(&t.dee, secret.clone()).await,
        ["Secret ledger"]
    );
    assert!(t.titles_with(&t.bob, secret).await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn each_caller_finds_exactly_the_pages_the_rules_let_them_see(pool: PgPool) {
    let t = team(pool).await;
    let general_page = t.general_page("General page", "").await;
    let under = t
        .page(&t.ada, Location::Under(general_page), "Under general", "")
        .await;
    let open_page = t
        .page(&t.cy, Location::Teamspace(t.open), "Open page", "")
        .await;
    let secret_page = t
        .page(&t.cy, Location::Teamspace(t.secret), "Secret page", "")
        .await;
    let secret_under = t
        .page(&t.dee, Location::Under(secret_page), "Under secret", "")
        .await;
    let adas = t.page(&t.ada, Location::Private, "Ada's own", "").await;
    let cys = t.page(&t.cy, Location::Private, "Cy's own", "").await;
    t.page(&t.cy, Location::Under(cys), "Under Cy's own", "")
        .await;
    let tasks = t.database("Tasks").await;
    t.row(tasks, json!({"Name": "A row"})).await;
    let trashed = t.general_page("In the trash", "").await;
    t.h.pages
        .trash_page(&t.ada, trashed)
        .await
        .expect("trashed");

    // Shares: Gus gets the secret page and Ada's own; Dee Ada's own; Ace, Ada's agent, the
    // page under the secret one, which Ada herself does not see; Scout Cy's own.
    for (by, page, who) in [
        (&t.cy, secret_page, &t.gus),
        (&t.ada, adas, &t.gus),
        (&t.ada, adas, &t.dee),
        (&t.dee, secret_under, &t.ace),
        (&t.cy, cys, &t.scout),
        (&t.ada, under, &t.gus),
    ] {
        t.h.pages
            .share_page(by, page, who.user.id, PageAccess::View)
            .await
            .expect("shared");
    }
    assert!(
        t.everything(&t.gus)
            .await
            .iter()
            .all(|page| *page != open_page)
    );

    let all: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM pages WHERE workspace_id = $1")
        .bind(t.ws)
        .fetch_all(&t.h.pool)
        .await
        .expect("listed");
    for (name, who) in [
        ("Ada", &t.ada),
        ("Bob", &t.bob),
        ("Cy", &t.cy),
        ("Dee", &t.dee),
        ("Gus", &t.gus),
        ("Scout", &t.scout),
        ("Ace", &t.ace),
    ] {
        let mut allowed = BTreeSet::new();
        for page in &all {
            if let Ok(view) = t.h.pages.page(who, *page).await
                && !view.page.is_trashed()
            {
                allowed.insert(*page);
            }
        }
        assert_eq!(t.everything(who).await, allowed, "{name}");
        assert!(!allowed.contains(&trashed));
    }
    // Spot checks of what the rules give.
    let gus = t.everything(&t.gus).await;
    assert_eq!(
        gus,
        BTreeSet::from([secret_page, secret_under, adas, under])
    );
    let ace = t.everything(&t.ace).await;
    assert!(!ace.contains(&secret_under), "Ada sees no page of Secret");
    assert!(ace.contains(&general_page));
    let scout = t.everything(&t.scout).await;
    assert!(
        scout.contains(&secret_page) && scout.contains(&cys),
        "Cy shares her own page with her agent, and both see it"
    );
    let bob = t.everything(&t.bob).await;
    assert!(
        !bob.contains(&secret_page),
        "admins outside a private teamspace see none of it"
    );

    // Whoever is not in the workspace finds nothing of it.
    let outside = t.search.search(&t.eve, t.ws, SearchInput::default()).await;
    assert_eq!(refused(outside), "not_found");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn guests_get_no_people_and_may_not_filter_by_them(pool: PgPool) {
    let t = team(pool).await;
    let page = t.general_page("Shared plan", "").await;
    t.h.pages
        .share_page(&t.ada, page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let found = t.find(&t.gus, "shared").await;
    assert_eq!(titles(&found), ["Shared plan"]);
    assert!(found.hits[0].page.members_hidden);
    for input in [
        SearchInput {
            created_by: Some(t.ada.user.id),
            ..SearchInput::default()
        },
        SearchInput {
            edited_by: Some(t.ada.user.id),
            ..SearchInput::default()
        },
    ] {
        assert_eq!(
            refused(t.search.search(&t.gus, t.ws, input).await),
            "forbidden"
        );
    }
    // The page above one shared alone is left out of it; one shared with its parent is not.
    let parent = t.general_page("Parent", "").await;
    let child = t.page(&t.ada, Location::Under(parent), "Sprout", "").await;
    let under_shared = t
        .page(&t.ada, Location::Under(page), "Grandchild", "")
        .await;
    t.h.pages
        .share_page(&t.ada, child, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let found = t.find(&t.gus, "sprout").await;
    assert_eq!(titles(&found), ["Sprout"]);
    assert_eq!(found.hits[0].page.page.parent_id, None);
    let found = t.find(&t.gus, "grandchild").await;
    assert_eq!(found.hits[0].page.page.id, under_shared);
    assert_eq!(found.hits[0].page.page.parent_id, Some(page));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn titles_rank_first_and_matches_are_marked(pool: PgPool) {
    let t = team(pool).await;
    t.general_page("Notes", "the harbour plan, and the harbour budget\n")
        .await;
    t.general_page("Harbour", "").await;
    t.general_page("Harbour budget", "").await;
    t.general_page("Budget", "A harbour\n").await;

    // The title that is the query, then titles that hold it, then texts.
    let found = t.find(&t.dee, "harbour").await;
    assert_eq!(titles(&found)[0], "Harbour");
    assert_eq!(titles(&found)[1], "Harbour budget");
    assert!(
        found
            .hits
            .windows(2)
            .all(|pair| pair[0].rank >= pair[1].rank)
    );
    // More of the words ranks higher.
    assert_eq!(
        t.titles(&t.dee, "harbour budget").await[0],
        "Harbour budget"
    );

    let notes = found
        .hits
        .iter()
        .find(|hit| hit.page.page.title == "Notes")
        .expect("found by its text");
    let marked: Vec<(&str, bool)> = notes
        .snippet
        .iter()
        .map(|run| (run.text.as_str(), run.highlight))
        .collect();
    assert_eq!(
        marked,
        [
            ("the ", false),
            ("harbour", true),
            (" plan, and the ", false),
            ("harbour", true),
            (" budget", false)
        ]
    );
    assert_eq!(found.hits[0].title[0].text, "Harbour");
    assert!(found.hits[0].title[0].highlight);
    // Prefixes are marked whole.
    let found = t.find(&t.dee, "harb").await;
    assert_eq!(found.hits.len(), 4);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn words_in_any_script_short_ones_and_the_query_syntax(pool: PgPool) {
    let t = team(pool).await;
    t.general_page("Ünïcödé Straße", "東京タワー の 夜景\n")
        .await;
    t.general_page("Emoji 🚀 launch", "a b c\n").await;
    t.general_page("Draft plan", "an old idea\n").await;
    t.general_page("Final plan", "a new idea\n").await;

    assert_eq!(t.titles(&t.dee, "ÜNÏCÖDÉ").await, ["Ünïcödé Straße"]);
    assert_eq!(t.titles(&t.dee, "straße").await, ["Ünïcödé Straße"]);
    assert_eq!(t.titles(&t.dee, "東京").await, ["Ünïcödé Straße"]);
    assert_eq!(t.titles(&t.dee, "launch 🚀").await, ["Emoji 🚀 launch"]);
    // One character is a whole word; two find words they start.
    assert_eq!(t.titles(&t.dee, "b").await, ["Emoji 🚀 launch"]);
    assert!(t.titles(&t.dee, "l").await.is_empty());
    assert_eq!(t.titles(&t.dee, "la").await, ["Emoji 🚀 launch"]);

    let mut both = t.titles(&t.dee, "plan").await;
    both.sort();
    assert_eq!(both, ["Draft plan", "Final plan"]);
    assert_eq!(t.titles(&t.dee, "plan -draft").await, ["Final plan"]);
    assert_eq!(t.titles(&t.dee, "\"old idea\"").await, ["Draft plan"]);
    assert!(t.titles(&t.dee, "\"idea old\"").await.is_empty());
    let mut either = t.titles(&t.dee, "draft OR final").await;
    either.sort();
    assert_eq!(either, ["Draft plan", "Final plan"]);
    // What the query language would read as its own is plain text: `!` leaves nothing out.
    assert_eq!(t.titles(&t.dee, "plan & !draft |").await, ["Draft plan"]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn misspelled_titles_are_found_and_misspelled_text_is_not(pool: PgPool) {
    let t = team(pool).await;
    t.general_page("Quarterly roadmap", "Infrastructure migration\n")
        .await;
    t.general_page("Hiring", "").await;

    assert_eq!(t.titles(&t.dee, "quaterly").await, ["Quarterly roadmap"]);
    assert_eq!(t.titles(&t.dee, "roadmp").await, ["Quarterly roadmap"]);
    assert_eq!(
        t.titles(&t.dee, "quaterly roadmp").await,
        ["Quarterly roadmap"]
    );
    assert!(t.titles(&t.dee, "infrastrcture").await.is_empty());
    assert!(
        t.titles(&t.dee, "hirng plan").await.is_empty(),
        "too far from both"
    );
    // Loose matches rank below exact ones.
    t.general_page("Quaterly", "").await;
    assert_eq!(t.titles(&t.dee, "quaterly").await[0], "Quaterly");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn results_come_a_part_at_a_time_and_refusals_say_why(pool: PgPool) {
    let t = team(pool).await;
    for n in 0..7 {
        t.general_page(&format!("Report {n}"), "").await;
    }
    let first = t
        .find_with(
            &t.dee,
            SearchInput {
                query: "report".to_owned(),
                limit: Some(3),
                ..SearchInput::default()
            },
        )
        .await;
    assert_eq!(first.hits.len(), 3);
    assert_eq!(first.next.as_deref(), Some("3"));
    let mut seen: BTreeSet<String> = titles(&first).into_iter().collect();
    let mut after = first.next;
    while let Some(cursor) = after {
        let part = t
            .find_with(
                &t.dee,
                SearchInput {
                    query: "report".to_owned(),
                    limit: Some(3),
                    after: Some(cursor),
                    ..SearchInput::default()
                },
            )
            .await;
        for title in titles(&part) {
            assert!(seen.insert(title), "listed twice");
        }
        after = part.next;
    }
    assert_eq!(seen.len(), 7);

    let search = |input: SearchInput| t.search.search(&t.dee, t.ws, input);
    for (input, code) in [
        (
            SearchInput {
                query: "x".repeat(201),
                ..SearchInput::default()
            },
            "invalid_search",
        ),
        (
            SearchInput {
                query: "-draft".to_owned(),
                ..SearchInput::default()
            },
            "invalid_search",
        ),
        (
            SearchInput {
                limit: Some(51),
                ..SearchInput::default()
            },
            "invalid_input",
        ),
        (
            SearchInput {
                after: Some("nonsense".to_owned()),
                ..SearchInput::default()
            },
            "invalid_input",
        ),
        (
            SearchInput {
                edited_within: Some(Duration::from_secs(11 * 365 * 24 * 3600)),
                ..SearchInput::default()
            },
            "invalid_input",
        ),
    ] {
        assert_eq!(refused(search(input).await), code);
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn rebuilding_stores_the_text_of_documents_never_saved_as_snapshots(pool: PgPool) {
    let t = team(pool).await;
    let page = t.general_page("Logbook", "").await;
    // Updates logged, as a live document logs them, but no snapshot: the server stopped
    // before it saved one.
    let doc = Doc::new();
    let fragment = neon_blocks::fragment(&doc);
    let update = {
        let mut txn = doc.transact_mut();
        neon_blocks::append(
            &mut txn,
            &fragment,
            &[Block::paragraph("A starboard entry")],
            &mut SequentialIds::new("b"),
        )
        .expect("a document");
        txn.encode_update_v1()
    };
    documents::append(
        &t.h.pool,
        page,
        &[NewUpdate {
            update: &update,
            author_id: t.ada.user.id,
            created_at: t.h.time.clock().now(),
        }],
    )
    .await
    .expect("logged");
    assert!(t.titles(&t.dee, "starboard").await.is_empty());

    assert_eq!(t.search.reindex(false).await.expect("rebuilt"), 1);
    assert_eq!(t.titles(&t.dee, "starboard").await, ["Logbook"]);
    // Nothing is missing now; every page is rebuilt when asked.
    assert_eq!(t.search.reindex(false).await.expect("rebuilt"), 0);
    t.general_page("Second", "With text\n").await;
    assert_eq!(t.search.reindex(true).await.expect("rebuilt"), 2);
}
