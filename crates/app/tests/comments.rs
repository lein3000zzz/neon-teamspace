//! Comments: threads on a page and on its text, replies, edits and deletions by their
//! authors, resolving, who may comment as the sharing model and the agent intersection
//! say, anchors that keep to their text through two clients' concurrent edits and come
//! apart once it is deleted, threads on a block's text, and what a mention may name.

mod support;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use neon_app::comments::{CommentError, CommentService, Placing, ThreadQuery};
use neon_app::pages::{Location, NewPage};
use neon_app::ratelimit::{Bucket, RateLimiter};
use neon_app::sync::{Connection, Limits, Next, SyncService, access_feed};
use neon_blocks::{Block, Inline, SequentialIds};
use neon_domain::auth::Principal;
use neon_domain::pages::{PageAccess, Placement};
use neon_domain::workspaces::WorkspaceRole;
use sqlx::PgPool;
use support::Harness;
use tokio::time::timeout;
use uuid::Uuid;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{
    Assoc, Doc, IndexedSequence as _, ReadTxn as _, Text as _, Transact as _, Update,
    XmlFragment as _, XmlFragmentRef, XmlOut, XmlTextRef,
};

const PATIENCE: Duration = Duration::from_secs(10);

/// Ada owns Design, with Bea a member and Gus a guest; Notes is Ada's page in General.
struct Team {
    h: Harness,
    ada: Principal,
    bea: Principal,
    gus: Principal,
    workspace: Uuid,
    page: Uuid,
    sync: SyncService,
    comments: CommentService,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let bea = h.person("bea@example.com").await;
    let gus = h.person("gus@example.com").await;
    let workspace = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, workspace, &bea, WorkspaceRole::Member)
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
    let page = h
        .pages
        .create_page(
            &ada,
            workspace,
            Location::Teamspace(general),
            Placement::Last,
            NewPage {
                title: "Notes".into(),
                ..NewPage::default()
            },
        )
        .await
        .expect("created")
        .page
        .id;
    let (_, feed) = access_feed();
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed)
        .limits(Limits {
            persist_delay: Duration::from_millis(5),
            idle_unload: Duration::from_secs(3600),
            ..Limits::default()
        })
        .limiter(Arc::clone(&h.limiter) as Arc<dyn RateLimiter>)
        .build();
    let comments = CommentService::new(h.pages.clone(), sync.clone());
    Team {
        h,
        ada,
        bea,
        gus,
        workspace,
        page,
        sync,
        comments,
    }
}

fn text_of(content: &[Inline]) -> String {
    Inline::plain_text(content)
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn threads_take_replies_and_are_resolved_and_reopened(pool: PgPool) {
    let t = team(pool).await;
    let started = t
        .comments
        .start_thread(&t.ada, t.page, "What **now**?", Placing::Page)
        .await
        .expect("started");
    let thread = started.thread.thread.id;
    assert!(started.thread.anchored.is_none(), "on the whole page");
    let replied = t
        .comments
        .reply(&t.bea, thread, "Ship it.")
        .await
        .expect("replied");
    assert_eq!(replied.thread.comments.len(), 2);

    let listed = t
        .comments
        .threads(&t.bea, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert!(listed.may_comment);
    assert_eq!(listed.threads.len(), 1);
    let comments = &listed.threads[0].comments;
    assert_eq!(comments[0].comment.body, "What **now**?");
    assert_eq!(text_of(&comments[0].content), "What now?");
    assert_eq!(
        comments[0].author.as_ref().map(|author| author.id),
        Some(t.ada.user.id)
    );
    assert_eq!(comments[1].comment.body, "Ship it.");

    let (resolved, _) = t
        .comments
        .set_resolved(&t.bea, thread, true)
        .await
        .expect("resolved");
    assert!(resolved.thread.resolved_at.is_some());
    assert_eq!(resolved.thread.resolved_by, Some(t.bea.user.id));
    let (reopened, _) = t
        .comments
        .set_resolved(&t.ada, thread, false)
        .await
        .expect("reopened");
    assert!(reopened.thread.resolved_at.is_none());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn its_author_changes_a_comment_and_it_or_an_editor_deletes_it(pool: PgPool) {
    let t = team(pool).await;
    // Gus, a guest, comments on Notes; Bea, a member, edits it.
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::Comment)
        .await
        .expect("shared");
    let started = t
        .comments
        .start_thread(&t.ada, t.page, "First", Placing::Page)
        .await
        .expect("started");
    let first = started.comment_id;
    let thread = started.thread.thread.id;
    let reply = t
        .comments
        .reply(&t.bea, thread, "Second")
        .await
        .expect("replied")
        .comment_id;

    for who in [&t.bea, &t.gus] {
        let refused = t.comments.edit(who, first, "Mine now").await;
        assert!(
            matches!(refused, Err(CommentError::Forbidden(_))),
            "{refused:?}"
        );
    }
    let refused = t.comments.delete(&t.gus, first).await;
    match refused {
        Err(CommentError::Forbidden(message)) => assert_eq!(
            message,
            "only its author, or those who edit the page, delete a comment"
        ),
        other => panic!("{other:?}"),
    }

    let edited = t
        .comments
        .edit(&t.ada, first, "First, edited")
        .await
        .expect("edited");
    let comment = &edited.thread.comments[0].comment;
    assert_eq!(comment.body, "First, edited");
    assert!(comment.edited_at.is_some(), "it shows as edited");
    assert!(edited.thread.comments[1].comment.edited_at.is_none());

    // Bea, who edits the page, deletes Ada's first comment, which leaves the reply;
    // deleting the last comment deletes the thread.
    assert!(t.comments.delete(&t.bea, first).await.expect("deleted"));
    let listed = t
        .comments
        .threads(&t.ada, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert_eq!(listed.threads[0].comments.len(), 1);
    assert_eq!(listed.threads[0].count, 1);
    assert!(!t.comments.delete(&t.bea, reply).await.expect("deleted"));
    let listed = t
        .comments
        .threads(&t.ada, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert!(listed.threads.is_empty());
    let gone = t.comments.edit(&t.ada, first, "again").await;
    assert!(matches!(gone, Err(CommentError::NotFound(_))), "{gone:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_editor_deletes_a_whole_thread_and_a_commenter_does_not(pool: PgPool) {
    let t = team(pool).await;
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::Comment)
        .await
        .expect("shared");
    let thread = t
        .comments
        .start_thread(&t.gus, t.page, "Mine", Placing::Page)
        .await
        .expect("started")
        .thread
        .thread
        .id;
    t.comments
        .reply(&t.ada, thread, "A reply")
        .await
        .expect("replied");
    let refused = t.comments.delete_thread(&t.gus, thread).await;
    assert!(
        matches!(refused, Err(CommentError::Forbidden(_))),
        "{refused:?}"
    );
    t.comments
        .delete_thread(&t.bea, thread)
        .await
        .expect("deleted");
    let gone = t.comments.thread(&t.ada, thread).await;
    assert!(matches!(gone, Err(CommentError::NotFound(_))), "{gone:?}");
    // Nobody hears that it was there.
    let hidden = t.comments.delete_thread(&t.bea, thread).await;
    assert!(
        matches!(hidden, Err(CommentError::NotFound(_))),
        "{hidden:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn who_may_comment_follows_access_and_shares(pool: PgPool) {
    let t = team(pool).await;
    let comment = |who: Principal| {
        let comments = t.comments.clone();
        let page = t.page;
        async move {
            comments
                .start_thread(&who, page, "A thought", Placing::Page)
                .await
                .map(|_| ())
        }
    };
    // A guest sees nothing unshared: the page is not found.
    let hidden = comment(t.gus.clone()).await;
    assert!(
        matches!(hidden, Err(CommentError::NotFound(_))),
        "{hidden:?}"
    );
    assert!(matches!(
        t.comments
            .threads(&t.gus, t.page, ThreadQuery::default())
            .await,
        Err(CommentError::NotFound(_))
    ));

    // Shared to view, they read the comments and may not write one.
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let listed = t
        .comments
        .threads(&t.gus, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert!(!listed.may_comment);
    assert!(listed.members_hidden);
    let refused = comment(t.gus.clone()).await;
    assert!(
        matches!(refused, Err(CommentError::Forbidden(_))),
        "{refused:?}"
    );

    // Shared to comment, they comment, and still may not edit the page.
    t.h.pages
        .change_share(&t.ada, t.page, t.gus.user.id, PageAccess::Comment)
        .await
        .expect("changed");
    comment(t.gus.clone()).await.expect("commented");
    assert!(
        t.comments
            .threads(&t.gus, t.page, ThreadQuery::default())
            .await
            .expect("listed")
            .may_comment
    );
    let view = t.h.pages.page(&t.gus, t.page).await.expect("seen");
    assert_eq!(view.access, PageAccess::Comment);

    // Members of the teamspace edit, so they comment.
    comment(t.bea.clone()).await.expect("commented");

    // In the trash, nobody comments.
    t.h.pages.trash_page(&t.ada, t.page).await.expect("trashed");
    let trashed = comment(t.ada.clone()).await;
    assert!(
        matches!(trashed, Err(CommentError::Forbidden(_))),
        "{trashed:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_agent_comments_only_as_far_as_its_owner_may(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.ada, t.workspace, "Scout").await;
    let (_, agent) = t.h.agent_token(&t.ada, &scout, &["read", "write"]).await;
    // The agent is in General, as every member is, and so is Ada: it comments.
    t.comments
        .start_thread(&agent, t.page, "Reviewed.", Placing::Page)
        .await
        .expect("commented");

    // On a page of Bea's private pages shared with the agent, Ada sees nothing: hidden.
    let private =
        t.h.pages
            .create_page(
                &t.bea,
                t.workspace,
                Location::Private,
                Placement::Last,
                NewPage::default(),
            )
            .await
            .expect("created")
            .page
            .id;
    t.h.pages
        .share_page(&t.bea, private, scout.id, PageAccess::Comment)
        .await
        .expect("shared");
    let hidden = t
        .comments
        .start_thread(&agent, private, "Hello", Placing::Page)
        .await;
    assert!(
        matches!(hidden, Err(CommentError::NotFound(_))),
        "{hidden:?}"
    );

    // Shared with Ada to view only, the agent may still not comment there.
    t.h.pages
        .share_page(&t.bea, private, t.ada.user.id, PageAccess::View)
        .await
        .expect("shared");
    let refused = t
        .comments
        .start_thread(&agent, private, "Hello", Placing::Page)
        .await;
    assert!(
        matches!(refused, Err(CommentError::Forbidden(_))),
        "{refused:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn comments_are_bounded_inline_markdown(pool: PgPool) {
    let t = team(pool).await;
    let posted = t
        .comments
        .start_thread(
            &t.ada,
            t.page,
            "# not a heading\n- nor a list, but @[Bea](user:0193a5b2-0000-7000-8000-0000000000a2)",
            Placing::Page,
        )
        .await
        .expect("posted");
    let content = &posted.thread.comments[0].content;
    assert_eq!(text_of(content), "# not a heading\n- nor a list, but @Bea");
    assert!(content.iter().any(|run| matches!(run, Inline::Mention(_))));

    for (body, why) in [
        ("   ", "a comment holds some text"),
        (
            &"x".repeat(10_001)[..],
            "a comment holds at most 10000 characters",
        ),
        // The database keeps no NUL: it is refused as the caller's, not failed as the
        // server's.
        (
            "a\u{0}b",
            "a comment holds no control characters but line breaks and tabs",
        ),
    ] {
        let refused = t
            .comments
            .start_thread(&t.ada, t.page, body, Placing::Page)
            .await;
        match refused {
            Err(CommentError::InvalidInput(message)) => assert_eq!(message, why),
            other => panic!("{other:?}"),
        }
    }
    let bad = t
        .comments
        .start_thread(
            &t.ada,
            t.page,
            "x",
            Placing::Text {
                start: Vec::new(),
                end: vec![1],
                quote: "x".into(),
            },
        )
        .await;
    assert!(matches!(bad, Err(CommentError::InvalidInput(_))), "{bad:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_thread_holds_at_most_its_replies(pool: PgPool) {
    let t = team(pool).await;
    let thread = t
        .comments
        .start_thread(&t.ada, t.page, "First", Placing::Page)
        .await
        .expect("started")
        .thread
        .thread
        .id;
    sqlx::query(
        "INSERT INTO comments (id, thread_id, author_id, body, search_text, created_at)
         SELECT gen_random_uuid(), $1, $2, 'more', 'more', now() FROM generate_series(1, 499)",
    )
    .bind(thread)
    .bind(t.ada.user.id)
    .execute(&t.h.pool)
    .await
    .expect("filled");
    let refused = t.comments.reply(&t.bea, thread, "One too many").await;
    assert!(
        matches!(refused, Err(CommentError::TooManyReplies)),
        "{refused:?}"
    );
}

impl Team {
    /// Adds `threads` threads to the page, as the database holds them, resolved or not, and
    /// says their ids.
    async fn fill_threads(&self, threads: i64, resolved: bool) -> Vec<Uuid> {
        sqlx::query_scalar(
            "INSERT INTO comment_threads (id, workspace_id, page_id, resolved_at, created_by,
                                          created_at)
             SELECT gen_random_uuid(), $1, $2, CASE WHEN $4 THEN now() END, $3, now()
             FROM generate_series(1, $5)
             RETURNING id",
        )
        .bind(self.workspace)
        .bind(self.page)
        .bind(self.ada.user.id)
        .bind(resolved)
        .bind(threads)
        .fetch_all(&self.h.pool)
        .await
        .expect("filled")
    }

    /// Adds `comments` comments by Ada to a thread, as the database holds them.
    async fn fill_comments(&self, thread: Uuid, comments: i64) {
        sqlx::query(
            "INSERT INTO comments (id, thread_id, author_id, body, search_text, created_at)
             SELECT gen_random_uuid(), $1, $2, 'more ' || n, 'more ' || n, now()
             FROM generate_series(1, $3) AS n",
        )
        .bind(thread)
        .bind(self.ada.user.id)
        .bind(comments)
        .execute(&self.h.pool)
        .await
        .expect("filled");
    }
}

/// A client of a page's live document, talking to its connection directly.
struct Client {
    connection: Connection,
    doc: Doc,
    fragment: XmlFragmentRef,
}

impl Client {
    async fn open(sync: &SyncService, who: &Principal, page: Uuid, client_id: u64) -> Self {
        let admission = sync.admit(who.clone(), page).await.expect("admitted");
        let connection = sync.connect(admission).await.expect("connected");
        let doc = Doc::with_client_id(client_id);
        let fragment = neon_blocks::fragment(&doc);
        let mut client = Self {
            connection,
            doc,
            fragment,
        };
        let state_vector = client.doc.transact().state_vector();
        client
            .send(Message::Sync(SyncMessage::SyncStep1(state_vector)))
            .await;
        loop {
            if matches!(
                client.next().await,
                Message::Sync(SyncMessage::SyncStep2(_))
            ) {
                return client;
            }
        }
    }

    async fn send(&mut self, message: Message) {
        self.connection
            .receive(Bytes::from(message.encode_v1()))
            .await
            .expect("sent");
    }

    async fn next(&mut self) -> Message {
        let Next::Send(data) = timeout(PATIENCE, self.connection.next())
            .await
            .expect("something comes")
        else {
            panic!("the connection closed");
        };
        let message = Message::decode_v1(&data).expect("a y-sync message");
        if let Message::Sync(SyncMessage::SyncStep2(update) | SyncMessage::Update(update)) =
            &message
        {
            self.doc
                .transact_mut()
                .apply_update(Update::decode_v1(update).expect("an update"))
                .expect("applies");
        }
        message
    }

    /// Applies what the server sent until the client's document has `want`, the other
    /// client's change.
    async fn until_text(&mut self, want: &str) {
        while !self.text().contains(want) {
            self.next().await;
        }
    }

    fn text(&self) -> String {
        neon_blocks::read_doc(&self.doc)
            .expect("blocks")
            .plain_text()
    }

    /// Makes a change in the client's document and sends it.
    async fn change(&mut self, change: impl FnOnce(&mut yrs::TransactionMut<'_>, &XmlTextRef)) {
        let update = {
            let text = self.first_text();
            let mut txn = self.doc.transact_mut();
            change(&mut txn, &text);
            txn.encode_update_v1()
        };
        self.send(Message::Sync(SyncMessage::Update(update))).await;
    }

    fn first_text(&self) -> XmlTextRef {
        let txn = self.doc.transact();
        let Some(XmlOut::Element(group)) = self.fragment.first_child() else {
            panic!("a block group")
        };
        let Some(XmlOut::Element(container)) = group.get(&txn, 0) else {
            panic!("a block")
        };
        let Some(XmlOut::Element(content)) = container.first_child() else {
            panic!("its content")
        };
        content
            .children(&txn)
            .find_map(|child| match child {
                XmlOut::Text(text) => Some(text),
                _ => None,
            })
            .expect("a text")
    }

    /// An anchor over `range` of the first block's text, as the editor makes one.
    fn anchor(&self, range: std::ops::Range<u32>, quote: &str) -> Placing {
        let text = self.first_text();
        let txn = self.doc.transact();
        Placing::Text {
            start: text
                .sticky_index(&txn, range.start, Assoc::After)
                .expect("a start")
                .encode_v1(),
            end: text
                .sticky_index(&txn, range.end, Assoc::After)
                .expect("an end")
                .encode_v1(),
            quote: quote.to_owned(),
        }
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_inline_thread_keeps_to_its_text_through_two_clients_edits(pool: PgPool) {
    let t = team(pool).await;
    let mut ada = Client::open(&t.sync, &t.ada, t.page, 11).await;
    let mut bea = Client::open(&t.sync, &t.bea, t.page, 12).await;
    let update = {
        let mut txn = ada.doc.transact_mut();
        neon_blocks::append(
            &mut txn,
            &ada.fragment,
            &[
                Block::paragraph("The quick brown fox jumps."),
                Block::paragraph("Another line."),
            ],
            &mut SequentialIds::new("block"),
            neon_blocks::Deadline::NONE,
        )
        .expect("a document");
        txn.encode_update_v1()
    };
    ada.send(Message::Sync(SyncMessage::Update(update))).await;
    bea.until_text("Another line.").await;

    let thread = t
        .comments
        .start_thread(
            &t.ada,
            t.page,
            "Which fox?",
            ada.anchor(10..19, "brown fox"),
        )
        .await
        .expect("started")
        .thread;
    let anchored = thread.anchored.expect("resolved");
    assert_eq!(anchored.block_id.as_deref(), Some("block-1"));
    assert!(!anchored.detached);
    assert_eq!(
        thread
            .thread
            .anchor
            .as_ref()
            .map(|anchor| anchor.quote.as_str()),
        Some("brown fox")
    );

    // At once: Ada types before the range, Bea inside it and after it.
    ada.change(|txn, text| text.insert(txn, 0, "Look: ")).await;
    bea.change(|txn, text| {
        text.insert(txn, 16, "red ");
        text.insert(txn, 24, " high");
    })
    .await;
    ada.until_text("high").await;
    bea.until_text("Look: ").await;
    assert_eq!(ada.text(), bea.text());

    let listed = t
        .comments
        .threads(&t.bea, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    let anchored = listed.threads[0].anchored.clone().expect("resolved");
    assert_eq!(anchored.block_id.as_deref(), Some("block-1"));
    assert!(
        !anchored.detached,
        "its text is still there: {}",
        ada.text()
    );

    // Deleting all of its text, the thread stays, detached.
    let range = {
        let content = ada.text();
        let start = content.find("brown").expect("there");
        let end = content.find(" fox").expect("there") + " fox".len();
        (
            u32::try_from(start).expect("small"),
            u32::try_from(end - start).expect("small"),
        )
    };
    bea.until_text("Look: ").await;
    ada.change(|txn, text| text.remove_range(txn, range.0, range.1))
        .await;
    bea.until_text("Look: The quick  ").await;
    let listed = t
        .comments
        .threads(&t.ada, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert_eq!(listed.threads.len(), 1, "the thread stays");
    let anchored = listed.threads[0].anchored.clone().expect("resolved");
    assert!(anchored.detached);
    assert_eq!(anchored.block_id, None);
    assert_eq!(
        listed.threads[0]
            .thread
            .anchor
            .as_ref()
            .map(|anchor| anchor.quote.as_str()),
        Some("brown fox"),
        "it keeps the text it was made on"
    );

    // Once saved and loaded afresh, it reads the same.
    t.sync.shutdown().await;
    let (_, feed) = access_feed();
    let reloaded =
        SyncService::builder(t.h.pool.clone(), t.h.pages.clone(), t.h.auth.clone(), feed).build();
    let comments = CommentService::new(t.h.pages.clone(), reloaded);
    let listed = comments
        .threads(&t.ada, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert!(
        listed.threads[0]
            .anchored
            .as_ref()
            .is_some_and(|at| at.detached)
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_thread_on_a_block_covers_its_text(pool: PgPool) {
    let t = team(pool).await;
    let mut ada = Client::open(&t.sync, &t.ada, t.page, 11).await;
    let update = {
        let mut txn = ada.doc.transact_mut();
        neon_blocks::append(
            &mut txn,
            &ada.fragment,
            &[Block::paragraph("Deploy on Friday")],
            &mut SequentialIds::new("block"),
            neon_blocks::Deadline::NONE,
        )
        .expect("a document");
        txn.encode_update_v1()
    };
    ada.send(Message::Sync(SyncMessage::Update(update))).await;
    let posted = t
        .comments
        .start_thread(
            &t.bea,
            t.page,
            "Why Friday?",
            Placing::Block("block-1".into()),
        )
        .await
        .expect("started");
    let anchor = posted.thread.thread.anchor.expect("anchored");
    assert_eq!(anchor.quote, "Deploy on Friday");
    assert_eq!(
        posted.thread.anchored.and_then(|at| at.block_id).as_deref(),
        Some("block-1")
    );
    let missing = t
        .comments
        .start_thread(&t.bea, t.page, "Here?", Placing::Block("block-9".into()))
        .await;
    assert!(
        matches!(missing, Err(CommentError::InvalidInput(_))),
        "{missing:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mentions_find_members_and_pages_with_who_may_see_the_page(pool: PgPool) {
    let t = team(pool).await;
    let private =
        t.h.pages
            .create_page(
                &t.ada,
                t.workspace,
                Location::Private,
                Placement::Last,
                NewPage {
                    title: "Notes to self".into(),
                    ..NewPage::default()
                },
            )
            .await
            .expect("created")
            .page
            .id;
    let found = t
        .comments
        .candidates(&t.ada, private, "")
        .await
        .expect("found");
    let people: Vec<(&str, bool)> = found
        .people
        .iter()
        .map(|candidate| (candidate.person.display_name.as_str(), candidate.sees_page))
        .collect();
    assert_eq!(people, [("ada", true), ("bea", false), ("gus", false)]);
    let titles: Vec<&str> = found
        .pages
        .iter()
        .map(|view| view.page.title.as_str())
        .collect();
    assert!(titles.contains(&"Notes") && titles.contains(&"Notes to self"));

    let bea = t
        .comments
        .candidates(&t.bea, t.page, "BE")
        .await
        .expect("found");
    assert_eq!(bea.people.len(), 1);
    assert!(bea.people[0].sees_page);
    let pages = t
        .comments
        .candidates(&t.bea, t.page, "notes")
        .await
        .expect("found");
    let titles: Vec<&str> = pages
        .pages
        .iter()
        .map(|view| view.page.title.as_str())
        .collect();
    assert_eq!(titles, ["Notes"], "Ada's private page is not Bea's to find");

    // A guest finds pages alone.
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    let guest = t
        .comments
        .candidates(&t.gus, t.page, "")
        .await
        .expect("found");
    assert!(guest.people.is_empty());
    assert_eq!(guest.pages.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn threads_and_their_comments_are_read_a_part_at_a_time(pool: PgPool) {
    let t = team(pool).await;
    let mut started = Vec::new();
    for n in 0..3 {
        let posted = t
            .comments
            .start_thread(&t.ada, t.page, &format!("Thread {n}"), Placing::Page)
            .await
            .expect("started");
        started.push(posted.thread.thread.id);
    }
    let long = started[0];
    for n in 0..29 {
        t.comments
            .reply(&t.bea, long, &format!("Reply {n}"))
            .await
            .expect("replied");
    }
    t.comments
        .set_resolved(&t.ada, started[2], true)
        .await
        .expect("resolved");

    // Threads oldest first, two to a part.
    let first = t
        .comments
        .threads(
            &t.ada,
            t.page,
            ThreadQuery {
                limit: Some(2),
                ..ThreadQuery::default()
            },
        )
        .await
        .expect("listed");
    let ids: Vec<Uuid> = first.threads.iter().map(|view| view.thread.id).collect();
    assert_eq!(ids, started[..2]);
    assert_eq!(first.next, Some(started[1]));
    assert_eq!(first.open, 2);
    let rest = t
        .comments
        .threads(
            &t.ada,
            t.page,
            ThreadQuery {
                after: first.next,
                limit: Some(2),
                ..ThreadQuery::default()
            },
        )
        .await
        .expect("listed");
    assert_eq!(rest.threads.len(), 1);
    assert_eq!(rest.next, None);
    let open = t
        .comments
        .threads(
            &t.ada,
            t.page,
            ThreadQuery {
                resolved: Some(false),
                ..ThreadQuery::default()
            },
        )
        .await
        .expect("listed");
    assert_eq!(open.threads.len(), 2);

    // A long thread shows its first comment and its latest nine, and counts them all.
    let view = &first.threads[0];
    assert_eq!(view.count, 30);
    let bodies: Vec<&str> = view
        .comments
        .iter()
        .map(|comment| comment.comment.body.as_str())
        .collect();
    let mut want = vec!["Thread 0".to_owned()];
    want.extend((20..29).map(|n| format!("Reply {n}")));
    assert_eq!(bodies, want);
    // A reply's answer holds its own thread alone, as bounded.
    let posted = t
        .comments
        .reply(&t.bea, started[1], "Short")
        .await
        .expect("replied");
    assert_eq!(posted.thread.count, 2);
    assert!(
        posted
            .thread
            .comments
            .iter()
            .all(|comment| comment.comment.thread_id == started[1])
    );

    // Every comment of the long one, a part at a time.
    let mut read = Vec::new();
    let mut after = None;
    loop {
        let part = t
            .comments
            .comments(&t.bea, long, after, Some(12))
            .await
            .expect("read");
        read.extend(part.comments.into_iter().map(|view| view.comment.body));
        after = part.next;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(read.len(), 30);
    assert_eq!(read[1], "Reply 0");
    // Hidden from whoever may not see the page.
    let hidden = t.comments.comments(&t.gus, long, None, None).await;
    assert!(
        matches!(hidden, Err(CommentError::NotFound(_))),
        "{hidden:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn only_open_threads_count_toward_a_pages_limit(pool: PgPool) {
    let t = team(pool).await;
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::Comment)
        .await
        .expect("shared");
    t.fill_threads(5, true).await;
    t.fill_threads(999, false).await;
    // Gus takes the last place.
    let last = t
        .comments
        .start_thread(&t.gus, t.page, "The last one", Placing::Page)
        .await
        .expect("started")
        .thread
        .thread
        .id;
    let full = t
        .comments
        .start_thread(&t.ada, t.page, "One more", Placing::Page)
        .await;
    assert!(
        matches!(full, Err(CommentError::TooManyThreads)),
        "{full:?}"
    );
    // Resolving one makes room, and reopening it waits for room again.
    t.comments
        .set_resolved(&t.ada, last, true)
        .await
        .expect("resolved");
    let made = t
        .comments
        .start_thread(&t.ada, t.page, "One more", Placing::Page)
        .await
        .expect("started")
        .thread
        .thread
        .id;
    let reopened = t.comments.set_resolved(&t.ada, last, false).await;
    assert!(
        matches!(reopened, Err(CommentError::TooManyThreads)),
        "{reopened:?}"
    );
    // So does deleting one, which those who edit the page may do to anyone's.
    t.comments
        .delete_thread(&t.ada, made)
        .await
        .expect("deleted");
    t.comments
        .set_resolved(&t.ada, last, false)
        .await
        .expect("reopened");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn threads_started_at_once_keep_to_the_limit(pool: PgPool) {
    let t = team(pool).await;
    // One place left, which several take at once.
    t.fill_threads(999, false).await;
    let mut started = tokio::task::JoinSet::new();
    for n in 0..8 {
        let (comments, ada, page) = (t.comments.clone(), t.ada.clone(), t.page);
        started.spawn(async move {
            comments
                .start_thread(&ada, page, &format!("At once {n}"), Placing::Page)
                .await
        });
    }
    let (mut made, mut refused) = (0, 0);
    while let Some(outcome) = started.join_next().await {
        match outcome.expect("ran") {
            Ok(_) => made += 1,
            Err(CommentError::TooManyThreads) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((made, refused), (1, 7));
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM comment_threads WHERE page_id = $1 AND resolved_at IS NULL",
    )
    .bind(t.page)
    .fetch_one(&t.h.pool)
    .await
    .expect("counted");
    assert_eq!(open, 1000);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_holds_at_most_its_comments_until_an_editor_deletes_some(pool: PgPool) {
    let t = team(pool).await;
    let thread = t
        .comments
        .start_thread(&t.ada, t.page, "First", Placing::Page)
        .await
        .expect("started")
        .thread
        .thread
        .id;
    let other = t.fill_threads(1, true).await[0];
    t.fill_comments(other, 4999).await;
    let refused = t.comments.reply(&t.bea, thread, "One too many").await;
    assert!(
        matches!(refused, Err(CommentError::TooManyComments)),
        "{refused:?}"
    );
    let refused = t
        .comments
        .start_thread(&t.bea, t.page, "Nor here", Placing::Page)
        .await;
    assert!(
        matches!(refused, Err(CommentError::TooManyComments)),
        "{refused:?}"
    );
    t.comments
        .delete_thread(&t.bea, other)
        .await
        .expect("deleted");
    t.comments
        .reply(&t.bea, thread, "Room again")
        .await
        .expect("replied");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn comments_past_their_rate_are_refused(pool: PgPool) {
    let t = team(pool).await;
    let thread = t
        .comments
        .start_thread(&t.ada, t.page, "First", Placing::Page)
        .await
        .expect("started");
    let comment = thread.comment_id;
    let thread = thread.thread.thread.id;
    t.h.limiter.refuse(Bucket::Comment);
    for refused in [
        t.comments
            .start_thread(&t.bea, t.page, "Again", Placing::Page)
            .await,
        t.comments.reply(&t.bea, thread, "Again").await,
        t.comments.edit(&t.ada, comment, "Again").await,
    ] {
        assert!(
            matches!(refused, Err(CommentError::RateLimited { .. })),
            "{refused:?}"
        );
    }
    let counted: Vec<String> =
        t.h.limiter
            .seen()
            .into_iter()
            .filter(|(bucket, _)| *bucket == Bucket::Comment)
            .map(|(_, key)| key)
            .collect();
    assert_eq!(
        counted,
        [
            t.ada.user.id.to_string(),
            t.bea.user.id.to_string(),
            t.bea.user.id.to_string(),
            t.ada.user.id.to_string()
        ],
        "each write is counted against its writer"
    );
    // Reading, resolving and deleting are not writing comments.
    t.comments
        .set_resolved(&t.bea, thread, true)
        .await
        .expect("resolved");
    t.comments.delete(&t.ada, comment).await.expect("deleted");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_search_finds_pages_the_caller_sees_however_many_hidden_ones_match(pool: PgPool) {
    let t = team(pool).await;
    let general =
        t.h.workspaces
            .list_teamspaces(&t.ada, t.workspace)
            .await
            .expect("listed")
            .teamspaces[0]
            .teamspace
            .id;
    let make = |who: Principal, title: String, location: Location| {
        let pages = t.h.pages.clone();
        let workspace = t.workspace;
        async move {
            pages
                .create_page(
                    &who,
                    workspace,
                    location,
                    Placement::Last,
                    NewPage {
                        title,
                        ..NewPage::default()
                    },
                )
                .await
                .expect("created")
                .page
                .id
        }
    };
    make(
        t.ada.clone(),
        "Layoffs FAQ (public)".into(),
        Location::Teamspace(general),
    )
    .await;
    // More of Ada's private pages match, and are newer, than a search answers with.
    for n in 0..60 {
        make(
            t.ada.clone(),
            format!("Layoffs plan {n}"),
            Location::Private,
        )
        .await;
    }
    let found = t
        .comments
        .candidates(&t.bea, t.page, "layoffs")
        .await
        .expect("found");
    let titles: Vec<&str> = found
        .pages
        .iter()
        .map(|view| view.page.title.as_str())
        .collect();
    assert_eq!(titles, ["Layoffs FAQ (public)"]);
    // Ada finds her own, at most ten.
    let own = t
        .comments
        .candidates(&t.ada, t.page, "layoffs")
        .await
        .expect("found");
    assert_eq!(own.pages.len(), 10);

    // A guest searches what is shared with them, and the pages under it, alone.
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    make(
        t.ada.clone(),
        "Notes, part two".into(),
        Location::Under(t.page),
    )
    .await;
    let guest = t
        .comments
        .candidates(&t.gus, t.page, "")
        .await
        .expect("found");
    let mut titles: Vec<&str> = guest
        .pages
        .iter()
        .map(|view| view.page.title.as_str())
        .collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Notes", "Notes, part two"]);
    let none = t
        .comments
        .candidates(&t.gus, t.page, "layoffs")
        .await
        .expect("found");
    assert!(none.pages.is_empty());
}
