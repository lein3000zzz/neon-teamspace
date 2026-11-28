//! Page history: the versions snapshots keep, with their authors and summaries, listing,
//! reading and comparing them as the rules allow, restoring one through the live document,
//! with editors connected and typing, beside the page's comments, and thinning them out.

use crate::support;

use std::time::Duration;

use bytes::Bytes;
use neon_app::comments::{CommentService, Placing, ThreadQuery};
use neon_app::history::{HistoryError, HistoryService};
use neon_app::pages::{Location, NewPage, PageError};
use neon_app::ratelimit::{Bucket, RateLimiter};
use neon_app::settings::Change as Setting;
use neon_app::sync::{Connection, ContentError, Limits, Next, SyncService, access_feed, revision};
use neon_blocks::diff::Change;
use neon_blocks::edit::Edit;
use neon_blocks::markdown::from_markdown;
use neon_blocks::{Block, Document};
use neon_domain::auth::{Principal, Scope};
use neon_domain::history::Dated;
use neon_domain::pages::{PageAccess, Placement};
use neon_domain::workspaces::WorkspaceRole;
use neon_store::versions::{self, NewVersion};
use serde_json::json;
use sqlx::PgPool;
use support::Harness;
use tokio::time::timeout;
use uuid::Uuid;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{Doc, ReadTxn as _, Text as _, Transact as _, Update, XmlFragment as _, XmlOut};

/// Long enough for any step here; reaching it means something never came.
const PATIENCE: Duration = Duration::from_secs(10);

/// Ada owns Design, with Dee as a member and Gus as a guest; Eve is in no workspace.
struct Team {
    h: Harness,
    workspace: Uuid,
    general: Uuid,
    ada: Principal,
    dee: Principal,
    gus: Principal,
    eve: Principal,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let dee = h.person("dee@example.com").await;
    let gus = h.person("gus@example.com").await;
    let eve = h.person("eve@example.com").await;
    let workspace = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, workspace, &dee, WorkspaceRole::Member)
        .await;
    h.bring_in(&ada, workspace, &gus, WorkspaceRole::Guest)
        .await;
    let general = h
        .workspaces
        .list_teamspaces(&ada, workspace)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    Team {
        h,
        workspace,
        general,
        ada,
        dee,
        gus,
        eve,
    }
}

/// Logs at once and keeps documents loaded until the service stops.
fn quick() -> Limits {
    Limits {
        persist_delay: Duration::from_millis(5),
        idle_unload: Duration::from_secs(3600),
        ..Limits::default()
    }
}

impl Team {
    fn sync(&self) -> SyncService {
        let (_, feed) = access_feed();
        SyncService::builder(
            self.h.pool.clone(),
            self.h.pages.clone(),
            self.h.auth.clone(),
            feed,
        )
        .limits(quick())
        .limiter(std::sync::Arc::clone(&self.h.limiter) as std::sync::Arc<dyn RateLimiter>)
        .clock(self.h.time.clock())
        .build()
    }

    fn history(&self, sync: &SyncService) -> HistoryService {
        HistoryService::builder(
            self.h.pool.clone(),
            std::sync::Arc::clone(&self.h.settings),
            self.h.pages.clone(),
            sync.clone(),
        )
        .clock(self.h.time.clock())
        .build()
    }

    /// A page Ada makes in General with the blocks of `markdown`.
    async fn page(&self, sync: &SyncService, title: &str, markdown: &str) -> Uuid {
        let content = sync
            .first_snapshot(&from_markdown(markdown))
            .await
            .expect("not too large");
        self.h
            .pages
            .create_page(
                &self.ada,
                self.workspace,
                Location::Teamspace(self.general),
                Placement::Last,
                NewPage {
                    title: title.to_owned(),
                    content,
                    ..NewPage::default()
                },
            )
            .await
            .expect("created")
            .page
            .id
    }

    async fn count(&self, page: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM page_versions WHERE page_id = $1")
            .bind(page)
            .fetch_one(&self.h.pool)
            .await
            .expect("counted")
    }
}

fn append(markdown: &str) -> Vec<Edit> {
    vec![Edit::Append {
        blocks: from_markdown(markdown).blocks,
    }]
}

async fn text_of(sync: &SyncService, page: Uuid) -> String {
    sync.content(page).await.expect("content").plain_text()
}

fn content_error(error: HistoryError) -> String {
    match error {
        HistoryError::Content(ContentError::Page(error)) | HistoryError::Page(error) => {
            error.code().to_owned()
        }
        other => other.code().to_owned(),
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_made_with_content_starts_its_history(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "# Goals\n\nShip it\n").await;

    let list = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed");
    assert_eq!(list.versions.len(), 1);
    let first = &list.versions[0];
    assert_eq!(first.authors, [t.ada.user.id]);
    assert_eq!(
        (
            first.summary.added,
            first.summary.changed,
            first.summary.removed
        ),
        (2, 0, 0)
    );
    assert_eq!(first.created_at, t.h.time.clock().now());
    assert_eq!(list.next, None);
    // Its blocks are the document's, ids and all.
    let read = history.version(&t.ada, page, first.id).await.expect("read");
    assert_eq!(read.document, sync.content(page).await.expect("content"));

    // A page made empty has none until something is written.
    let empty =
        t.h.pages
            .create_page(
                &t.ada,
                t.workspace,
                Location::Teamspace(t.general),
                Placement::Last,
                NewPage::default(),
            )
            .await
            .expect("created")
            .page
            .id;
    assert_eq!(t.count(empty).await, 0);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn each_snapshot_that_changed_the_page_keeps_a_version_with_who_changed_it(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let page = t.page(&sync, "Plan", "First\n").await;
    let agent = t.h.agent(&t.ada, t.workspace, "Scout").await;
    let (_, scout) = t.h.agent_token(&t.ada, &agent, &[Scope::Write]).await;

    t.h.time.advance(Duration::from_secs(60));
    sync.edit(&t.dee, page, append("By Dee"), None)
        .await
        .expect("edited");
    sync.edit(&scout, page, append("By Scout"), None)
        .await
        .expect("edited");
    // Stopping saves every live document as a snapshot.
    sync.shutdown().await;

    let sync = t.sync();
    let history = t.history(&sync);
    let list = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed");
    assert_eq!(list.versions.len(), 2);
    let latest = &list.versions[0];
    let mut authors = vec![t.dee.user.id, scout.user.id];
    authors.sort_unstable();
    assert_eq!(latest.authors, authors);
    assert_eq!(latest.summary.added, 2);
    assert_eq!(latest.created_at, t.h.time.clock().now());

    // A snapshot that changes nothing that shows keeps no version.
    let read = sync.read(&t.ada, page).await.expect("read").1;
    sync.edit(
        &t.dee,
        page,
        vec![Edit::ReplaceBody {
            blocks: read.blocks.clone(),
        }],
        Some(revision(&read)),
    )
    .await
    .expect("edited");
    sync.shutdown().await;
    assert_eq!(t.count(page).await, 2);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn lists_go_a_part_at_a_time_latest_first(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "Zero\n").await;
    let base = t.h.time.clock().now();
    for n in 1..=4i64 {
        versions::insert(
            &t.h.pool,
            NewVersion {
                id: Uuid::now_v7(),
                page_id: page,
                created_at: base + time::Duration::minutes(n),
                authors: &[t.dee.user.id],
                content: &json!([]),
                added: 0,
                changed: i32::try_from(n).expect("small"),
                removed: 0,
                restored_from: None,
            },
        )
        .await
        .expect("kept");
    }
    let first = history
        .versions(&t.ada, page, None, Some(3))
        .await
        .expect("listed");
    let changed: Vec<usize> = first.versions.iter().map(|v| v.summary.changed).collect();
    assert_eq!(changed, [4, 3, 2]);
    let rest = history
        .versions(&t.ada, page, first.next.as_deref(), Some(3))
        .await
        .expect("listed");
    let changed: Vec<usize> = rest.versions.iter().map(|v| v.summary.changed).collect();
    assert_eq!(changed, [1, 0]);
    assert_eq!(rest.next, None);

    for (after, limit) in [
        (Some("not a cursor"), None),
        (None, Some(0)),
        (None, Some(101)),
    ] {
        let refused = history
            .versions(&t.ada, page, after, limit)
            .await
            .expect_err("refused");
        assert_eq!(refused.code(), "invalid_input");
    }
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn history_shows_to_those_who_edit_the_page_and_guests_get_no_authors(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "First\n").await;
    let version = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;

    // Nobody outside the workspace, and no guest the page is not shared with, knows of it.
    for who in [&t.eve, &t.gus] {
        let refused = history
            .versions(who, page, None, None)
            .await
            .expect_err("hidden");
        assert_eq!(refused.code(), "not_found");
        let refused = history
            .compare(who, page, version)
            .await
            .expect_err("hidden");
        assert_eq!(refused.code(), "not_found");
    }

    // A member who edits it sees it; a guest it is shared with to edit sees it without who
    // made the versions.
    let seen = history
        .versions(&t.dee, page, None, None)
        .await
        .expect("listed");
    assert_eq!(seen.versions[0].authors, [t.ada.user.id]);
    t.h.pages
        .share_page(&t.ada, page, t.gus.user.id, PageAccess::Edit)
        .await
        .expect("shared");
    let seen = history
        .versions(&t.gus, page, None, None)
        .await
        .expect("listed");
    assert!(
        seen.versions[0].authors.is_empty(),
        "guests know no members"
    );
    let read = history.version(&t.gus, page, version).await.expect("read");
    assert!(read.version.authors.is_empty());
    // An agent sees it once it and its owner both edit the page, whatever its token's
    // scopes, as reading is all it does.
    let agent = t.h.agent(&t.ada, t.workspace, "Reader").await;
    let (_, reader) = t.h.agent_token(&t.ada, &agent, &[Scope::Read]).await;
    history
        .versions(&reader, page, None, None)
        .await
        .expect("listed");

    // A version of another page, or of none, is not found.
    let other = t.page(&sync, "Other", "Else\n").await;
    for (page, version) in [(other, version), (page, Uuid::now_v7())] {
        let refused = history
            .version(&t.ada, page, version)
            .await
            .expect_err("no such version");
        assert!(matches!(refused, HistoryError::NoVersion));
    }

    // A database has no body, and no history.
    let database =
        t.h.databases
            .create_database(
                &t.ada,
                t.workspace,
                Location::Teamspace(t.general),
                Placement::Last,
                neon_app::databases::NewDatabase {
                    title: "Tasks".to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("created");
    let refused = history
        .versions(&t.ada, database.page.page.id, None, None)
        .await
        .expect_err("refused");
    assert_eq!(refused.code(), "invalid_input");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn those_who_only_view_a_page_see_none_of_what_it_held_before(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let pay = t
        .page(&sync, "Pay", "Salary band: 9999 confidential\n")
        .await;
    let read = sync.read(&t.ada, pay).await.expect("read").1;
    sync.edit(
        &t.ada,
        pay,
        vec![Edit::ReplaceBody {
            blocks: from_markdown("See HR.\n").blocks,
        }],
        Some(revision(&read)),
    )
    .await
    .expect("edited");
    let notes =
        t.h.pages
            .create_page(
                &t.ada,
                t.workspace,
                Location::Private,
                Placement::Last,
                NewPage {
                    title: "Notes".to_owned(),
                    content: sync
                        .first_snapshot(&from_markdown("Draft figures\n"))
                        .await
                        .expect("not too large"),
                    ..NewPage::default()
                },
            )
            .await
            .expect("created")
            .page
            .id;
    // Stopping saves Pay as it is now, its second version.
    sync.shutdown().await;
    let sync = t.sync();
    let history = t.history(&sync);
    let ids = |list: neon_app::history::VersionList| -> Vec<Uuid> {
        list.versions.iter().map(|version| version.id).collect()
    };
    let pay_versions = ids(history
        .versions(&t.ada, pay, None, None)
        .await
        .expect("listed"));
    assert_eq!(pay_versions.len(), 2);
    let notes_versions = ids(history
        .versions(&t.ada, notes, None, None)
        .await
        .expect("listed"));

    // Pay is shared with a guest to view once the figure is gone, and Ada's private notes
    // with her agent to view: neither sees the history, nor what the page held before,
    // though both see the page as it is.
    t.h.pages
        .share_page(&t.ada, pay, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let agent = t.h.agent(&t.ada, t.workspace, "Viewer").await;
    let (_, viewer) =
        t.h.agent_token(&t.ada, &agent, &[Scope::Read, Scope::Write])
            .await;
    t.h.pages
        .share_page(&t.ada, notes, agent.id, PageAccess::View)
        .await
        .expect("shared");
    for (who, page, versions) in [
        (&t.gus, pay, pay_versions),
        (&viewer, notes, notes_versions),
    ] {
        t.h.pages.page(who, page).await.expect("seen");
        let refused = history
            .versions(who, page, None, None)
            .await
            .expect_err("forbidden");
        assert_eq!(refused.code(), "forbidden");
        for version in versions {
            let read = history.version(who, page, version).await.map(|_| ());
            let compared = history.compare(who, page, version).await.map(|_| ());
            for refused in [read, compared] {
                assert_eq!(refused.expect_err("forbidden").code(), "forbidden");
            }
        }
    }
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_comparison_tells_block_by_block_what_changed_since(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "# Goals\n\nShip it\n\nOld\n").await;
    let version = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    let read = sync.read(&t.ada, page).await.expect("read").1;
    let (ship, old) = (
        read.blocks[1].id.clone().expect("an id"),
        read.blocks[2].id.clone().expect("an id"),
    );
    sync.edit(
        &t.dee,
        page,
        vec![
            Edit::FindReplace {
                find: "Ship it".to_owned(),
                replace: "Ship it now".to_owned(),
                all: false,
            },
            Edit::ReplaceRange {
                from: old.clone(),
                to: old,
                blocks: from_markdown("New\n").blocks,
            },
        ],
        Some(revision(&read)),
    )
    .await
    .expect("edited");

    // What is being edited counts, saved or not.
    let compared = history
        .compare(&t.ada, page, version)
        .await
        .expect("compared");
    let lines: Vec<(Change, String)> = compared
        .lines
        .iter()
        .map(|line| {
            let block = line.after.clone().or(line.before.clone()).expect("a block");
            (line.change, Document::new(vec![block]).plain_text())
        })
        .collect();
    assert_eq!(
        lines,
        [
            (Change::Same, "Goals".to_owned()),
            (Change::Changed, "Ship it now".to_owned()),
            (Change::Removed, "Old".to_owned()),
            (Change::Added, "New".to_owned()),
        ]
    );
    assert_eq!(
        compared.lines[1].after.as_ref().and_then(|b| b.id.clone()),
        Some(ship)
    );
    assert_eq!(
        (
            compared.summary.added,
            compared.summary.changed,
            compared.summary.removed
        ),
        (1, 1, 1)
    );
    let now = sync.content(page).await.expect("content");
    assert_eq!(compared.revision, revision(&now));
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn restoring_brings_a_version_back_and_keeps_what_it_replaced(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "# Goals\n\nShip it\n").await;
    let first = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .clone();
    let original = history
        .version(&t.ada, page, first.id)
        .await
        .expect("read")
        .document;
    t.h.time.advance(Duration::from_secs(60));
    sync.edit(&t.dee, page, append("Added later"), None)
        .await
        .expect("edited");

    let compared = history
        .compare(&t.dee, page, first.id)
        .await
        .expect("compared");
    t.h.time.advance(Duration::from_secs(60));
    let restored = history
        .restore(&t.dee, page, first.id, compared.revision)
        .await
        .expect("restored");
    assert_eq!(restored.version.id, first.id);
    assert_eq!(sync.content(page).await.expect("content"), original);
    assert_eq!(
        restored.revision,
        Some(revision(&sync.content(page).await.expect("content")))
    );

    // The page as it was went into the history first, then the restore did, naming the
    // version restored.
    let list = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions;
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].restored_from, Some(first.id));
    assert_eq!(list[0].restored_from_at, Some(first.created_at));
    assert_eq!(list[0].authors, [t.dee.user.id]);
    assert_eq!(list[0].summary.removed, 1);
    let before = history
        .version(&t.ada, page, list[1].id)
        .await
        .expect("read")
        .document;
    assert_eq!(before.plain_text(), "Goals\nShip it\nAdded later");
    assert_eq!(list[1].authors, [t.dee.user.id]);
    assert_eq!(list[2].id, first.id);

    // Restoring that one undoes the restore.
    let compared = history
        .compare(&t.ada, page, list[1].id)
        .await
        .expect("compared");
    history
        .restore(&t.ada, page, list[1].id, compared.revision)
        .await
        .expect("restored");
    assert_eq!(text_of(&sync, page).await, "Goals\nShip it\nAdded later");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_restore_keeps_the_comments_and_detaches_those_on_text_it_writes_anew(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let comments = CommentService::new(t.h.pages.clone(), sync.clone());
    let page = t.page(&sync, "Plan", "Ship it\n\nKeep this\n").await;
    let first = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    let ids: Vec<String> = sync
        .content(page)
        .await
        .expect("content")
        .blocks
        .iter()
        .map(|block| block.id.clone().expect("an id"))
        .collect();
    let mut threads = Vec::new();
    for (words, placing) in [
        ("On the page", Placing::Page),
        ("On shipping", Placing::Block(ids[0].clone())),
        ("On keeping", Placing::Block(ids[1].clone())),
    ] {
        let posted = comments
            .start_thread(&t.ada, page, words, placing)
            .await
            .expect("started");
        threads.push(posted.thread.thread.id);
    }
    t.h.time.advance(Duration::from_secs(60));
    sync.edit(
        &t.dee,
        page,
        vec![Edit::FindReplace {
            find: "it".to_owned(),
            replace: "it today".to_owned(),
            all: false,
        }],
        None,
    )
    .await
    .expect("edited");
    let anchors = |listed: &neon_app::comments::PageThreads| -> Vec<(Uuid, Option<bool>)> {
        listed
            .threads
            .iter()
            .map(|thread| {
                (
                    thread.thread.id,
                    thread.anchored.as_ref().map(|anchored| anchored.detached),
                )
            })
            .collect()
    };
    let before = comments
        .threads(&t.ada, page, ThreadQuery::default())
        .await
        .expect("listed");
    assert_eq!(
        anchors(&before),
        [
            (threads[0], None),
            (threads[1], Some(false)),
            (threads[2], Some(false))
        ]
    );

    let compared = history
        .compare(&t.ada, page, first)
        .await
        .expect("compared");
    history
        .restore(&t.ada, page, first, compared.revision)
        .await
        .expect("restored");
    assert_eq!(text_of(&sync, page).await, "Ship it\nKeep this");
    // Versions hold the blocks alone: every thread stays, with its comments. The block the
    // restore wrote anew no longer holds the text its thread was on; the other still does.
    let after = comments
        .threads(&t.ada, page, ThreadQuery::default())
        .await
        .expect("listed");
    assert_eq!(
        anchors(&after),
        [
            (threads[0], None),
            (threads[1], Some(true)),
            (threads[2], Some(false))
        ]
    );
    assert!(after.threads.iter().all(|thread| thread.count == 1));
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_restore_that_changes_nothing_names_no_version(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "First\n").await;
    let only = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    let compared = history.compare(&t.ada, page, only).await.expect("compared");
    let summary = compared.summary;
    assert_eq!((summary.added, summary.changed, summary.removed), (0, 0, 0));
    history
        .restore(&t.ada, page, only, compared.revision)
        .await
        .expect("restored");
    assert_eq!(t.count(page).await, 1);

    // The next version is an ordinary one.
    sync.edit(&t.ada, page, append("An ordinary later edit"), None)
        .await
        .expect("edited");
    sync.shutdown().await;
    let sync = t.sync();
    let list = t
        .history(&sync)
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions;
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].restored_from, None);
    assert_eq!(list[0].restored_from_at, None);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn restores_count_against_a_rate_of_their_own(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "First\n").await;
    let first = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    sync.edit(&t.ada, page, append("Second"), None)
        .await
        .expect("edited");
    let base = history
        .compare(&t.ada, page, first)
        .await
        .expect("compared")
        .revision;

    t.h.limiter.refuse(Bucket::PageRestore);
    let refused = history
        .restore(&t.ada, page, first, base)
        .await
        .expect_err("limited");
    assert_eq!(content_error(refused), "rate_limited");
    assert!(
        t.h.limiter
            .seen()
            .contains(&(Bucket::PageRestore, t.ada.user.id.to_string()))
    );
    assert_eq!(text_of(&sync, page).await, "First\nSecond");
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_keeps_no_more_versions_than_the_cap_between_cleanups(pool: PgPool) {
    let t = team(pool).await;
    t.h.settings
        .update(
            [("history.max_versions".to_owned(), Setting::Set(json!(3)))],
            None,
        )
        .await
        .expect("set");
    let sync = t.sync();
    let page = t.page(&sync, "Plan", "Zero\n").await;
    sync.shutdown().await;
    for n in 1..=4 {
        t.h.time.advance(Duration::from_secs(60));
        let sync = t.sync();
        sync.edit(&t.ada, page, append(&format!("Edit {n}")), None)
            .await
            .expect("edited");
        sync.shutdown().await;
        assert!(t.count(page).await <= 3, "after edit {n}");
    }

    // Restoring two versions in turn, over and over, keeps no more either.
    let sync = t.sync();
    let history = t.history(&sync);
    let list = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions;
    assert_eq!(list.len(), 3);
    assert_eq!(
        history
            .version(&t.ada, page, list[0].id)
            .await
            .expect("read")
            .document
            .plain_text(),
        "Zero\nEdit 1\nEdit 2\nEdit 3\nEdit 4"
    );
    let mut pick = [list[1].id, list[0].id];
    for _ in 0..4 {
        t.h.time.advance(Duration::from_secs(60));
        let base = history
            .compare(&t.ada, page, pick[0])
            .await
            .expect("compared")
            .revision;
        history
            .restore(&t.ada, page, pick[0], base)
            .await
            .expect("restored");
        assert!(t.count(page).await <= 3);
        // The newest versions are the restore and what it replaced; alternate between them.
        let latest = history
            .versions(&t.ada, page, None, Some(2))
            .await
            .expect("listed")
            .versions;
        pick = [latest[1].id, latest[0].id];
    }
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_restore_from_a_stale_comparison_is_refused_and_changes_nothing(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "First\n").await;
    let first = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    sync.edit(&t.ada, page, append("Second"), None)
        .await
        .expect("edited");
    let compared = history
        .compare(&t.dee, page, first)
        .await
        .expect("compared");
    // Ada writes on while Dee looks at the comparison.
    sync.edit(&t.ada, page, append("Third"), None)
        .await
        .expect("edited");

    let refused = history
        .restore(&t.dee, page, first, compared.revision)
        .await
        .expect_err("refused");
    assert_eq!(refused.code(), "edit_conflict");
    assert_eq!(text_of(&sync, page).await, "First\nSecond\nThird");
    sync.shutdown().await;
    assert_eq!(
        t.count(page).await,
        2,
        "nothing restored, nothing kept for it"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn restoring_needs_editing_the_page(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "First\n").await;
    let first = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    let base = history
        .compare(&t.ada, page, first)
        .await
        .expect("compared")
        .revision;

    // A guest it is shared with to view, and a token that only reads, do not restore it.
    t.h.pages
        .share_page(&t.ada, page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let agent = t.h.agent(&t.ada, t.workspace, "Reader").await;
    let (_, reader) = t.h.agent_token(&t.ada, &agent, &[Scope::Read]).await;
    for who in [&t.gus, &reader] {
        let refused = history
            .restore(who, page, first, base.clone())
            .await
            .expect_err("refused");
        assert_eq!(content_error(refused), "forbidden");
    }
    // Nor does anyone while it is in the trash.
    t.h.pages.trash_page(&t.ada, page).await.expect("trashed");
    let refused = history
        .restore(&t.ada, page, first, base)
        .await
        .expect_err("refused");
    assert_eq!(content_error(refused), "forbidden");
    let refused = history
        .versions(&t.eve, page, None, None)
        .await
        .expect_err("hidden");
    assert!(matches!(
        refused,
        HistoryError::Page(PageError::NotFound(_))
    ));
    sync.shutdown().await;
}

/// A client with a document of its own, talking to a connection directly.
struct Client {
    connection: Connection,
    doc: Doc,
}

impl Client {
    async fn open(sync: &SyncService, who: &Principal, page: Uuid) -> Self {
        let admission = sync.admit(who.clone(), page).await.expect("admitted");
        let connection = sync.connect(admission).await.expect("connected");
        let mut client = Self {
            connection,
            doc: Doc::new(),
        };
        let state_vector = client.doc.transact().state_vector();
        client
            .send(&Message::Sync(SyncMessage::SyncStep1(state_vector)))
            .await;
        client
            .until(|message| matches!(message, Message::Sync(SyncMessage::SyncStep2(_))))
            .await;
        client
    }

    async fn send(&mut self, message: &Message) {
        self.connection
            .receive(Bytes::from(message.encode_v1()))
            .await
            .expect("sent");
    }

    /// Messages from the server, applied, until one `done` says is the one awaited.
    async fn until(&mut self, mut done: impl FnMut(&Message) -> bool) {
        loop {
            let next = timeout(PATIENCE, self.connection.next())
                .await
                .expect("something comes");
            let Next::Send(data) = next else {
                panic!("closed");
            };
            let message = Message::decode_v1(&data).expect("a y-sync message");
            if let Message::Sync(SyncMessage::SyncStep1(state_vector)) = &message {
                let update = self.doc.transact().encode_state_as_update_v1(state_vector);
                self.send(&Message::Sync(SyncMessage::SyncStep2(update)))
                    .await;
            }
            if let Message::Sync(SyncMessage::SyncStep2(update) | SyncMessage::Update(update)) =
                &message
            {
                self.doc
                    .transact_mut()
                    .apply_update(Update::decode_v1(update).expect("an update"))
                    .expect("applies");
            }
            if done(&message) {
                return;
            }
        }
    }

    /// Text typed at the end of the block at `index`, as the update that types it.
    fn type_into(&mut self, index: u32, text: &str) -> Message {
        let fragment = neon_blocks::fragment(&self.doc);
        let mut txn = self.doc.transact_mut();
        let Some(XmlOut::Element(group)) = fragment.first_child() else {
            panic!("a document");
        };
        let Some(XmlOut::Element(container)) = group.get(&txn, index) else {
            panic!("a block");
        };
        let Some(XmlOut::Element(content)) = container.first_child() else {
            panic!("its content");
        };
        let Some(XmlOut::Text(run)) = content.first_child() else {
            panic!("its text");
        };
        let end = run.len(&txn);
        run.insert(&mut txn, end, text);
        Message::Sync(SyncMessage::Update(txn.encode_update_v1()))
    }

    fn blocks(&self) -> Vec<Block> {
        neon_blocks::read_doc(&self.doc).expect("blocks").blocks
    }
}

fn texts(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|block| Document::new(vec![block.clone()]).plain_text())
        .collect()
}

fn is_update(message: &Message) -> bool {
    matches!(message, Message::Sync(SyncMessage::Update(_)))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_restore_reaches_the_editors_and_merges_with_what_they_type_meanwhile(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "Kept\n\nDraft\n").await;
    let first = history
        .versions(&t.ada, page, None, None)
        .await
        .expect("listed")
        .versions[0]
        .id;
    let mut ada = Client::open(&sync, &t.ada, page).await;
    let mut dee = Client::open(&sync, &t.dee, page).await;

    // Ada changes the draft, which Dee sees.
    let typed = ada.type_into(1, " v2");
    ada.send(&typed).await;
    dee.until(is_update).await;
    assert_eq!(texts(&dee.blocks()), ["Kept", "Draft v2"]);

    // Ada types in the block the version shares with the page; the server has not heard of
    // it yet when the restore of the version, compared just now, is made.
    let compared = history
        .compare(&t.dee, page, first)
        .await
        .expect("compared");
    let meanwhile = ada.type_into(0, " and typed");
    history
        .restore(&t.dee, page, first, compared.revision)
        .await
        .expect("restored");
    ada.send(&meanwhile).await;

    // Both editors get the restore, and Dee Ada's typing; all end with the same page.
    ada.until(is_update).await;
    let expected = ["Kept and typed", "Draft"];
    let both_heard = async {
        while texts(&dee.blocks()) != expected {
            dee.until(is_update).await;
        }
    };
    timeout(PATIENCE, both_heard)
        .await
        .expect("Dee hears both changes");
    assert_eq!(texts(&ada.blocks()), expected);
    assert_eq!(
        texts(&sync.content(page).await.expect("content").blocks),
        expected
    );
    assert_eq!(ada.blocks(), dee.blocks());
    drop((ada, dee));
    sync.shutdown().await;
}

/// The ids of a page's versions, in order.
async fn remaining(history: &HistoryService, who: &Principal, page: Uuid) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = history
        .versions(who, page, None, None)
        .await
        .expect("listed")
        .versions
        .into_iter()
        .map(|version| version.id)
        .collect();
    ids.sort_unstable();
    ids
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_cleanup_thins_versions_as_the_settings_say(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let page = t.page(&sync, "Plan", "First\n").await;
    let now = t.h.time.clock().now();
    let at_hours = |hours: i64| now - time::Duration::hours(hours);
    // The first version, now; then three an hour apart two days ago, and two on each of two
    // days ten and eleven days ago.
    let mut kept = vec![(
        history
            .versions(&t.ada, page, None, None)
            .await
            .expect("listed")
            .versions[0]
            .id,
        now,
    )];
    for hours in [48, 49, 50, 240, 241, 264, 265] {
        let id = Uuid::now_v7();
        versions::insert(
            &t.h.pool,
            NewVersion {
                id,
                page_id: page,
                created_at: at_hours(hours),
                authors: &[],
                content: &json!([]),
                added: 0,
                changed: 0,
                removed: 0,
                restored_from: None,
            },
        )
        .await
        .expect("kept");
        kept.push((id, at_hours(hours)));
    }
    let expect = |kept: &[(Uuid, time::OffsetDateTime)], times: &[i64]| {
        let mut ids: Vec<Uuid> = kept
            .iter()
            .filter(|(_, at)| times.iter().any(|hours| at_hours(*hours) == *at) || *at == now)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    };

    // By default every version of the last week stays, and older ones thin to one a day.
    assert_eq!(history.thin().await.expect("thinned"), 2);
    assert_eq!(
        remaining(&history, &t.ada, page).await,
        expect(&kept, &[48, 49, 50, 240, 264])
    );

    // Kept whole a day, one a day for three days, then one a week, at most four.
    t.h.set("history.keep_all", json!("1d")).await;
    t.h.set("history.keep_daily", json!("3d")).await;
    t.h.set("history.max_versions", json!(4)).await;
    history.thin().await.expect("thinned");
    let left = remaining(&history, &t.ada, page).await;
    assert_eq!(left.len(), 3, "now, two days ago, and one week");
    assert!(left.contains(&kept[0].0), "the latest stays");
    let dated: Vec<Dated> = kept
        .iter()
        .filter(|(id, _)| left.contains(id))
        .map(|&(id, at)| Dated { id, at })
        .collect();
    assert!(dated.iter().any(|version| version.at == at_hours(48)));

    // A page's versions go with it.
    t.h.pages.trash_page(&t.ada, page).await.expect("trashed");
    t.h.pages.delete_page(&t.ada, page).await.expect("deleted");
    assert_eq!(t.count(page).await, 0);
    sync.shutdown().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_row_s_body_has_versions_like_a_page(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync();
    let history = t.history(&sync);
    let database =
        t.h.databases
            .create_database(
                &t.ada,
                t.workspace,
                Location::Teamspace(t.general),
                Placement::Last,
                neon_app::databases::NewDatabase {
                    title: "Tasks".to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("created");
    let row =
        t.h.databases
            .create_row(
                &t.ada,
                database.page.page.id,
                neon_app::databases::NewRow {
                    values: serde_json::from_value(json!({"Name": "Fix login"})).expect("values"),
                    ..neon_app::databases::NewRow::default()
                },
            )
            .await
            .expect("created")
            .id;
    sync.edit(&t.dee, row, append("Steps to reproduce"), None)
        .await
        .expect("edited");
    sync.shutdown().await;

    let list = history
        .versions(&t.ada, row, None, None)
        .await
        .expect("listed");
    assert_eq!(list.versions.len(), 1);
    assert_eq!(list.versions[0].authors, [t.dee.user.id]);
}
