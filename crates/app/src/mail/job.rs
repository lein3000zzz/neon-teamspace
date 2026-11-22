use std::sync::Arc;

use async_trait::async_trait;
use neon_store::jobs::Job;
use tracing::{info, warn};

use super::queue::{KIND, MailQueue, Queued};
use super::{Mailer, SendError};
use crate::jobs::{Handler, JobError};

/// Runs mail jobs: opens the sealed mail, writes it with the settings of the moment, and
/// sends it through the [`Mailer`].
///
/// A mail server that refuses the recipient, a payload that does not open, and a link that
/// expired while the mail waited end the job at once; anything else is retried with the
/// worker's backoff. Its jobs are not kept once they finish, as their payload holds the
/// mail's links. Logs name the kind of mail, never its recipient or its links.
pub struct MailJob {
    queue: MailQueue,
    mailer: Arc<dyn Mailer>,
}

impl MailJob {
    pub fn new(queue: MailQueue, mailer: Arc<dyn Mailer>) -> Self {
        Self { queue, mailer }
    }
}

#[async_trait]
impl Handler for MailJob {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn keeps_finished(&self) -> bool {
        false
    }

    async fn run(&self, job: &Job) -> Result<(), JobError> {
        let Queued { mail, queued_at } = self.queue.open(job).map_err(JobError::permanent)?;
        let kind = mail.message.kind();
        if mail
            .message
            .link_expires(queued_at)
            .is_some_and(|expires| expires <= self.queue.now())
        {
            warn!(
                kind,
                "mail dropped: its link expired before it could be sent"
            );
            return Err(JobError::permanent(
                "the link in it expired before it could be sent",
            ));
        }
        let outgoing = self.queue.write(&mail).ok_or_else(|| {
            let error = SendError::NotConfigured(
                "server.public_url is not set: the links in mail point there".to_owned(),
            );
            self.queue.failed(kind, &error.to_string(), false);
            JobError::retry(error)
        })?;

        match self.mailer.send(&outgoing).await {
            Ok(()) => {
                self.queue.sent();
                info!(kind, "mail sent");
                Ok(())
            }
            Err(error) => {
                let permanent = matches!(error, SendError::Rejected(_));
                self.queue.failed(kind, &error.to_string(), permanent);
                if permanent {
                    Err(JobError::permanent(error))
                } else {
                    Err(JobError::retry(error))
                }
            }
        }
    }
}
