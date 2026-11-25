//! Webhooks: outgoing ones, which tell a URL of what happens to the rows an identity is
//! assigned to, and their deliveries; and incoming ones, which act as an integration on a
//! database or a page with what is posted to them: see [`incoming`].
//!
//! - **Registering.** An outgoing webhook belongs to one identity of a workspace, a person,
//!   an agent or an integration, and names the [`EventType`]s it wants. The workspace's
//!   owners and admins register them for anyone in it; everyone else but guests for
//!   themselves and for their own agents and integrations, as the rules say. Its URL is
//!   checked when it is registered or changed, and again at every delivery, through the
//!   [`WebhookTransport`] port, which refuses addresses of the server's network unless
//!   `webhooks.allow_private` allows them.
//! - **Secrets.** The server makes each webhook's signing secret, shows it once, and keeps
//!   it sealed with the instance's key. Rotating it makes a new one; the old one signs
//!   deliveries beside it for [`ROTATION_OVERLAP`], so receivers can move over.
//! - **Events** are queued with the change that makes them, on its transaction: see
//!   [`events`]. Each delivery is a job of [`DELIVERY_KIND`], tried up to [`MAX_ATTEMPTS`]
//!   times with the job queue's backoff. Before each attempt, the identity must still see
//!   the row, as the rules say then; a delivery whose identity no longer does, or whose
//!   webhook is off, is skipped. After [`DISABLE_AFTER`] deliveries in a row are given up
//!   on, the webhook turns itself off, saying why.
//! - **The log** keeps each webhook's last attempts: how each went, the answer's status, how
//!   long it took and a short excerpt of the answer, never what was sent.
//!
//! Every public use case runs in a span named `webhooks <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`]: a refusal is `ok`, only failures of the server are `error`. No
//! secret, URL, payload or answer reaches a log line or a span.

mod delivery;
mod error;
pub(crate) mod events;
pub mod incoming;
mod outgoing;
pub mod payload;
pub mod signature;
pub mod transport;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neon_domain::auth::Principal;
use neon_domain::webhooks::EventType;
use neon_domain::workspaces::{Action, Person, Resource, WebhookSubject};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use tracing::{Instrument as _, info_span};
use uuid::Uuid;

use crate::clock::Clock;
use crate::crypto::Envelope;
use crate::databases::DatabaseService;
use crate::pages::PageService;
use crate::ratelimit::{MemoryLimiter, RateLimiter};
use crate::settings::{SettingsService, webhooks as keys};
use crate::standing::Standing;
use crate::sync::SyncService;
use crate::workspaces::{AgentService, WorkspaceService};

pub use delivery::{DELIVERY_HEADER, DeliveryJob, EVENT_HEADER};
pub use error::{IncomingError, Refusal, WebhookError};
pub use incoming::{
    Accepted, BodyError, CreatedIncoming, IncomingRequest, IncomingUpdate, IncomingWebhook,
    Integration, LoggedRequest, NewIncomingWebhook, Target,
};
pub use outgoing::{Creator, DeliveryAttempt, NewWebhook, OutgoingWebhook, WebhookUpdate};
pub use payload::TargetKind;
pub use transport::WebhookTransport;

/// RED subsystem of the webhook use cases.
pub const SUBSYSTEM: &str = "webhooks";

/// The job kind of a delivery.
pub const DELIVERY_KIND: &str = "webhooks.deliver";

/// How many times a delivery is tried. With the job queue's backoff, from 10 seconds
/// doubling to an hour, half of each delay spread by jitter, the last try comes between
/// 1.7 and 3.4 hours after the first, as a test checks.
pub const MAX_ATTEMPTS: i32 = 12;

/// How many deliveries in a row may be given up on before the webhook turns itself off.
pub const DISABLE_AFTER: i32 = 5;

/// How long a rotated secret still signs deliveries beside the new one, and verifies an
/// incoming webhook's requests.
pub const ROTATION_OVERLAP: Duration = Duration::from_secs(24 * 3600);

/// The largest body an incoming webhook reads.
pub const MAX_INCOMING_BODY: usize = 256 * 1024;

const NO_WORKSPACE: &str = "no such workspace";
const NO_WEBHOOK: &str = "no such webhook";

/// The webhook use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct WebhookService {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    settings: Arc<SettingsService>,
    workspaces: WorkspaceService,
    agents: AgentService,
    pages: PageService,
    databases: DatabaseService,
    sync: SyncService,
    transport: Arc<dyn WebhookTransport>,
    limiter: Arc<dyn RateLimiter>,
    envelope: Envelope,
    clock: Clock,
    red: Red,
}

/// The services webhooks work through: the workspaces and agents they belong to, the pages
/// and rows their events are about, and the services incoming ones act through.
#[derive(Clone)]
pub struct Services {
    pub workspaces: WorkspaceService,
    pub agents: AgentService,
    pub pages: PageService,
    pub databases: DatabaseService,
    pub sync: SyncService,
}

/// Puts a [`WebhookService`] together. Without being told otherwise it seals nothing, so it
/// makes no webhook with a secret, limits incoming requests in memory, and reads the
/// system clock.
pub struct WebhookServiceBuilder {
    pool: PgPool,
    settings: Arc<SettingsService>,
    services: Services,
    transport: Arc<dyn WebhookTransport>,
    limiter: Option<Arc<dyn RateLimiter>>,
    envelope: Envelope,
    clock: Clock,
}

impl WebhookServiceBuilder {
    /// Where incoming requests' rates are counted, as other use cases count theirs.
    pub fn limiter(mut self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Seals webhooks' secrets: without it, none is made.
    pub fn envelope(mut self, envelope: Envelope) -> Self {
        self.envelope = envelope;
        self
    }

    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Call it after telemetry is installed, as it creates the RED metrics.
    pub fn build(self) -> WebhookService {
        let Services {
            workspaces,
            agents,
            pages,
            databases,
            sync,
        } = self.services;
        WebhookService {
            inner: Arc::new(Inner {
                pool: self.pool,
                settings: self.settings,
                workspaces,
                agents,
                pages,
                databases,
                sync,
                transport: self.transport,
                limiter: self
                    .limiter
                    .unwrap_or_else(|| Arc::new(MemoryLimiter::new())),
                envelope: self.envelope,
                clock: self.clock,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }
}

impl WebhookService {
    /// The webhooks of the workspaces `services` serve, outgoing ones sent through
    /// `transport`.
    pub fn builder(
        pool: PgPool,
        settings: Arc<SettingsService>,
        services: Services,
        transport: Arc<dyn WebhookTransport>,
    ) -> WebhookServiceBuilder {
        WebhookServiceBuilder {
            pool,
            settings,
            services,
            transport,
            limiter: None,
            envelope: Envelope::default(),
            clock: Clock::system(),
        }
    }

    fn pool(&self) -> &PgPool {
        &self.inner.pool
    }

    fn now(&self) -> OffsetDateTime {
        self.inner.clock.now()
    }

    /// Whether webhooks may reach a private network's addresses, as the setting says now.
    fn allow_private(&self) -> bool {
        *self.inner.settings.current().get(keys::ALLOW_PRIVATE)
    }

    /// Runs a use case in its span and records its outcome; see the module docs.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, WebhookError>>,
    ) -> Result<T, WebhookError> {
        let span = info_span!(
            "webhooks",
            otel.name = %format!("webhooks {operation}"),
            webhooks.operation = operation,
        );
        let guard = self.inner.red.start(operation);
        let result = work.instrument(span.clone()).await;
        match &result {
            Ok(_) => guard.ok(),
            Err(error) if error.is_expected() => {
                mark_expected(&span, error.code());
                guard.ok();
            }
            Err(error) => {
                record_unexpected(&span, error);
                guard.err();
            }
        }
        result
    }

    /// Where the principal stands in a workspace they may know of.
    async fn standing(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        workspace_id: Uuid,
    ) -> Result<Standing, WebhookError> {
        let workspace = neon_store::workspaces::get(&mut *conn, workspace_id)
            .await?
            .ok_or(WebhookError::NotFound(NO_WORKSPACE))?;
        let standing = Standing::read(&mut *conn, principal, workspace_id).await?;
        standing
            .authorize_same(
                Action::ViewWorkspace,
                Resource::Workspace {
                    members_can_create_teamspaces: workspace.members_can_create_teamspaces,
                },
            )
            .map_err(|denied| WebhookError::denied(denied, NO_WORKSPACE))?;
        Ok(standing)
    }
}

/// How `who` stands to `identity`, whose webhooks are asked about.
fn subject_of(who: Uuid, identity: &Person) -> WebhookSubject {
    if identity.id == who {
        WebhookSubject::Itself
    } else if identity.owner.as_ref().is_some_and(|owner| owner.id == who) {
        WebhookSubject::OwnDelegate
    } else {
        WebhookSubject::Other
    }
}

/// Asks the rules about an action on an identity's webhooks, as the principal stands to the
/// identity, and as an agent's owner does.
fn check(
    standing: &Standing,
    principal: &Principal,
    identity: &Person,
    action: Action,
) -> Result<(), WebhookError> {
    let mine = subject_of(principal.user.id, identity);
    let theirs = standing
        .owner_id()
        .map_or(mine, |owner| subject_of(owner, identity));
    standing
        .authorize(
            action,
            Resource::Webhook { subject: mine },
            Resource::Webhook { subject: theirs },
        )
        .map_err(|denied| WebhookError::denied(denied, NO_WEBHOOK))
}

/// The events of those stored that this server knows, in order.
fn known_events(stored: &[String]) -> Vec<EventType> {
    let mut events: Vec<EventType> = stored
        .iter()
        .filter_map(|text| EventType::parse(text))
        .collect();
    events.sort();
    events.dedup();
    events
}

/// What a webhook's secrets are sealed with: its id, so a sealed secret copied to another
/// webhook does not open there.
fn aad(id: Uuid) -> Vec<u8> {
    format!("webhook:{id}").into_bytes()
}

#[cfg(test)]
mod tests {
    use neon_domain::agents::AgentOwner;
    use neon_domain::auth::UserKind;

    use super::*;

    fn person(id: u128, owner: Option<u128>) -> Person {
        Person {
            id: Uuid::from_u128(id),
            kind: if owner.is_some() {
                UserKind::Agent
            } else {
                UserKind::Human
            },
            display_name: "x".into(),
            email: None,
            hue_override: None,
            owner: owner.map(|id| AgentOwner {
                id: Uuid::from_u128(id),
                display_name: "o".into(),
                hue_override: None,
            }),
        }
    }

    #[test]
    fn whose_webhooks_they_are_follows_identity_and_ownership() {
        let me = Uuid::from_u128(1);
        assert_eq!(subject_of(me, &person(1, None)), WebhookSubject::Itself);
        assert_eq!(
            subject_of(me, &person(2, Some(1))),
            WebhookSubject::OwnDelegate
        );
        assert_eq!(subject_of(me, &person(3, Some(4))), WebhookSubject::Other);
        assert_eq!(subject_of(me, &person(5, None)), WebhookSubject::Other);
    }

    #[test]
    fn deliveries_are_retried_for_a_bounded_time() {
        let backoff = crate::jobs::WorkerConfig::default().backoff;
        let waited = |jitter: f64| -> Duration {
            (1..MAX_ATTEMPTS)
                .map(|attempt| backoff.delay(u32::try_from(attempt).expect("an attempt"), jitter))
                .sum()
        };

        // 10 s, 20 s, … 2560 s, then the hour it stops at, twice; each at least its half.
        assert_eq!(waited(1.0), Duration::from_secs(12_310));
        assert_eq!(waited(0.0), Duration::from_secs(6_155));
        assert!(waited(0.5) > waited(0.0) && waited(0.5) < waited(1.0));
    }

    #[test]
    fn unknown_stored_events_are_left_out() {
        let stored = [
            "row.status_changed".to_owned(),
            "row.mentioned".to_owned(),
            "row.assigned".to_owned(),
        ];
        assert_eq!(
            known_events(&stored),
            [EventType::Assigned, EventType::StatusChanged]
        );
        assert_ne!(aad(Uuid::from_u128(1)), aad(Uuid::from_u128(2)));
    }
}
