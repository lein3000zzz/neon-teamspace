//! The mail of the inbox: for those who want a mail for each notification, one of what is
//! still unread a while after it came, one mail at a time, and digests, each written when it
//! goes, of what its person may see then.

use async_trait::async_trait;
use neon_domain::auth::{Principal, User};
use neon_domain::notifications::{DEFAULT_MAIL, MAX_DIGEST, MailPreference, NotificationKind};
use neon_domain::settings::Secret;
use neon_domain::workspaces::Action;
use neon_store::jobs::{self, Job, NewJob};
use neon_store::notifications::{self as repo, Notification};
use neon_store::{comments, users, workspace_invites, workspaces};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use tracing::{debug, info};
use uuid::Uuid;

use super::notify::reader;
use super::{InboxError, InboxService, MAIL_GAP, MAIL_KIND, comment_excerpt};
use crate::jobs::{Handler, JobError};
use crate::mail::{Mail, Message, Notice, Recipient};
use crate::pages::PageError;
use crate::settings::notifications as keys;

/// The app's inbox page.
const INBOX_PAGE: &str = "/inbox";

/// How many people's digests one run writes at most; the next run takes the rest.
const DIGESTS_PER_RUN: i64 = 100;

/// Runs the jobs that mail a person what came to their inbox.
pub struct NotificationMailJob {
    inbox: InboxService,
}

impl NotificationMailJob {
    pub fn new(inbox: InboxService) -> Self {
        Self { inbox }
    }
}

#[derive(Deserialize)]
struct Payload {
    user_id: Uuid,
}

#[async_trait]
impl Handler for NotificationMailJob {
    fn kind(&self) -> &'static str {
        MAIL_KIND
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        let payload: Payload = serde_json::from_value(job.payload.clone())
            .map_err(|_| JobError::permanent("the payload names nobody"))?;
        self.inbox
            .mail_due(payload.user_id)
            .await
            .map_err(JobError::retry)
    }
}

impl InboxService {
    /// Mails a person what came to their inbox, as the job of their mail runs: when the
    /// instance sends notification mail, they want a mail for each and their mail is due,
    /// every notification of theirs still unread, unarchived and unmailed that has waited
    /// `notifications.mail_delay`, and that they may still see. One goes on its own; more
    /// go together, as a digest lists them. What came since waits for the next, which is
    /// asked for once it too has waited, and [`MAIL_GAP`] after this one at the soonest.
    pub async fn mail_due(&self, user_id: Uuid) -> Result<(), InboxError> {
        self.observe("mail", async {
            let settings = self.inner.pages.settings().current();
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let Some((Some(due), sent)) = repo::lock_mail(&mut *tx, user_id).await? else {
                return Ok(());
            };
            if due > now {
                // Asked for again since: that job sends it.
                return Ok(());
            }
            let preference = repo::preference(&mut *tx, user_id)
                .await?
                .map_or(DEFAULT_MAIL, |(mail, _)| mail);
            let recipient = self.recipient(&mut tx, user_id).await?;
            let sends = *settings.get(keys::MAIL) && self.inner.mail.configured();
            let (Some(recipient), true, MailPreference::Each) = (recipient, sends, preference)
            else {
                // Nothing waits any more: what comes later asks afresh.
                repo::set_mail(&mut *tx, user_id, None, sent).await?;
                tx.commit().await?;
                return Ok(());
            };
            let delay = *settings.get(keys::MAIL_DELAY);
            let waited = now - delay;
            let waiting = repo::unmailed(
                &mut *tx,
                user_id,
                waited,
                i64::try_from(MAX_DIGEST * 5).unwrap_or(i64::MAX),
            )
            .await?;
            let mut told = Vec::new();
            let mut notices = Vec::new();
            for notification in &waiting {
                if let Some(notice) = self.notice(&mut tx, &recipient, notification).await? {
                    told.push(notification.id);
                    notices.push(notice);
                }
            }
            let mut last = sent;
            if !notices.is_empty()
                && let Some(unsubscribe) = self.unsubscribe_token(user_id)
            {
                let message = if notices.len() == 1 {
                    Message::Notification {
                        notice: notices.remove(0),
                        unsubscribe: Secret::new(unsubscribe),
                    }
                } else {
                    // Newest first, as a digest lists them.
                    notices.reverse();
                    let more = notices.len().saturating_sub(MAX_DIGEST);
                    notices.truncate(MAX_DIGEST);
                    Message::Digest {
                        notices,
                        more,
                        path: INBOX_PAGE.to_owned(),
                        unsubscribe: Secret::new(unsubscribe),
                    }
                };
                let mail = Mail {
                    to: recipient_of(&recipient),
                    message,
                };
                self.inner.mail.queue(&mut tx, &mail).await?;
                repo::mark_mailed(&mut *tx, &told, now).await?;
                last = Some(now);
                info!(user_id = %user_id, notifications = told.len(), "notifications mailed");
            }
            // What came since it waited asks for the next.
            let next = match repo::next_unmailed(&mut *tx, user_id, waited).await? {
                Some(came) => {
                    let due = last.map_or(came + delay, |last| (came + delay).max(last + MAIL_GAP));
                    let job = NewJob::new(MAIL_KIND, json!({ "user_id": user_id })).run_at(due);
                    jobs::enqueue(&mut *tx, &job).await?;
                    Some(due)
                }
                None => None,
            };
            repo::set_mail(&mut *tx, user_id, next, last).await?;
            tx.commit().await?;
            debug!(user_id = %user_id, next = ?next, "notification mail done");
            Ok(())
        })
        .await
    }

    /// Mails the digests that are due: to each who wants one and has had none for
    /// `notifications.digest_interval`, of what came to their inbox since, is still unread
    /// and they may see. Says how many went. The server runs it every few minutes.
    pub async fn send_digests(&self) -> Result<usize, InboxError> {
        self.observe("digests", async {
            let settings = self.inner.pages.settings().current();
            if !*settings.get(keys::MAIL) || !self.inner.mail.configured() {
                return Ok(0);
            }
            let now = self.now();
            let due_before = now - *settings.get(keys::DIGEST_INTERVAL);
            let mut tx = self.pool().begin().await?;
            let mut sent = 0;
            for user_id in repo::digests_due(&mut *tx, due_before, DIGESTS_PER_RUN).await? {
                repo::digest_sent(&mut *tx, user_id, now).await?;
                let Some(recipient) = self.recipient(&mut tx, user_id).await? else {
                    continue;
                };
                let waiting = repo::unmailed(
                    &mut *tx,
                    user_id,
                    now,
                    i64::try_from(MAX_DIGEST * 5).unwrap_or(i64::MAX),
                )
                .await?;
                let mut told = Vec::new();
                let mut notices = Vec::new();
                for notification in waiting.iter().rev() {
                    if let Some(notice) = self.notice(&mut tx, &recipient, notification).await? {
                        told.push(notification.id);
                        notices.push(notice);
                    }
                }
                let Some(unsubscribe) = self.unsubscribe_token(user_id) else {
                    continue;
                };
                if notices.is_empty() {
                    continue;
                }
                let more = notices.len().saturating_sub(MAX_DIGEST);
                notices.truncate(MAX_DIGEST);
                let mail = Mail {
                    to: recipient_of(&recipient),
                    message: Message::Digest {
                        notices,
                        more,
                        path: INBOX_PAGE.to_owned(),
                        unsubscribe: Secret::new(unsubscribe),
                    },
                };
                self.inner.mail.queue(&mut tx, &mail).await?;
                repo::mark_mailed(&mut *tx, &told, now).await?;
                sent += 1;
            }
            tx.commit().await?;
            if sent > 0 {
                info!(digests = sent, "digests mailed");
            }
            Ok(sent)
        })
        .await
    }

    /// The person a mail goes to: one with an address whose account works.
    async fn recipient(
        &self,
        conn: &mut PgConnection,
        user_id: Uuid,
    ) -> Result<Option<User>, InboxError> {
        Ok(users::by_id(&mut *conn, user_id)
            .await?
            .filter(|user| user.kind.is_human() && user.email.is_some() && !user.is_disabled()))
    }

    /// What a mail says of a notification, as its person may see it now; `None` when they
    /// may no longer see what it is about.
    async fn notice(
        &self,
        conn: &mut PgConnection,
        recipient: &User,
        notification: &Notification,
    ) -> Result<Option<Notice>, InboxError> {
        let actor = match notification.actor_id {
            Some(id) => self.name_of(conn, id).await?,
            None => None,
        };
        let actor = actor.unwrap_or_else(|| "Someone".to_owned());
        if notification.kind == NotificationKind::Invitation {
            let pending = workspace_invites::pending_for_user(&mut *conn, recipient.id, self.now())
                .await?
                .into_iter()
                .find(|(invitation, _, _)| Some(invitation.id) == notification.invitation_id);
            return Ok(pending.map(|(_, workspace, _)| Notice {
                summary: format!("{actor} invited you to join {}", workspace.name),
                excerpt: None,
                path: INBOX_PAGE.to_owned(),
            }));
        }
        let (Some(page_id), Some(reader)) = (
            notification.page_id,
            reader(
                &mut *conn,
                self.inner.pages.settings(),
                recipient.id,
                notification.workspace_id,
            )
            .await?,
        ) else {
            return Ok(None);
        };
        let Some(page) = self.visible_page(conn, &reader, page_id).await? else {
            return Ok(None);
        };
        let Some(workspace) = workspaces::get(&mut *conn, notification.workspace_id).await? else {
            return Ok(None);
        };
        let title = if page.title.is_empty() {
            "Untitled".to_owned()
        } else {
            page.title.clone()
        };
        let summary = match notification.kind {
            NotificationKind::Mention if notification.comment_id.is_some() => {
                format!("{actor} mentioned you in a comment on {title}")
            }
            NotificationKind::Mention => format!("{actor} mentioned you on {title}"),
            NotificationKind::Reply => format!("{actor} replied in a thread on {title}"),
            NotificationKind::Comment => format!("{actor} commented on {title}"),
            NotificationKind::Assigned => format!("{actor} assigned you to {title}"),
            NotificationKind::Invitation => return Ok(None),
        };
        let excerpt = match notification.comment_id {
            Some(id) => match comments::comment(&mut *conn, id).await? {
                Some(comment) => Some(comment_excerpt(&comment.body)),
                None => return Ok(None),
            },
            None => None,
        };
        let mut path = format!("/w/{}/p/{}", workspace.slug, page.id);
        if let Some(thread) = notification.thread_id {
            path.push_str(&format!("?comment={thread}"));
        } else if let Some(block) = &notification.block_id {
            let block: String = url::form_urlencoded::byte_serialize(block.as_bytes()).collect();
            path.push_str(&format!("?block={block}"));
        }
        Ok(Some(Notice {
            summary: format!("{summary}, in {}", workspace.name),
            excerpt,
            path,
        }))
    }

    /// A page as someone reading may see it now, out of the trash.
    async fn visible_page(
        &self,
        conn: &mut PgConnection,
        reader: &Principal,
        page_id: Uuid,
    ) -> Result<Option<neon_domain::pages::Page>, InboxError> {
        match self.inner.pages.context(&mut *conn, reader, page_id).await {
            Ok(context)
                if context.check(Action::ViewPage).is_ok() && !context.page().is_trashed() =>
            {
                Ok(Some(context.page().clone()))
            }
            Ok(_) | Err(PageError::NotFound(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// How a mail names someone: a person by their name, an agent or integration with the
    /// person it acts for.
    async fn name_of(
        &self,
        conn: &mut PgConnection,
        user_id: Uuid,
    ) -> Result<Option<String>, InboxError> {
        let Some(user) = users::by_id(&mut *conn, user_id).await? else {
            return Ok(None);
        };
        let owner = match user.owner_id {
            Some(owner) if !user.kind.is_human() => users::by_id(&mut *conn, owner).await?,
            _ => None,
        };
        Ok(Some(match owner {
            Some(owner) => format!("{} via {}", user.display_name, owner.display_name),
            None => user.display_name,
        }))
    }
}

fn recipient_of(user: &User) -> Recipient {
    Recipient::new(
        user.email
            .as_ref()
            .map(|email| email.as_str().to_owned())
            .unwrap_or_default(),
        Some(user.display_name.clone()),
    )
}
