//! Imports and exports: zips of markdown into nested pages with their content, the rules
//! and limits around them, exports of pages, their subpages and teamspaces, the live
//! document's content in an export, and a round trip.

mod support;

use std::collections::BTreeMap;
use std::io::{Cursor, Read as _, Write as _};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use neon_app::pages::{Location, NewPage, PageError, Tree};
use neon_app::ratelimit::{Bucket, RateLimiter};
use neon_app::sync::{Connection, Limits as SyncLimits, Next, SyncService, access_feed};
use neon_app::transfer::{
    Export, Format, ImportReport, Limits, SkipReason, Skipped, TransferError, TransferService,
    Upload,
};
use neon_app::workspaces::NewTeamspace;
use neon_blocks::{Block, SequentialIds};
use neon_domain::auth::Principal;
use neon_domain::pages::{MAX_DEPTH, PageAccess, Placement};
use neon_domain::workspaces::{TeamspaceAccess, WorkspaceRole};
use sqlx::PgPool;
use support::Harness;
use tokio::time::timeout;
use uuid::Uuid;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{
    Doc, ReadTxn as _, Transact as _, TransactionMut, Update, XmlElementPrelim, XmlFragment as _,
    XmlFragmentRef, XmlOut,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const PATIENCE: Duration = Duration::from_secs(10);

/// Ada owns Design, with Dee as a member and Gus as a guest. General is the default
/// teamspace; Secret is private to Ada.
struct Team {
    h: Harness,
    sync: SyncService,
    transfer: TransferService,
    workspace: Uuid,
    general: Uuid,
    secret: Uuid,
    ada: Principal,
    dee: Principal,
    gus: Principal,
}

async fn team(pool: PgPool) -> Team {
    team_with(pool, Limits::default()).await
}

async fn team_with(pool: PgPool, limits: Limits) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let dee = h.person("dee@example.com").await;
    let gus = h.person("gus@example.com").await;
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
    let secret = teamspace(&h, &ada, workspace, "Secret", TeamspaceAccess::Private).await;
    let (_, feed) = access_feed();
    // Updates wait an hour to be logged, so what an export reads of an open page comes
    // from the live document alone.
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed)
        .limits(SyncLimits {
            persist_delay: Duration::from_secs(3600),
            idle_unload: Duration::from_secs(3600),
            ..SyncLimits::default()
        })
        .build();
    let transfer = TransferService::builder(h.pages.clone(), sync.clone())
        .limiter(Arc::clone(&h.limiter) as Arc<dyn RateLimiter>)
        .limits(limits)
        .build();
    Team {
        h,
        sync,
        transfer,
        workspace,
        general,
        secret,
        ada,
        dee,
        gus,
    }
}

async fn teamspace(
    h: &Harness,
    by: &Principal,
    workspace: Uuid,
    name: &str,
    access: TeamspaceAccess,
) -> Uuid {
    h.workspaces
        .create_teamspace(
            by,
            workspace,
            NewTeamspace {
                name: name.to_owned(),
                icon: None,
                hue: None,
                access,
            },
        )
        .await
        .expect("created")
        .teamspace
        .id
}

/// A zip of `files`, each a name and its content; a name ending in `/` is a folder.
fn zipped(files: &[(&str, &str)]) -> Bytes {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in files {
        if name.ends_with('/') {
            writer
                .add_directory(*name, SimpleFileOptions::default())
                .expect("a folder");
        } else {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("a file");
            writer.write_all(content.as_bytes()).expect("written");
        }
    }
    Bytes::from(writer.finish().expect("finished").into_inner())
}

/// Each file of a zip, by name.
fn unzipped(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip");
    (0..zip.len())
        .map(|i| {
            let mut file = zip.by_index(i).expect("an entry");
            let mut text = String::new();
            file.read_to_string(&mut text).expect("text");
            (file.name().expect("a name").into_owned(), text)
        })
        .collect()
}

fn names(files: &BTreeMap<String, String>) -> Vec<&str> {
    files.keys().map(String::as_str).collect()
}

const ROADMAP: &[(&str, &str)] = &[
    ("Roadmap.md", "# Roadmap\n\nWhere we are going.\n"),
    ("Roadmap/", ""),
    ("Roadmap/Q1.md", "# Q1: Goals\n\n- [ ] ship\n- [x] plan\n"),
    ("Roadmap/Q1/Hiring.md", "Two people.\n"),
    ("Roadmap/diagram.png", "not really a png"),
    (
        "Archive/Old plan.md",
        "---\ntitle: Old\n---\n# Old plan\n\n> It was *fine*.\n",
    ),
    (
        "Welcome.md",
        "# Welcome\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
    ),
];

impl Team {
    async fn import(
        &self,
        by: &Principal,
        at: Location,
        name: &str,
        bytes: Bytes,
    ) -> Result<ImportReport, TransferError> {
        let ticket = self.transfer.admit_import(by, self.workspace, at).await?;
        self.transfer
            .import(
                ticket,
                Upload {
                    name: name.to_owned(),
                    bytes,
                },
            )
            .await
    }

    async fn roadmap(&self) -> ImportReport {
        self.import(
            &self.ada,
            Location::Teamspace(self.general),
            "export.zip",
            zipped(ROADMAP),
        )
        .await
        .expect("imported")
    }

    /// A teamspace's tree as each page's title, indented two spaces a level.
    async fn outline(&self, by: &Principal, teamspace: Uuid) -> Vec<String> {
        let Tree { pages, .. } = self
            .h
            .pages
            .teamspace_tree(by, teamspace, None, None)
            .await
            .expect("listed");
        let mut depth: BTreeMap<Uuid, usize> = BTreeMap::new();
        pages
            .iter()
            .map(|page| {
                let level = page
                    .parent_id
                    .and_then(|parent| depth.get(&parent))
                    .map_or(0, |level| level + 1);
                depth.insert(page.id, level);
                format!("{}{}", "  ".repeat(level), page.title)
            })
            .collect()
    }

    /// The page titled `title` in `teamspace`.
    async fn id_of(&self, teamspace: Uuid, title: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM pages WHERE teamspace_id = $1 AND title = $2")
            .bind(teamspace)
            .bind(title)
            .fetch_one(&self.h.pool)
            .await
            .expect("a page")
    }

    async fn export(&self, page: Uuid, children: bool) -> Export {
        self.transfer
            .export_page(&self.ada, page, children)
            .await
            .expect("exported")
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_zip_becomes_nested_pages_with_their_content(pool: PgPool) {
    let t = team(pool).await;

    let report = t.roadmap().await;

    let tops: Vec<&str> = report
        .pages
        .iter()
        .map(|view| view.page.title.as_str())
        .collect();
    assert_eq!(tops, ["Roadmap", "Archive", "Welcome"]);
    assert!(
        report
            .pages
            .iter()
            .all(|view| view.access == PageAccess::Edit)
    );
    assert_eq!(report.count, 6);
    assert_eq!(
        report.skipped,
        [Skipped {
            path: "Roadmap/diagram.png".to_owned(),
            reason: SkipReason::Attachment,
        }]
    );
    assert_eq!(
        t.outline(&t.ada, t.general).await,
        [
            "Roadmap",
            "  Q1: Goals",
            "    Hiring",
            "Archive",
            "  Old plan",
            "Welcome"
        ]
    );

    // Each file's content is its page's first snapshot, its title's heading left out, and
    // its text the page's search text.
    let q1 = t.id_of(t.general, "Q1: Goals").await;
    let content = t.sync.content(q1).await.expect("content");
    assert_eq!(content.plain_text(), "ship\nplan");
    let (last_seq, search): (i64, Option<String>) = sqlx::query_as(
        "SELECT s.last_seq, p.search_text FROM page_snapshots s JOIN pages p ON p.id = s.page_id
         WHERE s.page_id = $1",
    )
    .bind(q1)
    .fetch_one(&t.h.pool)
    .await
    .expect("a snapshot");
    assert_eq!((last_seq, search.as_deref()), (0, Some("ship\nplan")));
    let old = t
        .sync
        .content(t.id_of(t.general, "Old plan").await)
        .await
        .expect("content");
    assert_eq!(old.plain_text(), "It was fine.");
    // A folder without a file of its name is an empty page, without a snapshot.
    let archive = t.id_of(t.general, "Archive").await;
    assert!(t.sync.content(archive).await.expect("content").is_empty());
    let snapshots: i64 = t.h.scalar("SELECT count(*) FROM page_snapshots").await;
    assert_eq!(snapshots, 5);

    // A markdown file alone is one page, at the end of where it goes.
    let one = t
        .import(
            &t.ada,
            Location::Teamspace(t.general),
            "Notes from Monday.md",
            Bytes::from_static(b"No heading here.\n"),
        )
        .await
        .expect("imported");
    assert_eq!(one.count, 1);
    assert_eq!(one.pages[0].page.title, "Notes from Monday");
    assert!(one.skipped.is_empty());
    let outline = t.outline(&t.ada, t.general).await;
    assert_eq!(
        outline.last().map(String::as_str),
        Some("Notes from Monday")
    );

    // Under a page, and among private pages.
    let welcome = t.id_of(t.general, "Welcome").await;
    let under = t
        .import(
            &t.dee,
            Location::Under(welcome),
            "more.zip",
            zipped(&[("a.md", "a"), ("b.md", "b")]),
        )
        .await
        .expect("imported");
    assert!(
        under
            .pages
            .iter()
            .all(|view| view.page.parent_id == Some(welcome))
    );
    let private = t
        .import(
            &t.dee,
            Location::Private,
            "mine.md",
            Bytes::from_static(b"mine"),
        )
        .await
        .expect("imported");
    assert_eq!(private.pages[0].page.space.owner_id(), Some(t.dee.user.id));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn imports_keep_to_the_rules(pool: PgPool) {
    let t = team(pool).await;
    let page =
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
    t.h.pages
        .share_page(&t.ada, page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let refused = |result: Result<_, TransferError>| match result {
        Err(TransferError::Page(error)) => error,
        Err(other) => panic!("refused otherwise: {other:?}"),
        Ok(_) => panic!("let in"),
    };

    let outsider = refused(
        t.transfer
            .admit_import(&t.dee, t.workspace, Location::Teamspace(t.secret))
            .await,
    );
    assert!(matches!(outsider, PageError::NotFound(_)), "{outsider:?}");
    let viewer = refused(
        t.transfer
            .admit_import(&t.gus, t.workspace, Location::Under(page))
            .await,
    );
    assert!(matches!(viewer, PageError::Forbidden(_)), "{viewer:?}");
    let guest = refused(
        t.transfer
            .admit_import(&t.gus, t.workspace, Location::Private)
            .await,
    );
    assert!(matches!(guest, PageError::Forbidden(_)), "{guest:?}");
    let elsewhere = refused(
        t.transfer
            .admit_import(&t.ada, Uuid::now_v7(), Location::Private)
            .await,
    );
    assert!(matches!(elsewhere, PageError::NotFound(_)), "{elsewhere:?}");

    // Pages that would nest past the deepest level are refused as a whole.
    let mut deepest = page;
    for _ in 1..MAX_DEPTH - 1 {
        deepest =
            t.h.pages
                .create_page(
                    &t.ada,
                    t.workspace,
                    Location::Under(deepest),
                    Placement::Last,
                    NewPage::default(),
                )
                .await
                .expect("created")
                .page
                .id;
    }
    let before: i64 = t.h.scalar("SELECT count(*) FROM pages").await;
    let deep = t
        .import(
            &t.ada,
            Location::Under(deepest),
            "deep.zip",
            zipped(&[("a/b.md", "b")]),
        )
        .await;
    assert!(
        matches!(deep, Err(TransferError::Page(PageError::TooDeep))),
        "{deep:?}"
    );
    let after: i64 = t.h.scalar("SELECT count(*) FROM pages").await;
    assert_eq!(after, before);
    t.import(
        &t.ada,
        Location::Under(deepest),
        "a.md",
        Bytes::from_static(b"a"),
    )
    .await
    .expect("one level fits");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn imports_are_limited_by_rate_turn_place_and_time(pool: PgPool) {
    let t = team_with(
        pool,
        Limits {
            per_user: 1,
            ..Limits::default()
        },
    )
    .await;
    let general = Location::Teamspace(t.general);

    // A ticket holds one of its person's turns, while the upload comes, and nobody else's.
    let held = t
        .transfer
        .admit_import(&t.ada, t.workspace, general)
        .await
        .expect("let in");
    let busy = t.transfer.admit_import(&t.ada, t.workspace, general).await;
    assert!(matches!(busy, Err(TransferError::Busy)), "{:?}", busy.err());
    let export = t.transfer.export_teamspace(&t.ada, t.general).await;
    assert!(
        matches!(export, Err(TransferError::Busy)),
        "{:?}",
        export.err()
    );
    let other = t
        .transfer
        .admit_import(&t.dee, t.workspace, general)
        .await
        .expect("another person is let in");
    drop((held, other));
    t.transfer
        .admit_import(&t.ada, t.workspace, general)
        .await
        .expect("a turn came free");

    // The server's places are taken once the upload is in, not while it comes.
    let full = TransferService::builder(t.h.pages.clone(), t.sync.clone())
        .limits(Limits {
            parallel: 0,
            ..Limits::default()
        })
        .build();
    let ticket = full
        .admit_import(&t.ada, t.workspace, general)
        .await
        .expect("let in, though no place is free");
    let upload = Upload {
        name: "notes.md".to_owned(),
        bytes: Bytes::from_static(b"notes"),
    };
    let busy = full.import(ticket, upload).await;
    assert!(matches!(busy, Err(TransferError::Busy)), "{:?}", busy.err());
    let export = full.export_teamspace(&t.dee, t.general).await;
    assert!(
        matches!(export, Err(TransferError::Busy)),
        "{:?}",
        export.err()
    );

    // The rules answer before the limits: a place the caller may not see is not found.
    t.h.limiter.refuse(Bucket::PageImport);
    t.h.limiter.refuse(Bucket::PageExport);
    let hidden = t
        .transfer
        .admit_import(&t.dee, t.workspace, Location::Teamspace(t.secret))
        .await;
    assert_eq!(hidden.err().map(|error| error.code()), Some("not_found"));
    let hidden = t.transfer.export_teamspace(&t.dee, t.secret).await;
    assert_eq!(hidden.err().map(|error| error.code()), Some("not_found"));
    t.h.limiter.refuse(Bucket::PageImport);
    let limited = t.transfer.admit_import(&t.ada, t.workspace, general).await;
    assert!(
        matches!(limited, Err(TransferError::RateLimited { .. })),
        "{:?}",
        limited.err()
    );
    let key = (Bucket::PageImport, t.ada.user.id.to_string());
    assert!(t.h.limiter.seen().contains(&key));

    // Past its time, an import stops and makes nothing.
    let slow = TransferService::builder(t.h.pages.clone(), t.sync.clone())
        .limits(Limits {
            time_limit: Duration::ZERO,
            ..Limits::default()
        })
        .build();
    let ticket = slow
        .admit_import(&t.ada, t.workspace, general)
        .await
        .expect("let in");
    let late = slow
        .import(
            ticket,
            Upload {
                name: "late.zip".to_owned(),
                bytes: zipped(ROADMAP),
            },
        )
        .await;
    assert!(matches!(late, Err(TransferError::TimedOut(_))), "{late:?}");
    let pages: i64 = t.h.scalar("SELECT count(*) FROM pages").await;
    assert_eq!(pages, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn uploads_past_a_limit_make_nothing(pool: PgPool) {
    let t = team_with(
        pool,
        Limits {
            max_file: 64,
            ..Limits::default()
        },
    )
    .await;
    let large = "x".repeat(65);
    let refused = t
        .import(
            &t.ada,
            Location::Teamspace(t.general),
            "notes.zip",
            zipped(&[("small.md", "small"), ("large.md", &large)]),
        )
        .await
        .expect_err("too large");
    assert_eq!(refused.code(), "too_large");
    assert_eq!(
        refused.to_string(),
        "large.md unpacks to more than 64 B, the most a markdown file may be"
    );
    let unsafe_path = t
        .import(
            &t.ada,
            Location::Teamspace(t.general),
            "notes.zip",
            zipped(&[("ok.md", "ok"), ("../../etc/notes.md", "no")]),
        )
        .await
        .expect_err("refused");
    assert_eq!(unsafe_path.code(), "invalid_input");
    let pages: i64 = t.h.scalar("SELECT count(*) FROM pages").await;
    assert_eq!(pages, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn exports_hold_a_page_and_the_pages_under_it(pool: PgPool) {
    let t = team(pool).await;
    t.roadmap().await;
    let roadmap = t.id_of(t.general, "Roadmap").await;

    let alone = t.export(roadmap, false).await;
    assert_eq!(alone.format, Format::Markdown);
    assert_eq!(alone.name, "Roadmap.md");
    assert_eq!(
        String::from_utf8(alone.bytes).expect("text"),
        "# Roadmap\n\nWhere we are going.\n"
    );

    let zip = t.export(roadmap, true).await;
    assert_eq!(
        (zip.format, zip.name.as_str()),
        (Format::Zip, "Roadmap.zip")
    );
    let files = unzipped(&zip.bytes);
    assert_eq!(
        names(&files),
        [
            "Roadmap.md",
            "Roadmap/Q1 Goals.md",
            "Roadmap/Q1 Goals/Hiring.md"
        ]
    );
    assert_eq!(
        files["Roadmap/Q1 Goals.md"],
        "# Q1: Goals\n\n- [ ] ship\n- [x] plan\n"
    );
    assert_eq!(
        files["Roadmap/Q1 Goals/Hiring.md"],
        "# Hiring\n\nTwo people.\n"
    );

    // A page in the trash exports with the pages that went there with it, to its editors.
    let hiring = t.id_of(t.general, "Hiring").await;
    t.h.pages.trash_page(&t.ada, hiring).await.expect("trashed");
    assert_eq!(unzipped(&t.export(roadmap, true).await.bytes).len(), 2);
    t.h.pages
        .trash_page(&t.ada, roadmap)
        .await
        .expect("trashed");
    let trashed = unzipped(&t.export(roadmap, true).await.bytes);
    assert_eq!(names(&trashed), ["Roadmap.md", "Roadmap/Q1 Goals.md"]);
    let hidden = t.transfer.export_page(&t.gus, roadmap, true).await;
    assert!(
        matches!(hidden, Err(TransferError::Page(PageError::NotFound(_)))),
        "{:?}",
        hidden.err()
    );

    t.h.limiter.refuse(Bucket::PageExport);
    let limited = t.transfer.export_page(&t.ada, roadmap, false).await;
    assert!(matches!(limited, Err(TransferError::RateLimited { .. })));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn teamspaces_export_for_those_who_list_them(pool: PgPool) {
    let t = team(pool).await;
    t.roadmap().await;
    t.import(
        &t.ada,
        Location::Teamspace(t.secret),
        "secret.md",
        Bytes::from_static(b"# Hidden plans\n"),
    )
    .await
    .expect("imported");

    let general = t
        .transfer
        .export_teamspace(&t.dee, t.general)
        .await
        .expect("exported");
    assert_eq!(general.name, "General.zip");
    let files = unzipped(&general.bytes);
    assert_eq!(
        names(&files),
        [
            "Archive.md",
            "Archive/Old plan.md",
            "Roadmap.md",
            "Roadmap/Q1 Goals.md",
            "Roadmap/Q1 Goals/Hiring.md",
            "Welcome.md",
        ]
    );
    // A folder's page is its title alone.
    assert_eq!(files["Archive.md"], "# Archive\n");
    let secret = t
        .transfer
        .export_teamspace(&t.ada, t.secret)
        .await
        .expect("exported");
    assert_eq!(
        unzipped(&secret.bytes).into_iter().collect::<Vec<_>>(),
        [("Hidden plans.md".to_owned(), "# Hidden plans\n".to_owned())]
    );
    for (who, teamspace) in [(&t.dee, t.secret), (&t.gus, t.general)] {
        let hidden = t.transfer.export_teamspace(who, teamspace).await;
        assert!(
            matches!(hidden, Err(TransferError::Page(PageError::NotFound(_)))),
            "{:?}",
            hidden.err()
        );
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn exports_keep_to_their_limits(pool: PgPool) {
    let t = team_with(
        pool,
        Limits {
            max_export_pages: 5,
            ..Limits::default()
        },
    )
    .await;
    t.roadmap().await;
    let many = t.transfer.export_teamspace(&t.ada, t.general).await;
    assert_eq!(
        many.map(|_| ()).map_err(|error| error.code()),
        Err("too_large")
    );
    let roadmap = t.id_of(t.general, "Roadmap").await;
    t.transfer
        .export_page(&t.ada, roadmap, true)
        .await
        .expect("three pages fit");

    let small = TransferService::builder(t.h.pages.clone(), t.sync.clone())
        .limits(Limits {
            max_export_bytes: 40,
            ..Limits::default()
        })
        .build();
    let large = small.export_page(&t.ada, roadmap, true).await;
    assert_eq!(
        large.map(|_| ()).map_err(|error| error.code()),
        Err("too_large")
    );
}

/// Opens a page's live document as a client, syncs, then adds a paragraph to it. The
/// connection stays open, so the document stays live.
async fn write_live(t: &Team, page: Uuid, text: &str) -> Connection {
    edit_live(t, page, |txn, fragment| {
        neon_blocks::append(
            txn,
            fragment,
            &[Block::paragraph(text)],
            &mut SequentialIds::new("live"),
        )
        .expect("a document");
    })
    .await
}

/// Opens a page's live document as a client, syncs, then sends what `edit` changes of it.
async fn edit_live(
    t: &Team,
    page: Uuid,
    edit: impl FnOnce(&mut TransactionMut<'_>, &XmlFragmentRef),
) -> Connection {
    let admission = t.sync.admit(t.ada.clone(), page).await.expect("admitted");
    let mut connection = t.sync.connect(admission).await.expect("connected");
    let doc = Doc::new();
    let fragment = neon_blocks::fragment(&doc);
    let step1 = Message::Sync(SyncMessage::SyncStep1(doc.transact().state_vector()));
    connection
        .receive(Bytes::from(step1.encode_v1()))
        .await
        .expect("sent");
    loop {
        let Next::Send(data) = timeout(PATIENCE, connection.next())
            .await
            .expect("an answer")
        else {
            panic!("closed");
        };
        if let Message::Sync(SyncMessage::SyncStep2(update)) =
            Message::decode_v1(&data).expect("a message")
        {
            doc.transact_mut()
                .apply_update(Update::decode_v1(&update).expect("an update"))
                .expect("applies");
            break;
        }
    }
    let update = {
        let mut txn = doc.transact_mut();
        edit(&mut txn, &fragment);
        txn.encode_update_v1()
    };
    connection
        .receive(Bytes::from(
            Message::Sync(SyncMessage::Update(update)).encode_v1(),
        ))
        .await
        .expect("sent");
    connection
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn blocks_that_do_not_read_are_left_out_of_exports(pool: PgPool) {
    let t = team(pool).await;
    t.roadmap().await;
    let welcome = t.id_of(t.general, "Welcome").await;
    // A block of a type the editor does not know, as a client may write one.
    let open = edit_live(&t, welcome, |txn, fragment| {
        let group = fragment
            .get(txn, 0)
            .and_then(XmlOut::into_xml_element)
            .expect("a block group");
        let container = group.push_back(txn, XmlElementPrelim::empty("blockContainer"));
        container.push_back(txn, XmlElementPrelim::empty("callout"));
        neon_blocks::append(
            txn,
            fragment,
            &[Block::paragraph("After it.")],
            &mut SequentialIds::new("live"),
        )
        .expect("a document");
    })
    .await;

    let exported = String::from_utf8(t.export(welcome, false).await.bytes).expect("text");
    assert_eq!(
        exported,
        "# Welcome\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nAfter it.\n"
    );
    let zip = t
        .transfer
        .export_teamspace(&t.ada, t.general)
        .await
        .expect("the teamspace exports");
    assert!(
        unzipped(&zip.bytes)
            .values()
            .any(|markdown| markdown.ends_with("After it.\n"))
    );
    // Saved, it reads the same.
    drop(open);
    t.sync.shutdown().await;
    let saved = t.sync.content(welcome).await.expect("read");
    assert_eq!(saved.blocks.len(), 2);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn exports_read_what_is_live_and_round_trip(pool: PgPool) {
    let t = team(pool).await;
    t.roadmap().await;
    let welcome = t.id_of(t.general, "Welcome").await;

    // The document applies messages in turn, so the export's read comes after the update.
    let _open = write_live(&t, welcome, "Typed just now.").await;
    let exported = String::from_utf8(t.export(welcome, false).await.bytes).expect("text");
    assert_eq!(
        exported,
        "# Welcome\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nTyped just now.\n"
    );
    let logged: i64 = t.h.scalar("SELECT count(*) FROM page_updates").await;
    assert_eq!(
        logged, 0,
        "read from the live document, before it logged anything"
    );

    // What a teamspace exports imports back as the same pages with the same markdown.
    let first = t
        .transfer
        .export_teamspace(&t.ada, t.general)
        .await
        .expect("exported");
    let copy = teamspace(&t.h, &t.ada, t.workspace, "Copy", TeamspaceAccess::Open).await;
    let report = t
        .import(
            &t.ada,
            Location::Teamspace(copy),
            &first.name,
            Bytes::from(first.bytes.clone()),
        )
        .await
        .expect("imported");
    assert!(report.skipped.is_empty());
    assert_eq!(
        t.outline(&t.ada, copy).await,
        t.outline(&t.ada, t.general).await
    );
    let second = t
        .transfer
        .export_teamspace(&t.ada, copy)
        .await
        .expect("exported");
    assert_eq!(unzipped(&second.bytes), unzipped(&first.bytes));
}
