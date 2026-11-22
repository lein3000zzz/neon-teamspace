//! Requests to join.

use neon_domain::auth::email_token::MAX_LINK_TOKEN;
use neon_domain::auth::join_request::{DENIED_WAIT, LINK_TTL, MAX_OPEN, RETENTION, check_note};
use neon_domain::auth::{Email, JoinRequest, JoinRequestStatus, Principal};
use neon_domain::settings::Secret;
use neon_store::{is_unique_violation, join_requests, users};
use tracing::{info, warn};
use uuid::Uuid;

use super::{AuthError, AuthService, CreatedInvite, require_admin};
use crate::crypto::{hash_secret, new_secret};
use crate::mail::{Mail, Message, Recipient};
use crate::ratelimit::Bucket;
use crate::settings::auth as keys;

impl AuthService {
    /// Takes a request to join, mailing the address a link to confirm it. It answers the
    /// same whatever comes of it, and as fast, so the form tells nobody which addresses
    /// have accounts or requests: the work is done after the answer. Nothing is mailed to
    /// an address with an account, one with a request waiting for the admins, one denied in
    /// the last 30 days, or one past its links; and nothing is kept past the cap on open
    /// requests.
    pub async fn request_to_join(
        &self,
        email: &str,
        note: &str,
        ip: &str,
    ) -> Result<(), AuthError> {
        self.observe("request_to_join", async {
            self.limit(Bucket::Join, ip).await?;
            if !self.requests_open() {
                return Err(AuthError::RequestsOff);
            }
            let email = Email::parse(email)?;
            let note = check_note(note).map_err(AuthError::invalid)?;
            let service = self.clone();
            self.background("join_request", async move {
                service.take_request(email, note).await;
            });
            Ok(())
        })
        .await
    }

    async fn take_request(&self, email: Email, note: String) {
        if let Err(error) = self.take_request_inner(&email, note).await {
            warn!(%error, "request to join not taken");
        }
    }

    async fn take_request_inner(&self, email: &Email, note: String) -> Result<(), AuthError> {
        let now = self.now();
        if let Some((user, _)) = users::by_email(self.pool(), email.normalized()).await?
            && user.is_verified()
        {
            return Ok(());
        }
        if join_requests::denied_since(self.pool(), email.normalized(), now - DENIED_WAIT).await? {
            return Ok(());
        }

        let token = new_secret("");
        match join_requests::open_for(self.pool(), email.normalized()).await? {
            Some(open) if !open.may_mail_again(now) => return Ok(()),
            Some(open) => {
                if !join_requests::remail(
                    self.pool(),
                    open.id,
                    &hash_secret(&token),
                    now + LINK_TTL,
                    now,
                )
                .await?
                {
                    return Ok(());
                }
                info!(request_id = %open.id, "request to join mailed again");
            }
            None => {
                let request = JoinRequest {
                    id: Uuid::now_v7(),
                    email: email.as_str().to_owned(),
                    note,
                    created_at: now,
                    confirmed_at: None,
                    decided_at: None,
                    decided_by: None,
                    approved: None,
                    invite_id: None,
                    link_expires_at: now + LINK_TTL,
                    mails: 1,
                    mailed_at: now,
                };
                let kept = join_requests::insert_capped(
                    self.pool(),
                    &request,
                    email.normalized(),
                    &hash_secret(&token),
                    MAX_OPEN,
                )
                .await;
                match kept {
                    Ok(true) => info!(request_id = %request.id, "request to join taken"),
                    Ok(false) => {
                        warn!(
                            max_open = MAX_OPEN,
                            "request to join dropped: too many are open"
                        );
                        return Ok(());
                    }
                    // Another request for the address came in at the same moment.
                    Err(error) if is_unique_violation(&error, join_requests::OPEN_FOR_EMAIL) => {
                        return Ok(());
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }

        self.send(Mail {
            to: Recipient::new(email.as_str(), None),
            message: Message::JoinConfirmation {
                token: Secret::new(token),
                lasts: LINK_TTL,
            },
        })
        .await;
        Ok(())
    }

    /// Confirms the request of a mailed link. The admins see it from now, and are told by
    /// mail.
    pub async fn confirm_join(&self, token: &Secret, ip: &str) -> Result<(), AuthError> {
        self.observe("confirm_join", async {
            self.limit(Bucket::JoinConfirm, ip).await?;
            let token = token.expose().trim();
            if token.is_empty() || token.len() > MAX_LINK_TOKEN {
                return Err(AuthError::InvalidLink);
            }
            let request = join_requests::confirm(self.pool(), &hash_secret(token), self.now())
                .await?
                .ok_or(AuthError::InvalidLink)?;
            info!(request_id = %request.id, "request to join confirmed");
            let service = self.clone();
            self.background("join_requested_mail", async move {
                service.tell_admins(&request).await;
            });
            Ok(())
        })
        .await
    }

    async fn tell_admins(&self, request: &JoinRequest) {
        let admins = match users::active_admin_emails(self.pool()).await {
            Ok(admins) => admins,
            Err(error) => {
                warn!(%error, "looking the admins up failed");
                return;
            }
        };
        for admin in admins {
            self.send(Mail {
                to: Recipient::new(admin, None),
                message: Message::JoinRequested {
                    requester: request.email.clone(),
                    note: request.note.clone(),
                },
            })
            .await;
        }
    }

    /// The confirmed requests: those waiting first, then those decided in the last 30 days.
    pub async fn list_join_requests(
        &self,
        actor: &Principal,
    ) -> Result<Vec<JoinRequest>, AuthError> {
        self.observe("list_join_requests", async {
            require_admin(actor)?;
            Ok(join_requests::list(self.pool(), self.now() - RETENTION).await?)
        })
        .await
    }

    /// Approves a waiting request: an invite bound to its address, once, is made and
    /// mailed there. The invite is made even when the mail fails: the flag says whether it
    /// went, and the code can be copied.
    pub async fn approve_join(
        &self,
        actor: &Principal,
        id: Uuid,
    ) -> Result<(CreatedInvite, bool), AuthError> {
        self.observe("approve_join", async {
            let request = self.pending_request(actor, id).await?;
            let email = Email::parse(&request.email)?;
            self.refuse_existing_account(&email).await?;

            let now = self.now();
            let code = Secret::new(new_secret(neon_domain::auth::invite::GENERATED_PREFIX));
            let mut tx = self.pool().begin().await?;
            let created = self
                .new_invite(&mut tx, actor, code, Some(request.email.clone()), 1, now + self.invite_ttl())
                .await?;
            if !join_requests::decide(&mut *tx, id, true, actor.user.id, now, Some(created.invite.id)).await? {
                return Err(AuthError::NotFound("the request is not waiting for an admin"));
            }
            tx.commit().await?;
            info!(actor_id = %actor.user.id, request_id = %id, invite_id = %created.invite.id, "request to join approved");

            let sent = self.mail_invite(actor, &email, &created).await;
            Ok((created, sent))
        })
        .await
    }

    /// Denies a waiting request. The address may ask again in 30 days, and is told so by
    /// mail while `auth.request_denied_mail` is on and mail is set up.
    pub async fn deny_join(&self, actor: &Principal, id: Uuid) -> Result<(), AuthError> {
        self.observe("deny_join", async {
            let request = self.pending_request(actor, id).await?;
            if !join_requests::decide(self.pool(), id, false, actor.user.id, self.now(), None)
                .await?
            {
                return Err(AuthError::NotFound(
                    "the request is not waiting for an admin",
                ));
            }
            info!(actor_id = %actor.user.id, request_id = %id, "request to join denied");

            if *self.settings().get(keys::REQUEST_DENIED_MAIL) && self.mail_configured() {
                let service = self.clone();
                self.background("join_denied_mail", async move {
                    service
                        .send(Mail {
                            to: Recipient::new(request.email, None),
                            message: Message::JoinDenied {
                                ask_again_after: DENIED_WAIT,
                            },
                        })
                        .await;
                });
            }
            Ok(())
        })
        .await
    }

    async fn pending_request(&self, actor: &Principal, id: Uuid) -> Result<JoinRequest, AuthError> {
        require_admin(actor)?;
        let request = join_requests::get(self.pool(), id)
            .await?
            .ok_or(AuthError::NotFound("no such request"))?;
        if request.status() != JoinRequestStatus::Pending {
            return Err(AuthError::NotFound(
                "the request is not waiting for an admin",
            ));
        }
        Ok(request)
    }
}
