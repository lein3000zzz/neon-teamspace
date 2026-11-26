//! Outgoing webhooks' events: every writer of a row's values queues the deliveries its
//! change makes, on its own transaction, for the webhooks that ask for them; a delivery
//! carries the trace of the change that caused it; and one that fails in the server is
//! logged and counted as any other.
//!
//! Telemetry is installed once per process, so these tests sit in their own binary.

mod support;

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use neon_app::crypto::{Envelope, Keyring};
use neon_app::databases::PropertySpec;
use neon_app::databases::{GroupMove, NewDatabase, NewProperty, NewRow, NewView, RowMove};
use neon_app::jobs::{RedHook, Worker, WorkerConfig, run_span};
use neon_app::pages::Location;
use neon_app::sync::{SyncService, access_feed};
use neon_app::webhooks::transport::{Answer, Outgoing, SendError, WebhookTransport};
use neon_app::webhooks::{
    BodyError, DELIVERY_KIND, DeliveryJob, IncomingRequest, MAX_ATTEMPTS, NewIncomingWebhook,
    NewWebhook, Services, WebhookService,
};
use neon_app::workspaces::NewTeamspace;
use neon_domain::auth::Principal;
use neon_domain::auth::{Credential, Scope};
use neon_domain::databases::view::GroupingSpec;
use neon_domain::databases::{Layout, PropertyKind, ViewSpec};
use neon_domain::jobs::Backoff;
use neon_domain::pages::Placement;
use neon_domain::workspaces::TeamspaceAccess;
use neon_domain::workspaces::WorkspaceRole;
use neon_observability::propagation::current_traceparent;
use neon_observability::red::Buckets;
use neon_observability::{Config, ServiceInfo, Telemetry};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use support::{Harness, envelope};
use tracing::Instrument as _;
use url::Url;
use uuid::Uuid;

fn telemetry() -> &'static Telemetry {
    static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();
    TELEMETRY.get_or_init(|| {
        let mut config = Config::default();
        config.logging.level = "off".to_owned();
        neon_observability::init(
            &config,
            ServiceInfo::new("neon-test", "0.0.0"),
            &[(neon_app::jobs::SUBSYSTEM, Buckets::Long)],
        )
        .expect("telemetry installs once")
    })
}

/// Answers every request with 200, keeping what it was sent.
#[derive(Default)]
struct Kept(Mutex<Vec<Outgoing>>);

impl Kept {
    fn sent(&self) -> Vec<Outgoing> {
        self.0.lock().expect("lock").clone()
    }
}

#[async_trait]
impl WebhookTransport for Kept {
    async fn check(&self, _: &Url, _: bool) -> Result<(), SendError> {
        Ok(())
    }

    async fn send(&self, request: Outgoing, _: bool) -> Result<Answer, SendError> {
        self.0.lock().expect("lock").push(request);
        Ok(Answer {
            status: 204,
            excerpt: None,
        })
    }
}

/// Ada owns Design, with Dee and Cy as members; Tasks, in General, has an Assignee and a
/// Reviewer; Dee has a webhook for both events.
struct Team {
    h: Harness,
    webhooks: WebhookService,
    kept: Arc<Kept>,
    ada: Principal,
    dee: Principal,
    cy: Principal,
    workspace: Uuid,
    db: Uuid,
    assignee: Uuid,
    hook: Uuid,
}

async fn team(pool: PgPool) -> Team {
    let h = Harness::new(pool).await;
    let ada = h.person("ada@example.com").await;
    let dee = h.person("dee@example.com").await;
    let cy = h.person("cy@example.com").await;
    let id = h.workspace(&ada, "Design").await.workspace.id;
    h.bring_in(&ada, id, &dee, WorkspaceRole::Member).await;
    h.bring_in(&ada, id, &cy, WorkspaceRole::Member).await;
    let general = h
        .workspaces
        .list_teamspaces(&ada, id)
        .await
        .expect("listed")
        .teamspaces[0]
        .teamspace
        .id;
    let db = h
        .databases
        .create_database(
            &ada,
            id,
            Location::Teamspace(general),
            Placement::Last,
            NewDatabase {
                title: "Tasks".to_owned(),
                icon: None,
                inline: false,
            },
        )
        .await
        .expect("created")
        .page
        .page
        .id;
    let person = |name: &str| NewProperty {
        name: name.to_owned(),
        spec: PropertySpec {
            kind: PropertyKind::Person,
            number_format: None,
            currency: None,
            prefix: None,
        },
        options: None,
        placement: Placement::Last,
    };
    let assignee = h
        .databases
        .add_property(&ada, db, person("Assignee"))
        .await
        .expect("added")
        .id;
    h.databases
        .add_property(&ada, db, person("Reviewer"))
        .await
        .expect("added");
    let kept = Arc::new(Kept::default());
    let (_, feed) = access_feed();
    let sync = SyncService::builder(h.pool.clone(), h.pages.clone(), h.auth.clone(), feed).build();
    let webhooks = WebhookService::builder(
        h.pool.clone(),
        Arc::clone(&h.settings),
        Services {
            workspaces: h.workspaces.clone(),
            agents: h.agents.clone(),
            pages: h.pages.clone(),
            databases: h.databases.clone(),
            sync,
        },
        Arc::clone(&kept) as Arc<dyn WebhookTransport>,
    )
    .envelope(envelope())
    .clock(h.time.clock())
    .build();
    let (_, made) = webhooks
        .create_webhook(
            &dee,
            id,
            NewWebhook {
                user_id: dee.user.id,
                url: "https://hooks.example.com/dee".to_owned(),
                events: vec!["row.assigned".to_owned(), "row.status_changed".to_owned()],
                include_own: false,
            },
        )
        .await
        .expect("registered");
    Team {
        h,
        webhooks,
        kept,
        ada,
        dee,
        cy,
        workspace: id,
        db,
        assignee,
        hook: made.id,
    }
}

fn values(json: Value) -> Map<String, Value> {
    json.as_object().cloned().expect("an object")
}

impl Team {
    /// The deliveries queued and not run yet: each one's event type and identity, oldest
    /// first. They are taken off the queue.
    async fn take(&self) -> Vec<(String, Uuid)> {
        let payloads: Vec<Value> =
            sqlx::query_scalar("DELETE FROM jobs WHERE kind = $1 RETURNING payload")
                .bind(DELIVERY_KIND)
                .fetch_all(&self.h.pool)
                .await
                .expect("the queue");
        let mut events: Vec<(String, Uuid, String)> = payloads
            .into_iter()
            .map(|payload| {
                assert_eq!(payload["webhook_id"], self.hook.to_string());
                let event = &payload["event"];
                (
                    event["type"].as_str().expect("a type").to_owned(),
                    event["identity_id"]
                        .as_str()
                        .and_then(|id| Uuid::try_parse(id).ok())
                        .expect("an identity"),
                    event["id"].as_str().expect("an id").to_owned(),
                )
            })
            .collect();
        events.sort_by(|a, b| a.2.cmp(&b.2));
        events
            .into_iter()
            .map(|(kind, who, _)| (kind, who))
            .collect()
    }

    async fn row(&self, by: &Principal, values_: Value) -> Uuid {
        self.h
            .databases
            .create_row(
                by,
                self.db,
                NewRow {
                    values: values(values_),
                    ..NewRow::default()
                },
            )
            .await
            .expect("a row")
            .id
    }

    async fn change(&self, by: &Principal, row: Uuid, values_: Value) {
        self.h
            .databases
            .update_row(by, row, values(values_))
            .await
            .expect("changed");
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_writer_of_values_queues_the_events_its_change_makes(pool: PgPool) {
    let t = team(pool).await;
    let dee = t.dee.user.id;
    let assigned = || ("row.assigned".to_owned(), dee);
    let status = || ("row.status_changed".to_owned(), dee);

    // A new row with Dee in it.
    let task = t
        .row(
            &t.ada,
            json!({"Name": "Fix login", "Assignee": [dee.to_string()]}),
        )
        .await;
    assert_eq!(t.take().await, [assigned()]);

    // Its values changed: a status change for her, as she is assigned; the same status
    // again, or someone else's assignment, nothing for her.
    t.change(&t.ada, task, json!({"Status": "In progress"}))
        .await;
    assert_eq!(t.take().await, [status()]);
    t.change(&t.ada, task, json!({"Status": "In progress"}))
        .await;
    t.change(&t.cy, task, json!({"Reviewer": [t.cy.user.id.to_string()]}))
        .await;
    assert_eq!(t.take().await, []);
    // Added to another person property: assigned again, by that one; with a status change
    // in the same change, both.
    t.change(
        &t.ada,
        task,
        json!({"Reviewer": [dee.to_string()], "Status": "Done"}),
    )
    .await;
    assert_eq!(t.take().await, [assigned(), status()]);

    // Moving a card into her column on a board grouped by Assignee.
    let view =
        t.h.databases
            .create_view(
                &t.ada,
                t.db,
                NewView {
                    name: "By assignee".to_owned(),
                    layout: Layout::Board,
                    settings: ViewSpec {
                        group: Some(GroupingSpec {
                            property: t.assignee.to_string(),
                            order: Vec::new(),
                            hidden: Vec::new(),
                        }),
                        ..ViewSpec::default()
                    },
                    placement: Placement::Last,
                },
            )
            .await
            .expect("a board")
            .id;
    let other = t.row(&t.ada, json!({"Name": "Review"})).await;
    assert_eq!(t.take().await, []);
    t.h.databases
        .move_row(
            &t.ada,
            other,
            RowMove {
                placement: None,
                group: Some(GroupMove {
                    view_id: view,
                    from: None,
                    to: Some(dee),
                }),
            },
        )
        .await
        .expect("moved");
    assert_eq!(t.take().await, [assigned()]);

    // Taken off and put back: assigned again. A change that fails queues nothing.
    t.change(&t.ada, other, json!({"Assignee": null})).await;
    assert_eq!(t.take().await, []);
    let refused =
        t.h.databases
            .update_row(
                &t.ada,
                other,
                values(json!({"Assignee": [dee.to_string()], "Status": "No such status"})),
            )
            .await;
    assert!(refused.is_err());
    assert_eq!(t.take().await, []);
    t.change(&t.ada, other, json!({"Assignee": [dee.to_string()]}))
        .await;
    assert_eq!(t.take().await, [assigned()]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn own_changes_queue_nothing_unless_the_webhook_asks(pool: PgPool) {
    let t = team(pool).await;
    let dee = t.dee.user.id;

    let task = t
        .row(
            &t.dee,
            json!({"Name": "Mine", "Assignee": [dee.to_string()]}),
        )
        .await;
    t.change(&t.dee, task, json!({"Status": "Done"})).await;
    assert_eq!(t.take().await, []);

    t.webhooks
        .update_webhook(
            &t.dee,
            t.workspace,
            t.hook,
            neon_app::webhooks::WebhookUpdate {
                include_own: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("changed");
    t.change(&t.dee, task, json!({"Status": "In progress"}))
        .await;
    assert_eq!(t.take().await, [("row.status_changed".to_owned(), dee)]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_delivery_carries_the_trace_of_the_change_that_caused_it(pool: PgPool) {
    telemetry();
    let t = team(pool.clone()).await;
    let dee = t.dee.user.id;
    let request = tracing::info_span!("request");
    let caused_by = request
        .in_scope(current_traceparent)
        .expect("spans have a trace context");
    t.h.databases
        .create_row(
            &t.ada,
            t.db,
            NewRow {
                values: values(json!({"Name": "Traced", "Assignee": [dee.to_string()]})),
                ..NewRow::default()
            },
        )
        .instrument(request)
        .await
        .expect("a row");
    let queued: i32 = sqlx::query_scalar("SELECT max_attempts FROM jobs WHERE kind = $1")
        .bind(DELIVERY_KIND)
        .fetch_one(&pool)
        .await
        .expect("queued");
    assert_eq!(queued, MAX_ATTEMPTS);

    let worker = Worker::new(
        pool.clone(),
        WorkerConfig {
            poll_interval: Duration::from_millis(20),
            backoff: Backoff {
                base: Duration::from_millis(5),
                max: Duration::from_millis(20),
            },
            ..WorkerConfig::default()
        },
    )
    .handler(DeliveryJob::new(t.webhooks.clone()))
    .span(run_span)
    .hook(RedHook::new())
    .start()
    .await
    .expect("starts");
    let deadline = Instant::now() + Duration::from_secs(20);
    while t.kept.sent().is_empty() {
        assert!(Instant::now() < deadline, "nothing was sent");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    worker.stop().await;

    let sent = t.kept.sent();
    let header = |name: &str| {
        sent[0]
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    let traceparent = header("traceparent").expect("a traceparent");
    let trace = |traceparent: &str| traceparent.split('-').nth(1).map(str::to_owned);
    assert_eq!(trace(&traceparent), trace(&caused_by));
    assert_ne!(traceparent, caused_by, "sent from a span of its own");
    assert_eq!(
        header("neon-webhook-event").as_deref(),
        Some("row.assigned")
    );
    assert!(header("neon-webhook-signature").is_some_and(|value| value.starts_with("v1=")));
    let body: Value = serde_json::from_slice(&sent[0].body).expect("JSON");
    assert_eq!(body["identity"]["id"], dee.to_string());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn deliveries_that_fail_in_the_server_are_logged_and_counted(pool: PgPool) {
    let t = team(pool.clone()).await;
    let dee = t.dee.user.id;
    // A service whose key has the same name as the one the secret was sealed with, and
    // other bytes: the secret does not open, and nothing is sent.
    let other =
        Keyring::parse(&[format!("k1:{}", STANDARD.encode([8u8; 32]))]).expect("a valid key");
    let (_, feed) = access_feed();
    let sync =
        SyncService::builder(t.h.pool.clone(), t.h.pages.clone(), t.h.auth.clone(), feed).build();
    let broken = WebhookService::builder(
        t.h.pool.clone(),
        Arc::clone(&t.h.settings),
        Services {
            workspaces: t.h.workspaces.clone(),
            agents: t.h.agents.clone(),
            pages: t.h.pages.clone(),
            databases: t.h.databases.clone(),
            sync,
        },
        Arc::clone(&t.kept) as Arc<dyn WebhookTransport>,
    )
    .envelope(Envelope::new(other))
    .clock(t.h.time.clock())
    .build();
    t.h.databases
        .create_row(
            &t.ada,
            t.db,
            NewRow {
                values: values(json!({"Name": "Sealed", "Assignee": [dee.to_string()]})),
                ..NewRow::default()
            },
        )
        .await
        .expect("a row");
    let worker = Worker::new(
        pool.clone(),
        WorkerConfig {
            poll_interval: Duration::from_millis(20),
            backoff: Backoff {
                base: Duration::from_millis(5),
                max: Duration::from_millis(20),
            },
            ..WorkerConfig::default()
        },
    )
    .handler(DeliveryJob::new(broken))
    .start()
    .await
    .expect("starts");

    let max = usize::try_from(MAX_ATTEMPTS).expect("a count");
    let deadline = Instant::now() + Duration::from_secs(20);
    let log = loop {
        let log = t
            .webhooks
            .webhook_attempts(&t.dee, t.workspace, t.hook)
            .await
            .expect("the log");
        if log.first().is_some_and(|last| last.outcome == "failed") {
            break log;
        }
        assert!(Instant::now() < deadline, "the log stayed {log:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    worker.stop().await;

    assert_eq!(log.len(), max);
    assert!(
        log.iter()
            .all(|attempt| attempt.error_kind.as_deref() == Some("internal"))
    );
    assert!(log[1..].iter().all(|attempt| attempt.outcome == "retrying"));
    assert_eq!(log[0].attempt, MAX_ATTEMPTS);
    assert!(t.kept.sent().is_empty());
    let read = t
        .webhooks
        .webhook(&t.dee, t.workspace, t.hook)
        .await
        .expect("the webhook");
    assert_eq!(read.failures, 1);
    assert_eq!(read.disabled_at, None);
}

/// The principal an incoming webhook's integration acts as, as the webhook makes it.
async fn integration(t: &Team, id: Uuid) -> Principal {
    let (user, _) = neon_store::users::identity(&t.h.pool, id)
        .await
        .expect("read")
        .expect("an identity");
    Principal {
        user,
        credential: Credential::Token {
            id: Uuid::now_v7(),
            name: "hook".to_owned(),
            scopes: vec![Scope::Read, Scope::Write],
            workspace_id: Some(t.workspace),
        },
    }
}

fn request(token: &str) -> IncomingRequest<'_> {
    IncomingRequest {
        token,
        client_ip: "203.0.113.7",
        json: true,
        signature: None,
        timestamp: None,
    }
}

async fn body(json: Value) -> Result<Bytes, BodyError> {
    Ok(Bytes::from(json.to_string()))
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn an_integration_edits_its_target_alone_and_only_while_its_owner_may(pool: PgPool) {
    let t = team(pool).await;
    // A private teamspace of Ada's, its database the webhook's target.
    let ops =
        t.h.workspaces
            .create_teamspace(
                &t.ada,
                t.workspace,
                NewTeamspace {
                    name: "Ops".to_owned(),
                    icon: None,
                    hue: None,
                    access: TeamspaceAccess::Private,
                },
            )
            .await
            .expect("made")
            .teamspace
            .id;
    let incidents =
        t.h.databases
            .create_database(
                &t.ada,
                t.workspace,
                Location::Teamspace(ops),
                Placement::Last,
                NewDatabase {
                    title: "Incidents".to_owned(),
                    icon: None,
                    inline: false,
                },
            )
            .await
            .expect("made")
            .page
            .page
            .id;
    let created = t
        .webhooks
        .create_incoming(
            &t.ada,
            t.workspace,
            NewIncomingWebhook {
                name: "Pager".to_owned(),
                target_id: incidents,
                signed: false,
            },
        )
        .await
        .expect("made");
    let token = created.token.expose().to_owned();
    let page = json!({"version": 1, "action": "create_row", "values": {"Name": "Disk full"}});

    let accepted = t
        .webhooks
        .receive_incoming(request(&token), body(page.clone()))
        .await
        .expect("taken");
    assert_eq!(accepted.action, "create_row");

    // Its integration edits nothing else: Tasks, in General, it may only view.
    let acting = integration(&t, created.hook.integration.id).await;
    let elsewhere =
        t.h.databases
            .create_row(
                &acting,
                t.db,
                NewRow {
                    values: values(json!({"Name": "Not mine"})),
                    ..NewRow::default()
                },
            )
            .await;
    assert_eq!(
        elsewhere.expect_err("refused").to_string(),
        "you may view this page, not edit it"
    );

    // Its owner leaves Ops: the integration's own share is not enough without theirs.
    t.h.workspaces
        .leave_teamspace(&t.ada, t.workspace, ops)
        .await
        .expect("left");
    let refused = t
        .webhooks
        .receive_incoming(request(&token), body(page))
        .await
        .expect_err("refused");
    assert_eq!((refused.status(), refused.code()), (404, "not_found"));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn rows_an_incoming_webhook_makes_are_told_to_outgoing_ones(pool: PgPool) {
    let t = team(pool).await;
    let created = t
        .webhooks
        .create_incoming(
            &t.ada,
            t.workspace,
            NewIncomingWebhook {
                name: "Forms".to_owned(),
                target_id: t.db,
                signed: false,
            },
        )
        .await
        .expect("made");
    let token = created.token.expose().to_owned();
    let dee = t.dee.user.id.to_string();

    t.webhooks
        .receive_incoming(
            request(&token),
            body(json!({"version": 1, "action": "create_row", "values": {"Name": "Call back", "Assignee": [dee]}})),
        )
        .await
        .expect("taken");

    assert_eq!(t.take().await, [("row.assigned".to_owned(), t.dee.user.id)]);
}
