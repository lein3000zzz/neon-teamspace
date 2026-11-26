//! Delivering events: the job that tries each delivery, what it sends, signed, and the log
//! of how each attempt went.
//!
//! The payload, version 1, is a JSON object:
//!
//! | Field | What it holds |
//! |---|---|
//! | `version` | `1` |
//! | `id` | The event's id, the same on every attempt and for each of the identity's webhooks. |
//! | `type` | `row.assigned`, `row.status_changed`, or `webhook.test` |
//! | `occurred_at` | When the change was made, RFC 3339. |
//! | `workspace` | `{"id"}` |
//! | `identity` | Whose event it is: `{"id", "kind", "name"}`, `kind` being `human`, `agent` or `integration`. |
//! | `row` | `{"id", "unique_id", "title", "database", "status"}` as it is when sent: `database` is `{"id", "title"}`, `status` `{"id", "name"}` or null; null for a test event. |
//! | `property` | The property the event is about, `{"id", "name", "type"}`, `type` being `person` or `status`; null for a test event. |
//! | `change` | For `row.status_changed` alone: `{"from", "to"}`, each `{"id", "name"}` or null. |
//!
//! Names that come from the database, its title, a property's and an option's, are null for
//! an identity who sees the row only through its own share and not its database, as the row
//! then shows them no database either; and so for whoever set the webhook's URL.
//!
//! A delivery is sent only while its identity may see the row, and so may whoever set the
//! webhook's URL, as the rules say then. Otherwise it is skipped; and when whoever set the URL
//! may no longer register webhooks for the identity, the webhook turns off, saying why.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use neon_domain::auth::{Credential, Principal, Scope, User};
use neon_domain::databases::{PropertyKind, Schema, unique_id};
use neon_domain::pages::PageKind;
use neon_domain::webhooks::{EventType, PAYLOAD_VERSION, TEST_EVENT, check_url};
use neon_domain::workspaces::Action;
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::propagation::current_traceparent;
use neon_store::jobs::Job;
use neon_store::webhooks::{self as repo, Attempt, GaveUp, Listed, Webhook};
use neon_store::{databases, pages, users};
use serde_json::{Value as Json, json};
use sqlx::PgConnection;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::{Instrument as _, info, info_span, warn};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::events::{Choice, Delivery, Event, status_property};
use super::outgoing::{DeliveryAttempt, url_setter};
use super::signature::{self, SIGNATURE_HEADER, TIMESTAMP_HEADER};
use super::transport::{Answer, Outgoing, SendError};
use super::{DELIVERY_KIND, DISABLE_AFTER, WebhookError, WebhookService, Whose, aad};
use crate::jobs::{Handler, JobError};
use crate::pages::{Context, PageError};

/// The header with the event's type.
pub const EVENT_HEADER: &str = "Neon-Webhook-Event";
/// The header with the delivery's id, the same on each of its attempts.
pub const DELIVERY_HEADER: &str = "Neon-Webhook-Delivery";

/// Runs the deliveries of webhooks: one attempt per run, as the module docs say. A run that
/// fails is retried by the queue; its jobs are not kept once they finish, the webhook's log
/// keeping what matters of them.
pub struct DeliveryJob {
    service: WebhookService,
}

impl DeliveryJob {
    pub fn new(service: WebhookService) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Handler for DeliveryJob {
    fn kind(&self) -> &'static str {
        DELIVERY_KIND
    }

    fn keeps_finished(&self) -> bool {
        false
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        self.service.deliver(job).await
    }
}

/// How an attempt went, as the log keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Delivered,
    Retrying,
    Failed,
    Skipped,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Retrying => "retrying",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// What an attempt that was sent came to.
struct Sent {
    status: Option<u16>,
    excerpt: Option<String>,
    latency: Duration,
    /// Why it did not go through, if it did not.
    failure: Option<Failure>,
}

/// Why an attempt that was sent did not go through.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Failure {
    Send(SendError),
    Redirect(u16),
    Status(u16),
}

impl Failure {
    fn kind(&self) -> &'static str {
        match self {
            Self::Send(error) => error.kind(),
            Self::Redirect(_) => "redirect",
            Self::Status(_) => "http_status",
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Send(error) => error.to_string(),
            Self::Redirect(status) => {
                format!("the receiver answered {status}, a redirect, which webhooks do not follow")
            }
            Self::Status(status) => format!("the receiver answered {status}"),
        }
    }
}

/// Whether a delivery may be sent, as the rules say when it is tried.
enum Reach {
    /// Yes: about the row, to the identity, as it works now.
    Row(Box<(User, RowFacts)>),
    /// No: the row is out of the identity's reach, or of whoever set the URL.
    Hidden,
    /// No, nor any other: whoever set the URL may no longer register it, saying why.
    Lapsed(String),
}

/// What of a row a payload shows, as it is when sent.
struct RowFacts {
    title: String,
    unique_id: String,
    database_id: Uuid,
    /// Its database's title, its property names and its options' names, for an identity
    /// who sees the database.
    schema: Option<(String, Schema)>,
    cells: Json,
}

impl WebhookService {
    /// Makes one attempt at a delivery; see the module docs.
    pub(crate) async fn deliver(&self, job: &Job) -> Result<(), JobError> {
        let delivery: Delivery = serde_json::from_value(job.payload.clone())
            .map_err(|_| JobError::permanent("the payload is no webhook delivery"))?;
        let span = info_span!(
            "webhooks",
            otel.name = "webhooks deliver",
            webhooks.operation = "deliver",
            webhook.id = %delivery.webhook_id,
            webhook.event = %delivery.event.kind,
            webhook.attempt = job.attempt,
        );
        let guard = self.inner.red.start("deliver");
        let result = self.attempt(job, &delivery).instrument(span.clone()).await;
        match result {
            Ok(outcome) => {
                guard.ok();
                outcome
            }
            Err(error) => {
                if error.is_expected() {
                    mark_expected(&span, error.code());
                    guard.ok();
                } else {
                    record_unexpected(&span, &error);
                    guard.err();
                }
                Err(JobError::retry(error.code()))
            }
        }
    }

    /// The attempt, answering how its run ends: done, or to be tried again.
    async fn attempt(
        &self,
        job: &Job,
        delivery: &Delivery,
    ) -> Result<Result<(), JobError>, WebhookError> {
        let event = &delivery.event;
        let record = |outcome, sent: Option<&Sent>, error_kind: Option<&str>| Attempt {
            id: Uuid::now_v7(),
            webhook_id: delivery.webhook_id,
            delivery_id: Some(job.id),
            event_id: event.id,
            event_type: event.kind.clone(),
            attempt: job.attempt,
            outcome: Outcome::as_str(outcome).to_owned(),
            status_code: sent.and_then(|sent| sent.status).map(i32::from),
            latency_ms: sent.map(|sent| millis(sent.latency)),
            error_kind: error_kind.map(str::to_owned),
            excerpt: sent.and_then(|sent| sent.excerpt.clone()),
            at: self.now(),
        };
        let Some(webhook) = repo::by_id(self.pool(), delivery.webhook_id).await? else {
            // Deleted since: nothing to tell, nowhere to log it.
            return Ok(Ok(()));
        };
        if webhook.disabled_at.is_some() {
            self.log(&record(Outcome::Skipped, None, Some("disabled")))
                .await?;
            return Ok(Ok(()));
        }
        let (identity, facts) = match self.reach(&webhook, event).await? {
            Reach::Row(found) => *found,
            Reach::Hidden => {
                info!(webhook_id = %webhook.id, "webhook delivery skipped: the row is out of reach");
                self.log(&record(Outcome::Skipped, None, Some("no_access")))
                    .await?;
                return Ok(Ok(()));
            }
            Reach::Lapsed(reason) => {
                repo::turn_off(self.pool(), webhook.id, &reason, self.now()).await?;
                warn!(
                    webhook_id = %webhook.id,
                    "webhook turned off: whoever set its URL may no longer register it"
                );
                self.log(&record(Outcome::Skipped, None, Some("lapsed")))
                    .await?;
                return Ok(Ok(()));
            }
        };
        let body = row_payload(event, &identity, &facts);
        let sent = self.send(&webhook, &event.kind, job.id, &body).await?;
        // What the attempt did to the webhook is saved before the log tells of it, so the log
        // never shows an attempt whose effect is not there yet.
        match &sent.failure {
            None => {
                repo::delivered(self.pool(), webhook.id).await?;
                self.log(&record(Outcome::Delivered, Some(&sent), None))
                    .await?;
                info!(
                    webhook_id = %webhook.id,
                    status = sent.status,
                    latency_ms = millis(sent.latency),
                    "webhook delivered"
                );
                Ok(Ok(()))
            }
            Some(failure) => {
                let last = job.attempt >= job.max_attempts;
                let outcome = if last {
                    Outcome::Failed
                } else {
                    Outcome::Retrying
                };
                if last {
                    self.gave_up(&webhook, failure).await?;
                }
                self.log(&record(outcome, Some(&sent), Some(failure.kind())))
                    .await?;
                warn!(
                    webhook_id = %webhook.id,
                    error_kind = failure.kind(),
                    status = sent.status,
                    attempt = job.attempt,
                    last,
                    "webhook delivery failed"
                );
                Ok(Err(JobError::retry(failure.kind())))
            }
        }
    }

    /// One more delivery given up on, which turns the webhook off after [`DISABLE_AFTER`]
    /// in a row.
    async fn gave_up(&self, webhook: &Webhook, failure: &Failure) -> Result<(), WebhookError> {
        let reason = format!(
            "turned off after {DISABLE_AFTER} deliveries in a row failed; the last: {}",
            failure.describe()
        );
        if let Some(GaveUp {
            failures,
            disabled: true,
        }) = repo::gave_up(self.pool(), webhook.id, DISABLE_AFTER, &reason, self.now()).await?
        {
            warn!(
                webhook_id = %webhook.id,
                failures,
                "webhook turned off after deliveries kept failing"
            );
        }
        Ok(())
    }

    /// Sends a test event to a webhook, and logs and answers the attempt.
    pub(super) async fn send_test(&self, listed: &Listed) -> Result<DeliveryAttempt, WebhookError> {
        let webhook = &listed.webhook;
        let event_id = Uuid::now_v7();
        let now = self.now();
        let body = json!({
            "version": PAYLOAD_VERSION,
            "id": event_id.to_string(),
            "type": TEST_EVENT,
            "occurred_at": rfc3339(now),
            "workspace": {"id": webhook.workspace_id.to_string()},
            "identity": {
                "id": listed.identity.id.to_string(),
                "kind": listed.identity.kind.as_str(),
                "name": listed.identity.display_name,
            },
            "row": null,
            "property": null,
        });
        let sent = self
            .send(webhook, TEST_EVENT, Uuid::now_v7(), &body)
            .await?;
        let (outcome, error_kind) = match &sent.failure {
            None => (Outcome::Delivered, None),
            Some(failure) => (Outcome::Failed, Some(failure.kind().to_owned())),
        };
        let attempt = Attempt {
            id: Uuid::now_v7(),
            webhook_id: webhook.id,
            delivery_id: None,
            event_id,
            event_type: TEST_EVENT.to_owned(),
            attempt: 1,
            outcome: outcome.as_str().to_owned(),
            status_code: sent.status.map(i32::from),
            latency_ms: Some(millis(sent.latency)),
            error_kind,
            excerpt: sent.excerpt,
            at: self.now(),
        };
        self.log(&attempt).await?;
        Ok(attempt.into())
    }

    /// Signs and sends a payload to a webhook's URL, checked again now.
    async fn send(
        &self,
        webhook: &Webhook,
        event_type: &str,
        delivery_id: Uuid,
        body: &Json,
    ) -> Result<Sent, WebhookError> {
        let started = Instant::now();
        let url = check_url(&webhook.url)
            .map_err(|error| WebhookError::Internal(format!("a stored webhook URL: {error}")))?;
        let body = Bytes::from(
            serde_json::to_vec(body)
                .map_err(|error| WebhookError::Internal(format!("a payload: {error}")))?,
        );
        let secrets = self.secrets(webhook)?;
        let timestamp = self.now().unix_timestamp();
        let mut headers = vec![
            ("Content-Type", "application/json".to_owned()),
            (EVENT_HEADER, event_type.to_owned()),
            (DELIVERY_HEADER, delivery_id.to_string()),
            (TIMESTAMP_HEADER, timestamp.to_string()),
            (
                SIGNATURE_HEADER,
                signature::header(&secrets, timestamp, &body),
            ),
        ];
        if let Some(traceparent) = current_traceparent() {
            headers.push(("traceparent", traceparent));
        }
        let request = Outgoing { url, headers, body };
        let answer = self
            .inner
            .transport
            .send(request, self.allow_private())
            .await;
        let latency = started.elapsed();
        Ok(match answer {
            Ok(Answer { status, excerpt }) => Sent {
                status: Some(status),
                excerpt,
                latency,
                failure: match status {
                    200..=299 => None,
                    300..=399 => Some(Failure::Redirect(status)),
                    _ => Some(Failure::Status(status)),
                },
            },
            Err(error) => Sent {
                status: None,
                excerpt: None,
                latency,
                failure: Some(Failure::Send(error)),
            },
        })
    }

    /// The secrets that sign a webhook's deliveries now: its own, and the one before the
    /// last rotation while it still holds.
    fn secrets(&self, webhook: &Webhook) -> Result<Vec<Zeroizing<Vec<u8>>>, WebhookError> {
        let aad = aad(webhook.id);
        let open = |sealed: &[u8]| {
            self.inner.envelope.open(sealed, &aad).map_err(|error| {
                WebhookError::Internal(format!("a webhook's secret does not open: {error}"))
            })
        };
        let mut secrets = vec![open(&webhook.secret)?];
        if let (Some(previous), Some(until)) =
            (&webhook.previous_secret, webhook.previous_expires_at)
            && until > self.now()
        {
            secrets.push(open(previous)?);
        }
        Ok(secrets)
    }

    async fn log(&self, attempt: &Attempt) -> Result<(), WebhookError> {
        let mut conn = self.pool().acquire().await?;
        repo::record_attempt(&mut conn, attempt).await?;
        Ok(())
    }

    /// Whether a delivery may be sent now, as the module docs say: the row an event is
    /// about, out of the trash, when both the webhook's identity and whoever set its URL may
    /// see it, with the identity as it works now.
    async fn reach(&self, webhook: &Webhook, event: &Event) -> Result<Reach, WebhookError> {
        let mut conn = self.pool().acquire().await?;
        let Some((user, standing)) = users::identity(&mut *conn, webhook.user_id).await? else {
            return Ok(Reach::Hidden);
        };
        let agents_on = self.inner.workspaces.agents_on();
        let works = if user.kind.is_human() {
            !user.is_disabled()
        } else {
            user.workspace_id == Some(webhook.workspace_id)
                && standing.status(user.kind, agents_on).works()
        };
        if !works {
            return Ok(Reach::Hidden);
        }
        let setter = url_setter(&mut conn, webhook, Whose::from(&user), &user.display_name);
        let setter = match setter.await? {
            Ok(setter) => setter,
            Err(lapsed) => return Ok(Reach::Lapsed(lapsed.reason())),
        };
        let principal = Principal {
            user,
            credential: Credential::Token {
                id: webhook.id,
                name: "webhook".to_owned(),
                scopes: vec![Scope::Read],
                workspace_id: Some(webhook.workspace_id),
            },
        };
        let Some(seen) = self.sees(&mut conn, &principal, webhook, event).await? else {
            return Ok(Reach::Hidden);
        };
        let mut parent_seen = seen.parent_seen();
        if setter.user.id != principal.user.id {
            let Some(theirs) = self.sees(&mut conn, &setter, webhook, event).await? else {
                return Ok(Reach::Hidden);
            };
            parent_seen &= theirs.parent_seen();
        }
        let page = seen.page();
        let Some(data) = databases::row(&mut *conn, page.id).await? else {
            return Ok(Reach::Hidden);
        };
        let schema = Schema {
            properties: databases::properties(&mut *conn, data.database_id)
                .await?
                .into_iter()
                .map(|stored| stored.property)
                .collect(),
        };
        let prefix = schema.unique_id().and_then(|property| property.ty.prefix());
        let facts_schema = if parent_seen {
            let title = pages::get(&mut *conn, data.database_id)
                .await?
                .map(|database| database.title)
                .unwrap_or_default();
            Some((title, schema.clone()))
        } else {
            None
        };
        let facts = RowFacts {
            title: page.title.clone(),
            unique_id: unique_id(prefix, data.number),
            database_id: data.database_id,
            schema: facts_schema,
            cells: data.cells,
        };
        Ok(Reach::Row(Box::new((principal.user, facts))))
    }

    /// The row an event is about, as `principal` sees it, when they may: a row of the
    /// webhook's workspace, out of the trash.
    async fn sees(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        webhook: &Webhook,
        event: &Event,
    ) -> Result<Option<Context>, WebhookError> {
        let context = match self
            .inner
            .pages
            .context(&mut *conn, principal, event.row_id)
            .await
        {
            Ok(context) => context,
            Err(PageError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(WebhookError::Internal(error.to_string())),
        };
        let page = context.page();
        let seen = page.kind == PageKind::Row
            && page.workspace_id == webhook.workspace_id
            && !page.is_trashed()
            && context.check(Action::ViewPage).is_ok();
        Ok(seen.then_some(context))
    }
}

/// A row event's payload; see the module docs.
fn row_payload(event: &Event, identity: &User, facts: &RowFacts) -> Json {
    let schema = facts.schema.as_ref().map(|(_, schema)| schema);
    let choice = |choice: &Option<Choice>| {
        choice.as_ref().map_or(Json::Null, |choice| {
            json!({
                "id": choice.id.to_string(),
                "name": schema.map(|_| choice.name.clone()),
            })
        })
    };
    let property = schema.and_then(|schema| schema.get(event.property_id));
    let kind = if event.kind == EventType::StatusChanged.as_str() {
        PropertyKind::Status
    } else {
        PropertyKind::Person
    };
    let status = schema
        .and_then(status_property)
        .and_then(|property| current_status(property, &facts.cells));
    let mut payload = json!({
        "version": PAYLOAD_VERSION,
        "id": event.id.to_string(),
        "type": event.kind,
        "occurred_at": rfc3339(event.occurred_at()),
        "workspace": {"id": event.workspace_id.to_string()},
        "identity": {
            "id": identity.id.to_string(),
            "kind": identity.kind.as_str(),
            "name": identity.display_name,
        },
        "row": {
            "id": event.row_id.to_string(),
            "unique_id": facts.unique_id,
            "title": facts.title,
            "database": facts.schema.as_ref().map(|(title, _)| json!({
                "id": facts.database_id.to_string(),
                "title": title,
            })),
            "status": choice(&status),
        },
        "property": {
            "id": event.property_id.to_string(),
            "name": property.map(|property| property.name.clone()),
            "type": kind.as_str(),
        },
    });
    if event.kind == EventType::StatusChanged.as_str() {
        payload["change"] = json!({"from": choice(&event.from), "to": choice(&event.to)});
    }
    payload
}

/// The option a row's status holds now.
fn current_status(property: &neon_domain::databases::Property, cells: &Json) -> Option<Choice> {
    let id = cells
        .get(property.id.to_string())
        .and_then(Json::as_str)
        .and_then(|text| Uuid::try_parse(text).ok())?;
    let option = property
        .ty
        .options()
        .iter()
        .find(|option| option.id == id)?;
    Some(Choice {
        id,
        name: option.name.clone(),
    })
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_default()
}

fn millis(duration: Duration) -> i32 {
    i32::try_from(duration.as_millis()).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use neon_domain::auth::{Role, UserKind};
    use neon_domain::databases::{Choice as StatusOption, Property, PropertyType, StatusGroup};

    use super::*;

    const ASSIGNEE: Uuid = Uuid::from_u128(1);
    const STATUS: Uuid = Uuid::from_u128(2);
    const DOING: Uuid = Uuid::from_u128(3);

    fn identity() -> User {
        User {
            id: Uuid::from_u128(7),
            kind: UserKind::Agent,
            email: None,
            display_name: "Scout".into(),
            role: Role::User,
            hue_override: None,
            email_verified_at: None,
            disabled_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            workspace_id: Some(Uuid::from_u128(8)),
            owner_id: Some(Uuid::from_u128(9)),
        }
    }

    fn schema() -> Schema {
        Schema {
            properties: vec![
                Property {
                    id: ASSIGNEE,
                    name: "Assignee".into(),
                    ty: PropertyType::Person,
                },
                Property {
                    id: STATUS,
                    name: "Status".into(),
                    ty: PropertyType::Status {
                        options: vec![StatusOption {
                            id: DOING,
                            name: "Doing".into(),
                            hue: None,
                            group: Some(StatusGroup::InProgress),
                        }],
                    },
                },
            ],
        }
    }

    fn event(kind: EventType, property: Uuid) -> Event {
        Event {
            id: Uuid::from_u128(20),
            kind: kind.as_str().to_owned(),
            occurred_at: 1_795_348_800_000_000,
            workspace_id: Uuid::from_u128(8),
            database_id: Uuid::from_u128(21),
            row_id: Uuid::from_u128(22),
            property_id: property,
            identity_id: Uuid::from_u128(7),
            from: None,
            to: Some(Choice {
                id: DOING,
                name: "Doing".into(),
            }),
        }
    }

    fn facts(seen: bool) -> RowFacts {
        RowFacts {
            title: "Fix login".into(),
            unique_id: "NW-42".into(),
            database_id: Uuid::from_u128(21),
            schema: seen.then(|| ("Tasks".to_owned(), schema())),
            cells: json!({ STATUS.to_string(): DOING.to_string() }),
        }
    }

    #[test]
    fn an_assignment_names_the_row_its_database_and_the_property() {
        let payload = row_payload(
            &event(EventType::Assigned, ASSIGNEE),
            &identity(),
            &facts(true),
        );

        assert_eq!(
            payload,
            json!({
                "version": 1,
                "id": Uuid::from_u128(20).to_string(),
                "type": "row.assigned",
                "occurred_at": "2026-11-22T12:00:00Z",
                "workspace": {"id": Uuid::from_u128(8).to_string()},
                "identity": {"id": Uuid::from_u128(7).to_string(), "kind": "agent", "name": "Scout"},
                "row": {
                    "id": Uuid::from_u128(22).to_string(),
                    "unique_id": "NW-42",
                    "title": "Fix login",
                    "database": {"id": Uuid::from_u128(21).to_string(), "title": "Tasks"},
                    "status": {"id": DOING.to_string(), "name": "Doing"},
                },
                "property": {"id": ASSIGNEE.to_string(), "name": "Assignee", "type": "person"},
            })
        );
    }

    #[test]
    fn a_status_change_says_from_what_to_what() {
        let payload = row_payload(
            &event(EventType::StatusChanged, STATUS),
            &identity(),
            &facts(true),
        );

        assert_eq!(payload["type"], "row.status_changed");
        assert_eq!(payload["property"]["type"], "status");
        assert_eq!(payload["change"]["from"], Json::Null);
        assert_eq!(payload["change"]["to"]["name"], "Doing");
    }

    #[test]
    fn without_the_database_its_names_are_left_out() {
        let payload = row_payload(
            &event(EventType::StatusChanged, STATUS),
            &identity(),
            &facts(false),
        );

        assert_eq!(payload["row"]["database"], Json::Null);
        assert_eq!(payload["row"]["status"], Json::Null);
        assert_eq!(payload["property"]["name"], Json::Null);
        assert_eq!(payload["change"]["to"]["id"], DOING.to_string());
        assert_eq!(payload["change"]["to"]["name"], Json::Null);
        assert_eq!(payload["row"]["title"], "Fix login");
    }

    #[test]
    fn failures_have_stable_kinds() {
        assert_eq!(Failure::Redirect(302).kind(), "redirect");
        assert_eq!(Failure::Status(500).kind(), "http_status");
        assert_eq!(Failure::Send(SendError::Timeout).kind(), "timeout");
        assert_eq!(Failure::Status(503).describe(), "the receiver answered 503");
        assert_eq!(millis(Duration::from_millis(1500)), 1500);
    }
}
