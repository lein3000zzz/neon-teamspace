//! The inbox: notifications, as each person reads them, and the mail of them.
//!
//! - **Making them.** A notification is made in the transaction of what it tells of, by
//!   the use case that makes it: a comment's mentions, replies and the page's followers
//!   ([`mentions::comment`]), a page's text once its mentions settle in a stored snapshot
//!   ([`mentions::page_text`]), a row's assignment, and an invitation into a workspace. A
//!   comment tells each person once, by the first rule that reaches them, and only those
//!   who may see its page then.
//! - **Reading them.** An inbox holds a person's notifications from every workspace they
//!   are in, and their invitations into others while those wait for an answer; a token
//!   bound to a workspace reads that workspace's alone, and no invitation. Each is asked
//!   again whether its person may see what it is about whenever it is shown or counted:
//!   one about a page they lost is left out, not deleted, and shows again should they get
//!   the page back.
//! - **Live.** Once a change to an inbox commits, its room of the
//!   [`Broadcaster`](crate::sync::Broadcaster) hears it, which the person's feeds ([`live`])
//!   and MCP subscriptions follow.
//! - **Mail.** While `notifications.mail` is on and mail can go, a person who wants a mail
//!   for each notification gets one once it has waited `notifications.mail_delay` unread,
//!   through the job of [`MAIL_KIND`]. One such mail waits at a time for each person: what
//!   comes while it waits goes in it, once it too has waited, or in the next, and the next
//!   goes [`MAIL_GAP`] after the last at the soonest, so a flood of notifications makes a
//!   few mails that list them, not one each. One who wants a digest gets one every
//!   `notifications.digest_interval` at most, of what came since and is still unread, from
//!   [`InboxService::send_digests`]. Both go through the mail queue, say only what their
//!   person may see when they are written, and carry a link that stops them without
//!   signing in, signed with the instance's key.
//!
//! Every use case runs in a span named `notifications <operation>` and is measured as RED
//! metrics of [`SUBSYSTEM`].

mod error;
pub mod live;
mod mail;
pub(crate) mod mentions;
pub(crate) mod notify;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neon_blocks::Inline;
use neon_blocks::markdown::inline_from_markdown;
use neon_domain::auth::Principal;
use neon_domain::notifications::{MailPreference, NotificationKind, excerpt};
use neon_domain::workspaces::{Invitation, Person, Workspace};
use neon_observability::errors::{mark_expected, record_unexpected};
use neon_observability::red::Red;
use neon_store::notifications::{self as repo, Filter, Inbox, Listed, Notification};
use neon_store::{workspace_invites, workspaces};
use sqlx::{PgConnection, PgPool};
use tracing::{Instrument as _, info, info_span};
use uuid::Uuid;

use crate::crypto::Envelope;
use crate::mail::MailQueue;
use crate::pages::{Facts, PageService, PageView};
use crate::settings::notifications as keys;
use notify::Notifier;

pub use error::InboxError;
pub use live::{FeedLimits, InboxFeed, InboxFeeds};
pub use mail::NotificationMailJob;

/// RED subsystem of the inbox's use cases.
pub const SUBSYSTEM: &str = "notifications";

/// The job kind of the mail of a person's notifications, for those who want a mail for
/// each.
pub const MAIL_KIND: &str = "notifications.mail";

/// How long after one mail of a person's notifications the next goes at the soonest: what
/// comes meanwhile waits for it, so nobody gets more than four of them an hour.
pub const MAIL_GAP: time::Duration = time::Duration::minutes(15);

/// How long after it was due a mail that has not gone is taken as lost, its job gone with
/// the queue's retention say: the next notification asks for another.
const MAIL_LOST: time::Duration = time::Duration::hours(1);

/// What an unsubscribe link's signature is for.
const UNSUBSCRIBE: &str = "unsubscribe";

/// How many notifications a part of an inbox holds unless asked otherwise, and at most.
pub const DEFAULT_LIST: usize = 50;
pub const MAX_LIST: usize = 200;

/// The most unread notifications counted; more show as this many.
pub const MAX_COUNT: usize = 999;

const NO_WORKSPACE: &str = "no such workspace";
const NO_NOTIFICATION: &str = "no such notification";

/// Which part of an inbox to read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InboxQuery {
    /// One workspace's notifications; every workspace's when unset, but for a token bound
    /// to one.
    pub workspace_id: Option<Uuid>,
    pub unread: bool,
    pub archived: bool,
    /// The notifications older than this one.
    pub before: Option<Uuid>,
    pub limit: Option<usize>,
}

/// A notification as its person reads it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub notification: Notification,
    /// Who did what it tells of, while their account is there.
    pub actor: Option<Person>,
    /// The workspace it is in: the one an invitation invites to.
    pub workspace: Workspace,
    /// The page it is about, as its person sees it now.
    pub page: Option<PageView>,
    /// The start of the comment it is about.
    pub excerpt: Option<String>,
    /// The invitation it is, while it waits.
    pub invitation: Option<Invitation>,
    /// Its person knows no other member: what it shows of people leaves out their
    /// addresses.
    pub members_hidden: bool,
}

/// A part of an inbox, newest first, and where the next part starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxPage {
    pub notifications: Vec<Shown>,
    pub next: Option<Uuid>,
}

/// How a person wants mail of their inbox, and whether the instance sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailSettings {
    pub preference: MailPreference,
    /// The instance sends notification mail now.
    pub available: bool,
    pub digest_interval: Duration,
}

/// The inbox's use cases. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct InboxService {
    inner: Arc<Inner>,
}

struct Inner {
    pages: PageService,
    mail: MailQueue,
    envelope: Envelope,
    red: Red,
}

impl InboxService {
    /// The inboxes of `pages`' people, whose mail `mail` queues, with links signed by
    /// `envelope`. Call it after telemetry is installed, as it creates the RED metrics.
    pub fn new(pages: PageService, mail: MailQueue, envelope: Envelope) -> Self {
        Self {
            inner: Arc::new(Inner {
                pages,
                mail,
                envelope,
                red: Red::new(SUBSYSTEM),
            }),
        }
    }

    fn pool(&self) -> &PgPool {
        self.inner.pages.pool()
    }

    fn now(&self) -> time::OffsetDateTime {
        self.inner.pages.now()
    }

    /// Runs a use case in its span and records its outcome.
    async fn observe<T>(
        &self,
        operation: &'static str,
        work: impl Future<Output = Result<T, InboxError>>,
    ) -> Result<T, InboxError> {
        let span = info_span!(
            "notifications",
            otel.name = %format!("notifications {operation}"),
            notifications.operation = operation,
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

    /// The part of the principal's inbox a query asks for, each notification checked again
    /// against what they may see now; see the module docs.
    pub async fn inbox(
        &self,
        principal: &Principal,
        query: InboxQuery,
    ) -> Result<InboxPage, InboxError> {
        self.observe("list", async {
            let mut conn = self.pool().acquire().await?;
            let inbox = scope(&mut conn, principal, query.workspace_id).await?;
            let limit = query.limit.unwrap_or(DEFAULT_LIST).clamp(1, MAX_LIST);
            let filter = Filter {
                unread: query.unread,
                archived: query.archived,
            };
            let listed = repo::list(
                &mut *conn,
                inbox,
                filter,
                query.before,
                i64::try_from(limit).unwrap_or(i64::MAX),
                self.now(),
            )
            .await?;
            let next = (listed.len() == limit)
                .then(|| listed.last().map(|last| last.notification.id))
                .flatten();
            let notifications = self.show(&mut conn, principal, listed).await?;
            Ok(InboxPage {
                notifications,
                next,
            })
        })
        .await
    }

    /// How many of the principal's notifications are unread, of one workspace or all, up
    /// to [`MAX_COUNT`], each checked again against what they may see now.
    pub async fn unread_count(
        &self,
        principal: &Principal,
        workspace_id: Option<Uuid>,
    ) -> Result<usize, InboxError> {
        self.observe("count", async {
            let mut conn = self.pool().acquire().await?;
            let inbox = scope(&mut conn, principal, workspace_id).await?;
            let unread = repo::unread(
                &mut *conn,
                inbox,
                i64::try_from(MAX_COUNT).unwrap_or(i64::MAX),
                self.now(),
            )
            .await?;
            let pages: Vec<Uuid> = unread
                .iter()
                .filter_map(|(_, page)| *page)
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            let seen = Facts::read(&mut conn, principal, &pages)
                .await?
                .seen(principal)?;
            Ok(unread
                .iter()
                .filter(|(_, page)| {
                    page.is_none_or(|page| seen.get(&page).is_some_and(Option::is_some))
                })
                .count())
        })
        .await
    }

    /// Marks one of the principal's notifications read or unread, archived or not, as
    /// given, and tells their inbox.
    pub async fn mark(
        &self,
        principal: &Principal,
        id: Uuid,
        read: Option<bool>,
        archived: Option<bool>,
    ) -> Result<(), InboxError> {
        self.observe("mark", async {
            let mut conn = self.pool().acquire().await?;
            let found = repo::get(&mut *conn, principal.user.id, id)
                .await?
                .filter(|notification| reaches(principal, notification))
                .ok_or(InboxError::NotFound(NO_NOTIFICATION))?;
            repo::mark(
                &mut *conn,
                principal.user.id,
                found.id,
                read,
                archived,
                self.now(),
            )
            .await?;
            self.changed(principal.user.id);
            Ok(())
        })
        .await
    }

    /// Marks read every notification of the principal's inbox, of one workspace or all.
    /// Says how many.
    pub async fn mark_all_read(
        &self,
        principal: &Principal,
        workspace_id: Option<Uuid>,
    ) -> Result<u64, InboxError> {
        self.observe("mark_all_read", async {
            let mut conn = self.pool().acquire().await?;
            let inbox = scope(&mut conn, principal, workspace_id).await?;
            let marked = repo::mark_all_read(&mut *conn, inbox, self.now()).await?;
            if marked > 0 {
                self.changed(principal.user.id);
            }
            Ok(marked)
        })
        .await
    }

    /// How the principal wants mail of their inbox.
    pub async fn mail_settings(&self, principal: &Principal) -> Result<MailSettings, InboxError> {
        self.observe("mail_settings", async {
            let preference = repo::preference(self.pool(), principal.user.id)
                .await?
                .map_or(neon_domain::notifications::DEFAULT_MAIL, |(mail, _)| mail);
            Ok(self.settings_with(preference))
        })
        .await
    }

    /// Sets how the principal wants mail of their inbox: people choose, agents and
    /// integrations get none.
    pub async fn set_mail(
        &self,
        principal: &Principal,
        preference: MailPreference,
    ) -> Result<MailSettings, InboxError> {
        self.observe("set_mail", async {
            if !principal.user.kind.is_human() || principal.user.email.is_none() {
                return Err(InboxError::Forbidden(
                    "agents and integrations get no mail: their inbox is read over MCP and the API",
                ));
            }
            repo::set_preference(self.pool(), principal.user.id, preference, self.now()).await?;
            info!(user_id = %principal.user.id, mail = preference.as_str(), "mail preference set");
            Ok(self.settings_with(preference))
        })
        .await
    }

    fn settings_with(&self, preference: MailPreference) -> MailSettings {
        let settings = self.inner.pages.settings().current();
        MailSettings {
            preference,
            available: *settings.get(keys::MAIL) && self.inner.mail.configured(),
            digest_interval: *settings.get(keys::DIGEST_INTERVAL),
        }
    }

    /// The token of the link that stops someone's notification mail: their id and a
    /// signature of it. `None` without a key to sign with.
    pub fn unsubscribe_token(&self, user_id: Uuid) -> Option<String> {
        let subject = user_id.to_string();
        let signature = self.inner.envelope.sign(UNSUBSCRIBE, &subject)?;
        Some(format!("{subject}.{signature}"))
    }

    /// Stops the notification mail of the person a link's token names, without signing
    /// in: their preference becomes `none`.
    pub async fn unsubscribe(&self, token: &str) -> Result<(), InboxError> {
        self.observe("unsubscribe", async {
            let (subject, signature) = token.split_once('.').ok_or(InboxError::InvalidToken)?;
            let user_id = Uuid::try_parse(subject).map_err(|_| InboxError::InvalidToken)?;
            if subject != user_id.to_string()
                || !self.inner.envelope.verify(UNSUBSCRIBE, subject, signature)
            {
                return Err(InboxError::InvalidToken);
            }
            if neon_store::users::by_id(self.pool(), user_id)
                .await?
                .is_none()
            {
                return Err(InboxError::InvalidToken);
            }
            repo::set_preference(self.pool(), user_id, MailPreference::None, self.now()).await?;
            info!(user_id = %user_id, "notification mail stopped by its link");
            Ok(())
        })
        .await
    }

    /// Tells someone's inbox that it changed.
    fn changed(&self, user_id: Uuid) {
        let mut notifier = Notifier::of(&self.inner.pages);
        notifier.touched(user_id);
        notifier.tell();
    }

    /// The listed notifications the principal may see now, as they read them.
    async fn show(
        &self,
        conn: &mut PgConnection,
        principal: &Principal,
        listed: Vec<Listed>,
    ) -> Result<Vec<Shown>, InboxError> {
        let pages: Vec<Uuid> = listed
            .iter()
            .filter_map(|listed| listed.notification.page_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let seen = Facts::read(&mut *conn, principal, &pages)
            .await?
            .seen(principal)?;
        let invitations: HashMap<Uuid, (Invitation, Workspace)> = if listed
            .iter()
            .any(|listed| listed.notification.kind == NotificationKind::Invitation)
        {
            workspace_invites::pending_for_user(&mut *conn, principal.user.id, self.now())
                .await?
                .into_iter()
                .map(|(invitation, workspace, _)| (invitation.id, (invitation, workspace)))
                .collect()
        } else {
            HashMap::new()
        };
        let mut known: HashMap<Uuid, Option<Workspace>> = HashMap::new();
        let mut shown = Vec::with_capacity(listed.len());
        for Listed {
            notification,
            actor,
            comment_body,
        } in listed
        {
            let (page, invitation, workspace) = match notification.invitation_id {
                Some(id) => match invitations.get(&id) {
                    Some((invitation, workspace)) => {
                        (None, Some(invitation.clone()), workspace.clone())
                    }
                    None => continue,
                },
                None => {
                    let Some(view) = notification
                        .page_id
                        .and_then(|page| seen.get(&page).cloned().flatten())
                    else {
                        continue;
                    };
                    let workspace = match known.get(&notification.workspace_id) {
                        Some(workspace) => workspace.clone(),
                        None => {
                            let found =
                                workspaces::get(&mut *conn, notification.workspace_id).await?;
                            known.insert(notification.workspace_id, found.clone());
                            found
                        }
                    };
                    let Some(workspace) = workspace else {
                        continue;
                    };
                    (Some(view), None, workspace)
                }
            };
            let members_hidden = page.as_ref().is_some_and(|view| view.members_hidden);
            shown.push(Shown {
                excerpt: comment_body.as_deref().map(comment_excerpt),
                notification,
                actor,
                workspace,
                page,
                invitation,
                members_hidden,
            });
        }
        Ok(shown)
    }
}

/// The start of a comment's text, as an inbox shows it.
pub(crate) fn comment_excerpt(body: &str) -> String {
    excerpt(&Inline::plain_text(&inline_from_markdown(body)))
}

/// Whether a notification is in the principal's reach: of the workspace its token is bound
/// to, if it is bound, and an invitation only for a person signed in at a browser or with a
/// token across the instance.
fn reaches(principal: &Principal, notification: &Notification) -> bool {
    match principal.workspace() {
        Some(bound) => {
            notification.kind != NotificationKind::Invitation && notification.workspace_id == bound
        }
        None => true,
    }
}

/// The inbox the principal reads: of the workspace asked for, which they must be in, or of
/// every workspace, the one its token is bound to for a bound token; with invitations for
/// a person reading across the instance.
async fn scope(
    conn: &mut PgConnection,
    principal: &Principal,
    workspace_id: Option<Uuid>,
) -> Result<Inbox, InboxError> {
    let bound = principal.workspace();
    let workspace_id = match (workspace_id, bound) {
        (Some(asked), Some(bound)) if asked != bound => {
            return Err(InboxError::NotFound(NO_WORKSPACE));
        }
        (asked, bound) => asked.or(bound),
    };
    if let Some(workspace_id) = workspace_id
        && workspaces::member_role(&mut *conn, workspace_id, principal.user.id)
            .await?
            .is_none()
    {
        return Err(InboxError::NotFound(NO_WORKSPACE));
    }
    Ok(Inbox {
        user_id: principal.user.id,
        workspace_id,
        invitations: bound.is_none() && principal.user.kind.is_human(),
    })
}
