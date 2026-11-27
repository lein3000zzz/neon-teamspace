//! The inbox: every rule that makes a notification, each person once per comment, nobody
//! told of what they may not see and each notification asked again as it is shown, mentions
//! in pages' text once they settle, assignments and invitations, reading and archiving, the
//! live room, notification mail and digests with their links that stop them, and the
//! webhook events of mentions and replies.

mod support;

use std::time::Duration;

use neon_app::comments::{CommentService, Placing, ThreadQuery};
use neon_app::jobs::{Handler as _, Job};
use neon_app::mail::Message;
use neon_app::notifications::{InboxError, InboxQuery, InboxService, NotificationMailJob};
use neon_app::pages::{Location, NewPage};
use neon_app::ratelimit::RateLimiter;
use neon_app::sync::{Broadcaster as _, Limits, Next, Outbox, Room, SyncService, access_feed};
use neon_app::webhooks::transport::{Answer, Outgoing, SendError, WebhookTransport};
use neon_app::webhooks::{NewWebhook, Services, WebhookService};
use neon_blocks::markdown::from_markdown;
use neon_domain::auth::Principal;
use neon_domain::notifications::{MailPreference, NotificationKind};
use neon_domain::pages::{PageAccess, Placement};
use neon_domain::workspaces::WorkspaceRole;
use serde_json::{Value, json};
use sqlx::{PgPool, Row as _};
use support::{Harness, Options, envelope};
use tokio::time::{Instant, timeout};
use url::Url;
use uuid::Uuid;

/// Takes every delivery and answers it as delivered.
struct Accepting;

#[async_trait::async_trait]
impl WebhookTransport for Accepting {
    async fn check(&self, _: &Url, _: bool) -> Result<(), SendError> {
        Ok(())
    }

    async fn send(&self, _: Outgoing, _: bool) -> Result<Answer, SendError> {
        Ok(Answer {
            status: 204,
            excerpt: None,
        })
    }
}

const PATIENCE: Duration = Duration::from_secs(10);

/// Ada owns Design, with Bea and Cy members and Gus a guest; Notes is Ada's page in
/// General, and Secret a page of Ada's private pages.
struct Team {
    h: Harness,
    ada: Principal,
    bea: Principal,
    cy: Principal,
    gus: Principal,
    workspace: Uuid,
    general: Uuid,
    page: Uuid,
    secret: Uuid,
    sync: SyncService,
    comments: CommentService,
    inbox: InboxService,
}

async fn team(pool: PgPool) -> Team {
    team_with(pool, Vec::new()).await
}

async fn team_with(pool: PgPool, settings: Vec<(&'static str, Value)>) -> Team {
    let h = Harness::with(
        pool,
        Options {
            settings,
            ..Options::default()
        },
    )
    .await;
    let ada = h.person("ada@example.com").await;
    let bea = h.person("bea@example.com").await;
    let cy = h.person("cy@example.com").await;
    let gus = h.person("gus@example.com").await;
    let workspace = h.workspace(&ada, "Design").await.workspace.id;
    for (who, role) in [
        (&bea, WorkspaceRole::Member),
        (&cy, WorkspaceRole::Member),
        (&gus, WorkspaceRole::Guest),
    ] {
        h.bring_in(&ada, workspace, who, role).await;
    }
    let general = h
        .workspaces
        .list_teamspaces(&ada, workspace)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let page = |title: &str, location| {
        let pages = h.pages.clone();
        let ada = ada.clone();
        let title = title.to_owned();
        async move {
            pages
                .create_page(
                    &ada,
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
    let notes = page("Notes", Location::Teamspace(general)).await;
    let secret = page("Secret", Location::Private).await;
    let (_, feed) = access_feed();
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed)
        .limits(Limits {
            persist_delay: Duration::from_millis(5),
            idle_unload: Duration::from_secs(3600),
            mention_settle: Duration::from_millis(50),
            ..Limits::default()
        })
        .limiter(std::sync::Arc::clone(&h.limiter) as std::sync::Arc<dyn RateLimiter>)
        .build();
    let comments = CommentService::new(h.pages.clone(), sync.clone());
    let inbox = InboxService::new(h.pages.clone(), h.mail.clone(), envelope());
    // The jobs bringing everyone in queued, the mail of their invitations among them, and
    // the mail of their notifications that waits.
    for emptied in ["DELETE FROM jobs", "DELETE FROM notification_mail"] {
        sqlx::query(emptied)
            .execute(&h.pool)
            .await
            .expect("emptied");
    }
    Team {
        h,
        ada,
        bea,
        cy,
        gus,
        workspace,
        general,
        page: notes,
        secret,
        sync,
        comments,
        inbox,
    }
}

/// A mention of someone, as comments and pages write it.
fn mention(who: &Principal) -> String {
    format!("@[{}](user:{})", who.user.display_name, who.user.id)
}

impl Team {
    /// Someone's notifications as their inbox shows them, newest first: each kind, and the
    /// comment or block it is about.
    async fn shown(&self, who: &Principal) -> Vec<(NotificationKind, Option<Uuid>)> {
        self.inbox
            .inbox(who, InboxQuery::default())
            .await
            .expect("read")
            .notifications
            .into_iter()
            .map(|shown| (shown.notification.kind, shown.notification.comment_id))
            .collect()
    }

    async fn kinds(&self, who: &Principal) -> Vec<NotificationKind> {
        self.shown(who)
            .await
            .into_iter()
            .map(|(kind, _)| kind)
            .collect()
    }

    async fn unread(&self, who: &Principal) -> usize {
        self.inbox.unread_count(who, None).await.expect("counted")
    }

    /// Every notification made for someone, shown or not, but of the invitations that
    /// brought them in.
    async fn made(&self, who: &Principal) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM notifications WHERE user_id = $1 AND kind <> 'invitation'",
        )
        .bind(who.user.id)
        .fetch_one(&self.h.pool)
        .await
        .expect("counted")
    }

    /// The notifications of mentions in pages' text made for someone, oldest first: the
    /// page, who it is from and the block it is in.
    async fn text_mentions(&self, who: &Principal) -> Vec<(Uuid, Option<Uuid>, Option<String>)> {
        sqlx::query_as(
            "SELECT page_id, actor_id, block_id FROM notifications
             WHERE user_id = $1 AND kind = 'mention' AND comment_id IS NULL ORDER BY id",
        )
        .bind(who.user.id)
        .fetch_all(&self.h.pool)
        .await
        .expect("listed")
    }

    /// The webhooks' service, whose deliveries all arrive.
    fn webhooks(&self) -> WebhookService {
        WebhookService::builder(
            self.h.pool.clone(),
            std::sync::Arc::clone(&self.h.settings),
            Services {
                workspaces: self.h.workspaces.clone(),
                agents: self.h.agents.clone(),
                pages: self.h.pages.clone(),
                databases: self.h.databases.clone(),
                sync: self.sync.clone(),
                comments: self.comments.clone(),
            },
            std::sync::Arc::new(Accepting),
        )
        .envelope(envelope())
        .build()
    }

    /// A live documents' service of its own, with these limits.
    fn sync_with(&self, limits: Limits) -> SyncService {
        let (_, feed) = access_feed();
        SyncService::builder(
            self.h.pool.clone(),
            self.h.pages.clone(),
            self.h.auth.clone(),
            feed,
        )
        .limits(limits)
        .build()
    }

    async fn post(&self, who: &Principal, markdown: &str) -> neon_app::comments::Posted {
        self.comments
            .start_thread(who, self.page, markdown, Placing::Page)
            .await
            .expect("posted")
    }

    /// The jobs of `kind` queued, oldest first, taken out of the queue.
    async fn jobs(&self, kind: &str) -> Vec<Job> {
        let rows = sqlx::query(
            "DELETE FROM jobs WHERE kind = $1
             RETURNING id, kind, payload, attempts, max_attempts, traceparent, created_at",
        )
        .bind(kind)
        .fetch_all(&self.h.pool)
        .await
        .expect("the jobs");
        let mut jobs: Vec<Job> = rows
            .into_iter()
            .map(|row| Job {
                id: row.get("id"),
                kind: row.get("kind"),
                payload: row.get("payload"),
                attempt: row.get("attempts"),
                max_attempts: row.get("max_attempts"),
                traceparent: row.get("traceparent"),
                created_at: row.get("created_at"),
            })
            .collect();
        jobs.sort_by_key(|job| job.id);
        jobs
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_tells_those_who_see_the_page_and_the_writer_of_the_others(pool: PgPool) {
    let t = team(pool).await;
    let posted = t
        .comments
        .start_thread(
            &t.ada,
            t.secret,
            &format!("{} and {}, look", mention(&t.bea), mention(&t.ada)),
            Placing::Page,
        )
        .await
        .expect("posted");
    // Bea may not see Ada's private page: she is told nothing, and Ada hears so.
    assert_eq!(t.made(&t.bea).await, 0);
    let unnotified: Vec<Uuid> = posted.unnotified.iter().map(|who| who.id).collect();
    assert_eq!(unnotified, [t.bea.user.id]);
    assert_eq!(
        t.made(&t.ada).await,
        0,
        "nobody is told of their own comment"
    );

    let posted = t
        .post(&t.ada, &format!("{} see this", mention(&t.bea)))
        .await;
    assert!(posted.unnotified.is_empty());
    assert_eq!(
        t.shown(&t.bea).await,
        [(NotificationKind::Mention, Some(posted.comment_id))]
    );
    assert_eq!(t.unread(&t.bea).await, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn replies_tell_the_thread_and_comments_tell_the_followers_each_once(pool: PgPool) {
    let t = team(pool).await;
    // Ada made the page, so she follows it: Bea's thread tells her.
    let started = t.post(&t.bea, &format!("Plan? {}", mention(&t.cy))).await;
    let thread = started.thread.thread.id;
    assert_eq!(t.kinds(&t.ada).await, [NotificationKind::Comment]);
    assert_eq!(t.kinds(&t.cy).await, [NotificationKind::Mention]);

    // Cy replies: Bea wrote in the thread, Ada follows the page. Cy, mentioned before, is
    // not told of her own reply.
    let reply = t
        .comments
        .reply(&t.cy, thread, "Friday.")
        .await
        .expect("replied");
    assert_eq!(
        t.shown(&t.bea).await,
        [(NotificationKind::Reply, Some(reply.comment_id))]
    );
    assert_eq!(
        t.kinds(&t.ada).await,
        [NotificationKind::Comment, NotificationKind::Comment]
    );
    assert_eq!(t.made(&t.cy).await, 1);

    // Ada replies mentioning Bea: one notification, the mention, though Bea is in the
    // thread and follows the page now.
    let second = t
        .comments
        .reply(&t.ada, thread, &format!("{} ok?", mention(&t.bea)))
        .await
        .expect("replied");
    let bea = t.shown(&t.bea).await;
    assert_eq!(bea[0], (NotificationKind::Mention, Some(second.comment_id)));
    assert_eq!(bea.len(), 2);
    assert_eq!(
        t.shown(&t.cy).await[0],
        (NotificationKind::Reply, Some(second.comment_id)),
        "who was mentioned in the thread is in it"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn following_is_by_making_commenting_being_mentioned_or_choice(pool: PgPool) {
    let t = team(pool).await;
    // Ada stops following her own page.
    t.comments
        .follow(&t.ada, t.page, false)
        .await
        .expect("stopped");
    t.post(&t.bea, "First").await;
    assert_eq!(t.made(&t.ada).await, 0);
    // Cy chose to follow it; Bea follows it, having commented.
    t.comments
        .follow(&t.cy, t.page, true)
        .await
        .expect("followed");
    t.post(&t.ada, "Second").await;
    assert_eq!(t.kinds(&t.cy).await, [NotificationKind::Comment]);
    assert_eq!(t.kinds(&t.bea).await, [NotificationKind::Comment]);
    // Commenting again does not make Ada follow again: she chose.
    t.post(&t.cy, "Third").await;
    assert_eq!(t.made(&t.ada).await, 0);
    let listed = t
        .comments
        .threads(&t.ada, t.page, ThreadQuery::default())
        .await
        .expect("listed");
    assert!(!listed.following);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_edit_tells_only_those_it_mentions_anew_and_a_deletion_takes_them(pool: PgPool) {
    let t = team(pool).await;
    t.comments
        .follow(&t.ada, t.page, false)
        .await
        .expect("stopped");
    let posted = t.post(&t.ada, &format!("{} first", mention(&t.bea))).await;
    let comment = posted.comment_id;
    t.comments
        .edit(
            &t.ada,
            comment,
            &format!("{} first, edited", mention(&t.bea)),
        )
        .await
        .expect("edited");
    assert_eq!(t.made(&t.bea).await, 1, "not told again");
    t.comments
        .edit(
            &t.ada,
            comment,
            &format!("{} and {}", mention(&t.bea), mention(&t.cy)),
        )
        .await
        .expect("edited");
    assert_eq!(t.kinds(&t.cy).await, [NotificationKind::Mention]);
    assert_eq!(t.made(&t.bea).await, 1);

    t.comments.delete(&t.ada, comment).await.expect("deleted");
    assert_eq!(t.made(&t.bea).await, 0);
    assert_eq!(t.made(&t.cy).await, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn what_its_person_can_no_longer_see_is_not_shown_until_they_can_again(pool: PgPool) {
    let t = team(pool).await;
    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::View)
        .await
        .expect("shared");
    t.post(&t.ada, &format!("{} hi", mention(&t.gus))).await;
    assert_eq!(t.kinds(&t.gus).await, [NotificationKind::Mention]);
    let shown = t
        .inbox
        .inbox(&t.gus, InboxQuery::default())
        .await
        .expect("read");
    assert!(shown.notifications[0].members_hidden);

    t.h.pages
        .remove_share(&t.ada, t.page, t.gus.user.id)
        .await
        .expect("removed");
    assert!(t.kinds(&t.gus).await.is_empty());
    assert_eq!(t.unread(&t.gus).await, 0);
    assert_eq!(t.made(&t.gus).await, 1, "kept, not shown");

    t.h.pages
        .share_page(&t.ada, t.page, t.gus.user.id, PageAccess::Comment)
        .await
        .expect("shared");
    assert_eq!(t.kinds(&t.gus).await, [NotificationKind::Mention]);
    assert_eq!(t.unread(&t.gus).await, 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn notifications_are_read_archived_and_listed_a_part_at_a_time(pool: PgPool) {
    let t = team(pool).await;
    for n in 0..3 {
        t.post(&t.bea, &format!("{} {n}", mention(&t.cy))).await;
    }
    let all = t
        .inbox
        .inbox(&t.cy, InboxQuery::default())
        .await
        .expect("read");
    let ids: Vec<Uuid> = all
        .notifications
        .iter()
        .map(|shown| shown.notification.id)
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "newest first");
    assert!(
        all.notifications[0]
            .excerpt
            .as_deref()
            .is_some_and(|text| text.ends_with('2'))
    );

    let first = t
        .inbox
        .inbox(
            &t.cy,
            InboxQuery {
                limit: Some(2),
                ..InboxQuery::default()
            },
        )
        .await
        .expect("read");
    assert_eq!(first.notifications.len(), 2);
    let rest = t
        .inbox
        .inbox(
            &t.cy,
            InboxQuery {
                before: first.next,
                limit: Some(2),
                ..InboxQuery::default()
            },
        )
        .await
        .expect("read");
    assert_eq!(rest.notifications.len(), 1);
    assert_eq!(rest.next, None);

    t.inbox
        .mark(&t.cy, ids[0], Some(true), None)
        .await
        .expect("read");
    t.inbox
        .mark(&t.cy, ids[1], None, Some(true))
        .await
        .expect("archived");
    assert_eq!(t.unread(&t.cy).await, 1);
    let unread = t
        .inbox
        .inbox(
            &t.cy,
            InboxQuery {
                unread: true,
                ..InboxQuery::default()
            },
        )
        .await
        .expect("read");
    assert_eq!(unread.notifications.len(), 1);
    assert_eq!(unread.notifications[0].notification.id, ids[2]);
    let archived = t
        .inbox
        .inbox(
            &t.cy,
            InboxQuery {
                archived: true,
                ..InboxQuery::default()
            },
        )
        .await
        .expect("read");
    assert_eq!(archived.notifications.len(), 1);
    assert_eq!(archived.notifications[0].notification.id, ids[1]);

    assert_eq!(t.inbox.mark_all_read(&t.cy, None).await.expect("marked"), 2);
    assert_eq!(t.unread(&t.cy).await, 0);
    t.inbox
        .mark(&t.cy, ids[0], Some(false), None)
        .await
        .expect("unread");
    assert_eq!(t.unread(&t.cy).await, 1);
    // Someone else's is not found.
    let other = t.inbox.mark(&t.bea, ids[0], Some(true), None).await;
    assert!(matches!(other, Err(InboxError::NotFound(_))), "{other:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_inbox_opened_live_hears_each_change_to_it(pool: PgPool) {
    let t = team(pool).await;
    let (outbox, mut heard) = Outbox::new(8, 4096);
    let client = neon_app::sync::ClientId::next();
    t.h.broadcaster
        .join(Room::Inbox(t.bea.user.id), client, outbox);
    t.post(&t.ada, &format!("{} now", mention(&t.bea))).await;
    let Next::Send(message) = timeout(PATIENCE, heard.next()).await.expect("heard") else {
        panic!("closed")
    };
    assert_eq!(&message[..], br#"{"type":"inbox"}"#);
    t.inbox.mark_all_read(&t.bea, None).await.expect("marked");
    let Next::Send(message) = timeout(PATIENCE, heard.next()).await.expect("heard") else {
        panic!("closed")
    };
    assert_eq!(&message[..], br#"{"type":"inbox"}"#);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_pages_text_tells_those_it_mentions_once_it_settles(pool: PgPool) {
    let t = team(pool).await;
    let text = |markdown: String| from_markdown(&markdown).blocks;
    t.sync
        .append(
            &t.ada,
            t.page,
            text(format!("Ask {} and {}", mention(&t.bea), mention(&t.ada))),
        )
        .await
        .expect("appended");
    // The exact condition: Bea's notification is there, once the text settled.
    let deadline = Instant::now() + PATIENCE;
    while t.made(&t.bea).await == 0 {
        assert!(Instant::now() < deadline, "the mention never settled");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let shown = t
        .inbox
        .inbox(&t.bea, InboxQuery::default())
        .await
        .expect("read");
    let notification = &shown.notifications[0].notification;
    assert_eq!(notification.kind, NotificationKind::Mention);
    assert_eq!(notification.comment_id, None);
    assert_eq!(notification.actor_id, Some(t.ada.user.id));
    assert!(notification.block_id.is_some());
    assert_eq!(
        shown.notifications[0].excerpt.as_deref(),
        Some(
            format!(
                "Ask @{} and @{}",
                t.bea.user.display_name, t.ada.user.display_name
            )
            .as_str()
        ),
        "quoting its block"
    );
    assert_eq!(t.made(&t.ada).await, 0, "nobody is told of their own text");

    // More text, the mention still there, and a save: nobody is told again. Cy, who sees
    // the page, is mentioned anew; Gus, who does not, is told nothing.
    t.sync
        .append(
            &t.ada,
            t.page,
            text(format!("Also {} and {}", mention(&t.cy), mention(&t.gus))),
        )
        .await
        .expect("appended");
    t.sync.shutdown().await;
    assert_eq!(t.made(&t.bea).await, 1);
    assert_eq!(t.kinds(&t.cy).await, [NotificationKind::Mention]);
    assert_eq!(t.made(&t.gus).await, 0);
    // Being mentioned on it, they follow its comments.
    t.post(&t.ada, "A comment").await;
    assert_eq!(
        t.kinds(&t.bea).await,
        [NotificationKind::Comment, NotificationKind::Mention]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn assigning_a_row_tells_whom_it_assigns(pool: PgPool) {
    use neon_app::databases::{NewDatabase, NewProperty, NewRow, PropertySpec};
    use neon_domain::databases::PropertyKind;

    let t = team(pool).await;
    let db =
        t.h.databases
            .create_database(
                &t.ada,
                t.workspace,
                Location::Teamspace(t.general),
                Placement::Last,
                NewDatabase {
                    title: "Tasks".into(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("created")
            .page
            .page
            .id;
    t.h.databases
        .add_property(
            &t.ada,
            db,
            NewProperty {
                name: "Assignee".into(),
                spec: PropertySpec {
                    kind: PropertyKind::Person,
                    number_format: None,
                    currency: None,
                    prefix: None,
                },
                options: None,
                placement: Placement::Last,
            },
        )
        .await
        .expect("added");
    let values = |people: &[&Principal]| {
        let ids: Vec<String> = people.iter().map(|who| who.user.id.to_string()).collect();
        json!({"Name": "Fix login", "Assignee": ids})
            .as_object()
            .cloned()
            .expect("an object")
    };
    let row =
        t.h.databases
            .create_row(
                &t.ada,
                db,
                NewRow {
                    values: values(&[&t.bea, &t.ada]),
                    ..NewRow::default()
                },
            )
            .await
            .expect("created")
            .id;
    assert_eq!(t.kinds(&t.bea).await, [NotificationKind::Assigned]);
    assert_eq!(t.made(&t.ada).await, 0, "assigning oneself tells nobody");
    let shown = t
        .inbox
        .inbox(&t.bea, InboxQuery::default())
        .await
        .expect("read");
    assert_eq!(shown.notifications[0].notification.page_id, Some(row));

    // Taken off and put back while the first waits unread: one notification.
    t.h.databases
        .update_row(&t.ada, row, values(&[]))
        .await
        .expect("changed");
    t.h.databases
        .update_row(&t.ada, row, values(&[&t.bea, &t.cy]))
        .await
        .expect("changed");
    assert_eq!(t.made(&t.bea).await, 1);
    assert_eq!(t.kinds(&t.cy).await, [NotificationKind::Assigned]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn invitations_come_to_the_inbox_while_they_wait(pool: PgPool) {
    let t = team(pool).await;
    let dan = t.h.person("dan@example.com").await;
    let sent =
        t.h.workspaces
            .invite(
                &t.ada,
                t.workspace,
                dan.user.address(),
                WorkspaceRole::Member,
            )
            .await
            .expect("invited");
    let shown = t
        .inbox
        .inbox(&dan, InboxQuery::default())
        .await
        .expect("read");
    assert_eq!(shown.notifications.len(), 1);
    let invitation = &shown.notifications[0];
    assert_eq!(invitation.notification.kind, NotificationKind::Invitation);
    assert_eq!(invitation.workspace.name, "Design");
    assert_eq!(
        invitation
            .invitation
            .as_ref()
            .map(|invitation| invitation.id),
        Some(sent.invitation.id)
    );
    assert_eq!(t.unread(&dan).await, 1);

    t.h.workspaces
        .accept_invitation(&dan, sent.invitation.id)
        .await
        .expect("accepted");
    assert!(t.kinds(&dan).await.is_empty(), "answered, it is gone");
    assert_eq!(t.unread(&dan).await, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_token_bound_to_a_workspace_reads_its_notifications_alone(pool: PgPool) {
    let t = team(pool).await;
    let scout = t.h.agent(&t.bea, t.workspace, "Scout").await;
    let (_, agent) = t.h.agent_token(&t.bea, &scout, &["read", "write"]).await;
    let posted = t
        .post(&t.ada, &format!("@[Scout](user:{}) take this", scout.id))
        .await;
    let shown = t
        .inbox
        .inbox(&agent, InboxQuery::default())
        .await
        .expect("read");
    assert_eq!(shown.notifications.len(), 1);
    assert_eq!(
        shown.notifications[0].notification.comment_id,
        Some(posted.comment_id)
    );
    // Another workspace is hidden to it.
    let other = t.h.workspace(&t.bea, "Elsewhere").await.workspace.id;
    let hidden = t
        .inbox
        .inbox(
            &agent,
            InboxQuery {
                workspace_id: Some(other),
                ..InboxQuery::default()
            },
        )
        .await;
    assert!(matches!(hidden, Err(InboxError::NotFound(_))), "{hidden:?}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mail_for_each_goes_once_it_waited_unread_and_says_only_what_may_be_seen(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    let job = NotificationMailJob::new(t.inbox.clone());
    let posted = t
        .post(&t.ada, &format!("{} the **plan**", mention(&t.bea)))
        .await;
    let jobs = t.jobs(neon_app::notifications::MAIL_KIND).await;
    assert_eq!(jobs.len(), 1);
    // Before its delay, it waits.
    job.run(&jobs[0]).await.expect("ran");
    assert!(t.h.mails().await.is_empty());
    t.h.time.advance(Duration::from_secs(5 * 60));
    job.run(&jobs[0]).await.expect("ran");
    let mails = t.h.mails().await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to.address, "bea@example.com");
    let Message::Notification {
        notice,
        unsubscribe,
    } = &mails[0].message
    else {
        panic!("a notification: {:?}", mails[0].message)
    };
    assert_eq!(
        notice.summary,
        "ada mentioned you in a comment on Notes, in Design"
    );
    assert!(
        notice
            .excerpt
            .as_deref()
            .is_some_and(|text| text.ends_with("the plan"))
    );
    assert!(
        notice
            .path
            .ends_with(&format!("?comment={}", posted.thread.thread.id)),
        "{}",
        notice.path
    );
    // The link works, once, as signed for her.
    t.inbox
        .unsubscribe(unsubscribe.expose())
        .await
        .expect("stopped");
    assert_eq!(
        t.inbox
            .mail_settings(&t.bea)
            .await
            .expect("read")
            .preference,
        MailPreference::None
    );

    // Run again, the mail went already: nothing more.
    job.run(&jobs[0]).await.expect("ran");
    assert!(t.h.mails().await.is_empty());

    // Read before it goes, or out of reach, nothing goes either.
    t.inbox
        .set_mail(&t.cy, MailPreference::Each)
        .await
        .expect("set");
    t.post(&t.ada, &format!("{} again", mention(&t.cy))).await;
    let jobs = t.jobs(neon_app::notifications::MAIL_KIND).await;
    t.inbox.mark_all_read(&t.cy, None).await.expect("read");
    t.h.time.advance(Duration::from_secs(5 * 60));
    for queued in &jobs {
        job.run(queued).await.expect("ran");
    }
    assert!(t.h.mails().await.is_empty());
}

/// When the mail of `kind` queued waits until, as of the harness's start.
async fn due_after_start(t: &Team) -> time::Duration {
    let due: time::OffsetDateTime =
        sqlx::query_scalar("SELECT run_at FROM jobs WHERE kind = 'notifications.mail'")
            .fetch_one(&t.h.pool)
            .await
            .expect("queued");
    due - support::start()
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_flood_of_notifications_makes_one_mail_and_the_next_waits_its_gap(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    let job = NotificationMailJob::new(t.inbox.clone());
    // Bea starts forty threads on Ada's page, which Ada follows as its maker.
    for n in 0..40 {
        t.post(&t.bea, &format!("Thread {n}")).await;
    }
    assert_eq!(t.made(&t.ada).await, 40);
    let jobs = t.jobs(neon_app::notifications::MAIL_KIND).await;
    assert_eq!(jobs.len(), 1, "one mail asked for, not one each");
    assert_eq!(jobs[0].payload, json!({"user_id": t.ada.user.id}));
    t.h.time.advance(Duration::from_secs(5 * 60));
    job.run(&jobs[0]).await.expect("ran");
    let mails = t.h.mails().await;
    assert_eq!(mails.len(), 1);
    let Message::Digest { notices, more, .. } = &mails[0].message else {
        panic!("a mail that lists them: {:?}", mails[0].message)
    };
    assert_eq!((notices.len(), *more), (20, 20));
    assert!(
        t.jobs(neon_app::notifications::MAIL_KIND).await.is_empty(),
        "nothing more waits"
    );

    // One more: its mail waits for the gap after the last, longer than its delay.
    t.post(&t.bea, "One more").await;
    assert_eq!(due_after_start(&t).await, time::Duration::minutes(20));
    let jobs = t.jobs(neon_app::notifications::MAIL_KIND).await;
    job.run(&jobs[0]).await.expect("ran");
    assert!(t.h.mails().await.is_empty(), "not before its time");
    t.h.time.advance(Duration::from_secs(15 * 60));
    job.run(&jobs[0]).await.expect("ran");
    let mails = t.h.mails().await;
    assert_eq!(mails.len(), 1);
    assert!(
        matches!(mails[0].message, Message::Notification { .. }),
        "{:?}",
        mails[0].message
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn what_comes_while_a_mail_waits_goes_once_it_too_has_waited(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    let job = NotificationMailJob::new(t.inbox.clone());
    t.post(&t.bea, "First").await;
    let jobs = t.jobs(neon_app::notifications::MAIL_KIND).await;
    t.h.time.advance(Duration::from_secs(4 * 60));
    t.post(&t.bea, "Second").await;
    assert!(
        t.jobs(neon_app::notifications::MAIL_KIND).await.is_empty(),
        "the mail that waits takes it in"
    );
    t.h.time.advance(Duration::from_secs(60));
    job.run(&jobs[0]).await.expect("ran");
    let mails = t.h.mails().await;
    assert_eq!(mails.len(), 1);
    let Message::Notification { notice, .. } = &mails[0].message else {
        panic!("the first alone: {:?}", mails[0].message)
    };
    assert_eq!(notice.excerpt.as_deref(), Some("First"));
    // The second, a minute old then, goes in the next, a gap after this one.
    assert_eq!(due_after_start(&t).await, time::Duration::minutes(20));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mail_waits_for_its_delay_and_the_instance_and_person_to_want_it(pool: PgPool) {
    let t = team(pool).await;
    t.comments
        .follow(&t.ada, t.page, false)
        .await
        .expect("stopped");
    // Off on the instance, by default: no job.
    t.post(&t.ada, &format!("{} one", mention(&t.bea))).await;
    assert!(t.jobs(neon_app::notifications::MAIL_KIND).await.is_empty());

    t.h.set("notifications.mail", json!(true)).await;
    t.h.set("notifications.mail_delay", json!("10m")).await;
    t.post(&t.ada, &format!("{} two", mention(&t.bea))).await;
    let due: time::OffsetDateTime =
        sqlx::query_scalar("SELECT run_at FROM jobs WHERE kind = 'notifications.mail'")
            .fetch_one(&t.h.pool)
            .await
            .expect("queued");
    assert_eq!(due - support::start(), time::Duration::minutes(10));
    t.jobs(neon_app::notifications::MAIL_KIND).await;

    // Those who want none, or a digest, and agents, get no job.
    t.inbox
        .set_mail(&t.bea, MailPreference::Digest)
        .await
        .expect("set");
    t.post(&t.ada, &format!("{} three", mention(&t.bea))).await;
    assert!(t.jobs(neon_app::notifications::MAIL_KIND).await.is_empty());
    let scout = t.h.agent(&t.ada, t.workspace, "Scout").await;
    t.post(&t.bea, &format!("@[Scout](user:{}) four", scout.id))
        .await;
    assert!(t.jobs(neon_app::notifications::MAIL_KIND).await.is_empty());
    let (_, agent) = t.h.agent_token(&t.ada, &scout, &["read", "write"]).await;
    let refused = t.inbox.set_mail(&agent, MailPreference::Each).await;
    assert!(
        matches!(refused, Err(InboxError::Forbidden(_))),
        "{refused:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_digest_tells_what_came_since_the_last_and_may_be_seen(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    t.inbox
        .set_mail(&t.bea, MailPreference::Digest)
        .await
        .expect("set");
    t.post(&t.ada, &format!("{} one", mention(&t.bea))).await;
    // Shared with Bea when she is mentioned there, and no longer when the digest goes.
    t.h.pages
        .share_page(&t.ada, t.secret, t.bea.user.id, PageAccess::View)
        .await
        .expect("shared");
    t.comments
        .start_thread(
            &t.ada,
            t.secret,
            &format!("{} not for you", mention(&t.bea)),
            Placing::Page,
        )
        .await
        .expect("posted");
    assert_eq!(t.made(&t.bea).await, 2);
    t.h.pages
        .remove_share(&t.ada, t.secret, t.bea.user.id)
        .await
        .expect("removed");
    t.post(&t.cy, &format!("{} two", mention(&t.bea))).await;

    assert_eq!(t.inbox.send_digests().await.expect("sent"), 1);
    let mails = t.h.mails().await;
    assert_eq!(mails.len(), 1);
    let Message::Digest {
        notices,
        more,
        path,
        ..
    } = &mails[0].message
    else {
        panic!("a digest: {:?}", mails[0].message)
    };
    let summaries: Vec<&str> = notices
        .iter()
        .map(|notice| notice.summary.as_str())
        .collect();
    assert_eq!(
        summaries,
        [
            "cy mentioned you in a comment on Notes, in Design",
            "ada mentioned you in a comment on Notes, in Design"
        ]
    );
    assert_eq!(*more, 0);
    assert_eq!(path, "/inbox");

    // Until the interval passes, none goes; then one of what came since.
    t.post(&t.ada, &format!("{} three", mention(&t.bea))).await;
    assert_eq!(t.inbox.send_digests().await.expect("sent"), 0);
    t.h.time.advance(Duration::from_secs(3 * 3600));
    assert_eq!(t.inbox.send_digests().await.expect("sent"), 1);
    let Message::Digest { notices, .. } = &t.h.mails().await[0].message else {
        panic!("a digest")
    };
    assert_eq!(notices.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_unsubscribe_link_works_only_as_signed(pool: PgPool) {
    let t = team(pool).await;
    let token = t.inbox.unsubscribe_token(t.bea.user.id).expect("signed");
    let (subject, signature) = token.split_once('.').expect("two parts");
    assert_eq!(subject, t.bea.user.id.to_string());
    for forged in [
        format!("{}.{signature}", t.cy.user.id),
        format!("{subject}.{}", &signature[..signature.len() - 1]),
        "nonsense".to_owned(),
        String::new(),
    ] {
        let refused = t.inbox.unsubscribe(&forged).await;
        assert!(
            matches!(refused, Err(InboxError::InvalidToken)),
            "{forged}: {refused:?}"
        );
    }
    assert_eq!(
        t.inbox
            .mail_settings(&t.bea)
            .await
            .expect("read")
            .preference,
        MailPreference::Each
    );
    t.inbox.unsubscribe(&token).await.expect("stopped");
    assert_eq!(
        t.inbox
            .mail_settings(&t.bea)
            .await
            .expect("read")
            .preference,
        MailPreference::None
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mentions_and_replies_reach_webhooks_that_ask_for_them(pool: PgPool) {
    let t = team(pool).await;
    t.webhooks()
        .create_webhook(
            &t.bea,
            t.workspace,
            NewWebhook {
                user_id: t.bea.user.id,
                url: "https://hooks.example.com/bea".into(),
                events: vec!["page.mentioned".into(), "comment.replied".into()],
                include_own: false,
            },
        )
        .await
        .expect("registered");
    let started = t.post(&t.bea, "Mine").await;
    t.comments
        .reply(
            &t.ada,
            started.thread.thread.id,
            &format!("{} replied", mention(&t.bea)),
        )
        .await
        .expect("replied");
    // A mention of Bea she may not see the page of makes no event.
    t.comments
        .start_thread(
            &t.ada,
            t.secret,
            &format!("{} hidden", mention(&t.bea)),
            Placing::Page,
        )
        .await
        .expect("posted");
    let kinds: Vec<String> = t
        .jobs(neon_app::webhooks::DELIVERY_KIND)
        .await
        .iter()
        .map(|job| {
            job.payload["event"]["type"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds, ["page.mentioned", "comment.replied"]);
}

/// Limits under which a document is saved only once its service stops, so every change
/// before then is in the one snapshot.
fn saved_at_the_end() -> Limits {
    Limits {
        persist_delay: Duration::from_millis(5),
        idle_unload: Duration::from_secs(3600),
        mention_settle: Duration::from_secs(3600),
        ..Limits::default()
    }
}

fn text(markdown: &str) -> Vec<neon_blocks::Block> {
    from_markdown(markdown).blocks
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_in_a_pages_text_is_from_whoever_wrote_it(pool: PgPool) {
    let t = team(pool).await;
    let sync = t.sync_with(saved_at_the_end());
    // Ada mentions Bea; before the text is saved, Bea mentions Cy and both write on, so
    // neither the last change nor the one mentioned says whom each mention is from.
    for (who, markdown) in [
        (&t.ada, format!("Ask {}", mention(&t.bea))),
        (&t.bea, format!("And {}", mention(&t.cy))),
        (&t.ada, "More".to_owned()),
        (&t.bea, "Even more".to_owned()),
    ] {
        sync.append(who, t.page, text(&markdown))
            .await
            .expect("appended");
    }
    sync.shutdown().await;
    let from = |told: Vec<(Uuid, Option<Uuid>, Option<String>)>| -> Vec<Option<Uuid>> {
        told.into_iter().map(|(_, actor, _)| actor).collect()
    };
    assert_eq!(
        from(t.text_mentions(&t.bea).await),
        [Some(t.ada.user.id)],
        "Bea is told, though she changed the page since"
    );
    assert_eq!(from(t.text_mentions(&t.cy).await), [Some(t.bea.user.id)]);
    assert_eq!(t.made(&t.ada).await, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_logged_before_a_restart_is_still_from_its_writer(pool: PgPool) {
    let t = team(pool).await;
    // Bea's change was logged by a server that stopped before saving it.
    let update = {
        let doc = yrs::Doc::new();
        let fragment = neon_blocks::fragment(&doc);
        let mut txn = yrs::Transact::transact_mut(&doc);
        neon_blocks::append(
            &mut txn,
            &fragment,
            &text(&format!("For {}", mention(&t.cy))),
            &mut neon_blocks::SequentialIds::new("b"),
        )
        .expect("appended");
        txn.encode_update_v1()
    };
    neon_store::documents::append(
        &t.h.pool,
        t.page,
        &[neon_store::documents::NewUpdate {
            update: &update,
            author_id: t.bea.user.id,
            created_at: support::start(),
        }],
    )
    .await
    .expect("logged");
    let sync = t.sync_with(saved_at_the_end());
    sync.append(&t.ada, t.page, text("Ada's line"))
        .await
        .expect("appended");
    sync.shutdown().await;
    let told = t.text_mentions(&t.cy).await;
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].1, Some(t.bea.user.id));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_page_made_with_text_tells_whom_it_mentions_and_an_import_tells_nobody(pool: PgPool) {
    use neon_app::pages::{ImportedContent, ImportedPage};

    let t = team(pool).await;
    let content = t
        .sync
        .first_snapshot(&from_markdown(&format!(
            "Ask {} and {}",
            mention(&t.bea),
            mention(&t.ada)
        )))
        .await
        .expect("fits");
    let made =
        t.h.pages
            .create_page(
                &t.ada,
                t.workspace,
                Location::Teamspace(t.general),
                Placement::Last,
                NewPage {
                    title: "Plan".into(),
                    content,
                    ..NewPage::default()
                },
            )
            .await
            .expect("created")
            .page
            .id;
    let told = t.text_mentions(&t.bea).await;
    assert_eq!(told.len(), 1, "told as the page is made");
    assert_eq!((told[0].0, told[0].1), (made, Some(t.ada.user.id)));
    assert!(told[0].2.is_some(), "at its block");
    assert_eq!(t.made(&t.ada).await, 0);

    let imported = t
        .h
        .pages
        .import_pages(
            &t.ada,
            t.workspace,
            Location::Teamspace(t.general),
            vec![ImportedPage {
                title: "Old notes".into(),
                content: ImportedContent::of(&from_markdown(&format!("Ask {}", mention(&t.cy)))),
                files: Vec::new(),
                children: Vec::new(),
            }],
        )
        .await
        .expect("imported")
        .pages[0]
        .page
        .id;
    assert_eq!(t.made(&t.cy).await, 0, "imported text tells nobody");
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM page_mentions WHERE page_id = $1")
        .bind(imported)
        .fetch_one(&t.h.pool)
        .await
        .expect("counted");
    assert_eq!(recorded, 1);

    // Cy writes on both and they are saved: nobody is told again, nor as from Cy.
    t.sync
        .append(&t.cy, made, text("More"))
        .await
        .expect("appended");
    t.sync
        .append(&t.cy, imported, text("More"))
        .await
        .expect("appended");
    t.sync.shutdown().await;
    assert_eq!(t.text_mentions(&t.bea).await, told);
    assert_eq!(t.made(&t.cy).await, 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_in_a_block_of_a_long_id_is_told_without_it_and_the_page_saved(pool: PgPool) {
    let t = team(pool).await;
    let long = "L".repeat(200);
    let blocks: Vec<neon_blocks::Block> =
        text(&format!("Quarterly zebra findings, {}", mention(&t.cy)))
            .into_iter()
            .map(|block| block.with_id(long.clone()))
            .collect();
    t.sync
        .append(&t.bea, t.page, blocks)
        .await
        .expect("appended");
    t.sync.shutdown().await;
    let (snapshots, logged, search): (i64, i64, Option<String>) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM page_snapshots WHERE page_id = $1),
                (SELECT count(*) FROM page_updates WHERE page_id = $1),
                (SELECT search_text FROM pages WHERE id = $1)",
    )
    .bind(t.page)
    .fetch_one(&t.h.pool)
    .await
    .expect("read");
    assert_eq!((snapshots, logged), (1, 0), "saved as a snapshot");
    assert!(search.is_some_and(|text| text.contains("zebra")));
    assert_eq!(
        t.text_mentions(&t.cy).await,
        [(t.page, Some(t.bea.user.id), None)]
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_written_again_while_the_first_waits_unread_still_reaches_webhooks(pool: PgPool) {
    let t = team(pool).await;
    t.webhooks()
        .create_webhook(
            &t.bea,
            t.workspace,
            NewWebhook {
                user_id: t.bea.user.id,
                url: "https://hooks.example.com/bea".into(),
                events: vec!["page.mentioned".into()],
                include_own: false,
            },
        )
        .await
        .expect("registered");
    // Each change saved at once.
    let sync = t.sync_with(Limits {
        compact_updates: 1,
        ..saved_at_the_end()
    });
    let mentioned = text(&format!("Ask {}", mention(&t.bea)));
    let events = async || {
        t.jobs(neon_app::webhooks::DELIVERY_KIND)
            .await
            .iter()
            .map(|job| {
                job.payload["event"]["type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    sync.append(&t.ada, t.page, mentioned.clone())
        .await
        .expect("appended");
    assert_eq!(events().await, ["page.mentioned"]);
    // Taken out, saved, and written again while Bea has not read the first.
    let (_, document) = sync.read(&t.ada, t.page).await.expect("read");
    sync.edit(
        &t.ada,
        t.page,
        vec![neon_blocks::edit::Edit::ReplaceBody {
            blocks: text("Nobody"),
        }],
        Some(neon_app::sync::revision(&document)),
    )
    .await
    .expect("edited");
    let deadline = Instant::now() + PATIENCE;
    while sqlx::query_scalar::<_, i64>("SELECT count(*) FROM page_mentions WHERE page_id = $1")
        .bind(t.page)
        .fetch_one(&t.h.pool)
        .await
        .expect("counted")
        > 0
    {
        assert!(
            Instant::now() < deadline,
            "the text without the mention never saved"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sync.append(&t.ada, t.page, mentioned)
        .await
        .expect("appended");
    sync.shutdown().await;
    assert_eq!(events().await, ["page.mentioned"], "told again");
    assert_eq!(
        t.made(&t.bea).await,
        1,
        "one notification while it waits unread"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_in_a_pages_text_quotes_its_block_as_it_was_in_the_inbox_and_mail(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    let content = t
        .sync
        .first_snapshot(&from_markdown(&format!(
            "# Goals\n\n- Stage owner: {}\n  - Rehearse twice\n\nMore text",
            mention(&t.bea)
        )))
        .await
        .expect("fits");
    let made =
        t.h.pages
            .create_page(
                &t.ada,
                t.workspace,
                Location::Teamspace(t.general),
                Placement::Last,
                NewPage {
                    title: "Plan".into(),
                    content,
                    ..NewPage::default()
                },
            )
            .await
            .expect("created")
            .page
            .id;
    let quoted = format!("Stage owner: @{}", t.bea.user.display_name);
    let excerpt = || async {
        t.inbox
            .inbox(&t.bea, InboxQuery::default())
            .await
            .expect("read")
            .notifications[0]
            .excerpt
            .clone()
    };
    // The block alone, without the one under it.
    assert_eq!(excerpt().await.as_deref(), Some(quoted.as_str()));

    // The block changed since, the notification still quotes it as it was when told.
    t.sync
        .append(&t.ada, made, text("Changed later"))
        .await
        .expect("appended");
    t.sync.shutdown().await;
    assert_eq!(excerpt().await.as_deref(), Some(quoted.as_str()));

    let job = NotificationMailJob::new(t.inbox.clone());
    t.h.time.advance(Duration::from_secs(5 * 60));
    for queued in t.jobs(neon_app::notifications::MAIL_KIND).await {
        job.run(&queued).await.expect("ran");
    }
    let mails = t.h.mails().await;
    let Some(Message::Notification { notice, .. }) = mails.first().map(|mail| &mail.message) else {
        panic!("a notification's mail: {mails:?}")
    };
    assert_eq!(notice.summary, "ada mentioned you on Plan, in Design");
    assert_eq!(notice.excerpt.as_deref(), Some(quoted.as_str()));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mention_an_edit_adds_is_told_as_of_the_edit_and_mailed_from_then(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    t.comments
        .follow(&t.ada, t.page, false)
        .await
        .expect("stopped");
    let posted = t.post(&t.ada, "First").await;
    t.h.time.advance(Duration::from_secs(2 * 3600));
    t.comments
        .edit(
            &t.ada,
            posted.comment_id,
            &format!("First, {}", mention(&t.cy)),
        )
        .await
        .expect("edited");
    let made: time::OffsetDateTime = sqlx::query_scalar(
        "SELECT created_at FROM notifications WHERE user_id = $1 AND kind = 'mention'",
    )
    .bind(t.cy.user.id)
    .fetch_one(&t.h.pool)
    .await
    .expect("made");
    assert_eq!(made - support::start(), time::Duration::hours(2));
    let due: time::OffsetDateTime =
        sqlx::query_scalar("SELECT run_at FROM jobs WHERE kind = 'notifications.mail'")
            .fetch_one(&t.h.pool)
            .await
            .expect("queued");
    assert_eq!(
        due - support::start(),
        time::Duration::hours(2) + time::Duration::minutes(5),
        "the mail waits its delay after the edit"
    );
}

/// Listens to someone's inbox's room, as their open tabs and MCP subscriptions do.
fn listen(t: &Team, who: &Principal) -> neon_app::sync::OutboxReceiver {
    let (outbox, heard) = Outbox::new(16, 4096);
    t.h.broadcaster.join(
        Room::Inbox(who.user.id),
        neon_app::sync::ClientId::next(),
        outbox,
    );
    heard
}

async fn hears_the_inbox(heard: &mut neon_app::sync::OutboxReceiver, what: &str) {
    let Next::Send(message) = timeout(PATIENCE, heard.next())
        .await
        .unwrap_or_else(|_| panic!("{what}: nothing heard"))
    else {
        panic!("{what}: closed")
    };
    assert_eq!(&message[..], br#"{"type":"inbox"}"#, "{what}");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_inbox_hears_when_a_deletion_or_an_answer_takes_from_it(pool: PgPool) {
    let t = team(pool).await;
    let mut bea = listen(&t, &t.bea);
    let posted = t.post(&t.ada, &format!("{} look", mention(&t.bea))).await;
    hears_the_inbox(&mut bea, "the mention").await;
    t.comments
        .delete(&t.ada, posted.comment_id)
        .await
        .expect("deleted");
    hears_the_inbox(&mut bea, "the comment deleted").await;
    assert_eq!(t.unread(&t.bea).await, 0);

    let dan = t.h.person("dan@example.com").await;
    let mut heard = listen(&t, &dan);
    let invite = || async {
        t.h.workspaces
            .invite(
                &t.ada,
                t.workspace,
                dan.user.address(),
                WorkspaceRole::Member,
            )
            .await
            .expect("invited")
            .invitation
            .id
    };
    let sent = invite().await;
    hears_the_inbox(&mut heard, "invited").await;
    t.h.workspaces
        .revoke_invitation(&t.ada, t.workspace, sent)
        .await
        .expect("revoked");
    hears_the_inbox(&mut heard, "revoked").await;
    let sent = invite().await;
    hears_the_inbox(&mut heard, "invited again").await;
    t.h.workspaces
        .decline_invitation(&dan, sent)
        .await
        .expect("declined");
    hears_the_inbox(&mut heard, "declined").await;
    let sent = invite().await;
    hears_the_inbox(&mut heard, "invited once more").await;
    t.h.workspaces
        .accept_invitation(&dan, sent)
        .await
        .expect("accepted");
    hears_the_inbox(&mut heard, "accepted").await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_mail_whose_job_was_lost_is_asked_for_again(pool: PgPool) {
    let t = team_with(pool, vec![("notifications.mail", json!(true))]).await;
    t.post(&t.bea, "First").await;
    // Its job is gone, as a dead one goes with the queue's retention.
    t.jobs(neon_app::notifications::MAIL_KIND).await;
    t.post(&t.bea, "Second").await;
    assert!(t.jobs(neon_app::notifications::MAIL_KIND).await.is_empty());
    t.h.time.advance(Duration::from_secs(2 * 3600));
    t.post(&t.bea, "Third").await;
    assert_eq!(
        due_after_start(&t).await,
        time::Duration::hours(2) + time::Duration::minutes(5)
    );
}
