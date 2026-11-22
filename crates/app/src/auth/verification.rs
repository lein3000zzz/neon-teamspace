//! Verifying addresses by mailed links, and the links themselves.

use neon_domain::auth::email::normalize;
use neon_domain::auth::email_token::{LINK_GAP, LINK_WINDOW, LINKS_PER_WINDOW, MAX_LINK_TOKEN};
use neon_domain::auth::{EmailTokenPurpose, Principal, User};
use neon_domain::settings::Secret;
use neon_store::{email_tokens, users};
use tracing::{info, warn};
use uuid::Uuid;

use super::{AuthError, AuthService, require_admin};
use crate::crypto::{hash_secret, new_secret};
use crate::mail::{Mail, Message, Recipient};
use crate::ratelimit::Bucket;

impl AuthService {
    /// Makes a link of `purpose` for `user` and returns its token: at most one a minute
    /// and five a day of each purpose per account.
    pub(super) async fn new_link(
        &self,
        user: &User,
        purpose: EmailTokenPurpose,
    ) -> Result<Secret, AuthError> {
        let now = self.now();
        for (window, most) in [(LINK_GAP, 1), (LINK_WINDOW, LINKS_PER_WINDOW)] {
            let sent =
                email_tokens::count_since(self.pool(), user.id, purpose, now - window).await?;
            if sent >= most {
                return Err(AuthError::RateLimited {
                    retry_after: window,
                });
            }
        }
        let token = new_secret("");
        email_tokens::insert(
            self.pool(),
            &hash_secret(&token),
            user.id,
            purpose,
            now,
            now + purpose.lifetime(),
        )
        .await?;
        Ok(Secret::new(token))
    }

    /// Mails `user` a new link verifying its address. True when it went.
    pub(super) async fn send_verification(&self, user: &User) -> bool {
        let token = match self.new_link(user, EmailTokenPurpose::Verify).await {
            Ok(token) => token,
            Err(error) => {
                warn!(user_id = %user.id, %error, "no verification link made");
                return false;
            }
        };
        let sent = self
            .send(Mail {
                to: Recipient::new(user.email.as_str(), Some(user.display_name.clone())),
                message: Message::Verification {
                    token,
                    lasts: EmailTokenPurpose::Verify.lifetime(),
                },
            })
            .await;
        if sent {
            info!(user_id = %user.id, "verification link mailed");
        }
        sent
    }

    /// Verifies the address of the account a mailed link is for, and returns the address
    /// for the sign-in form. The link works once, and the account's other verification
    /// links stop working with it. It signs nobody in: the password is still needed.
    pub async fn verify_email(&self, token: &Secret, ip: &str) -> Result<String, AuthError> {
        self.observe("verify_email", async {
            self.limit(Bucket::Verify, ip).await?;
            let token = token.expose().trim();
            if token.is_empty() || token.len() > MAX_LINK_TOKEN {
                return Err(AuthError::InvalidLink);
            }
            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let user_id = email_tokens::take(
                &mut *tx,
                &hash_secret(token),
                EmailTokenPurpose::Verify,
                now,
            )
            .await?
            .ok_or(AuthError::InvalidLink)?;
            users::set_email_verified(&mut *tx, user_id, now).await?;
            email_tokens::use_all(&mut *tx, user_id, EmailTokenPurpose::Verify, now).await?;
            let user = users::by_id(&mut *tx, user_id)
                .await?
                .ok_or(AuthError::InvalidLink)?;
            tx.commit().await?;
            info!(%user_id, "address verified");
            Ok(user.email.as_str().to_owned())
        })
        .await
    }

    /// Mails a new verification link to the account of an address that must still verify
    /// it. It answers the same whether there is one or not, and as fast: the account is
    /// looked up and mailed after the answer.
    pub async fn resend_verification(&self, email: &str, ip: &str) -> Result<(), AuthError> {
        self.observe("resend_verification", async {
            self.limit(Bucket::Verify, ip).await?;
            if !self.mail_configured() {
                return Err(AuthError::MailNotSetUp);
            }
            let service = self.clone();
            let email = normalize(email);
            self.background("resend_verification", async move {
                match users::by_email(service.pool(), &email).await {
                    Ok(Some((user, _))) if !user.is_verified() && !user.is_disabled() => {
                        service.send_verification(&user).await;
                    }
                    Ok(_) => {}
                    Err(error) => warn!(%error, "looking the account up failed"),
                }
            });
            Ok(())
        })
        .await
    }

    /// Mails a user who must verify their address a new link, for an admin.
    pub async fn send_verification_link(
        &self,
        actor: &Principal,
        user_id: Uuid,
    ) -> Result<(), AuthError> {
        self.observe("send_verification_link", async {
            require_admin(actor)?;
            if !self.mail_configured() {
                return Err(AuthError::MailNotSetUp);
            }
            let user = users::by_id(self.pool(), user_id)
                .await?
                .ok_or(AuthError::NotFound("no such user"))?;
            if user.is_verified() {
                return Err(AuthError::InvalidInput(
                    "the address is verified already".to_owned(),
                ));
            }
            let token = self.new_link(&user, EmailTokenPurpose::Verify).await?;
            let sent = self
                .send(Mail {
                    to: Recipient::new(user.email.as_str(), Some(user.display_name.clone())),
                    message: Message::Verification {
                        token,
                        lasts: EmailTokenPurpose::Verify.lifetime(),
                    },
                })
                .await;
            if !sent {
                return Err(AuthError::MailFailed(
                    "the link was made but not mailed".to_owned(),
                ));
            }
            info!(actor_id = %actor.user.id, %user_id, "verification link mailed");
            Ok(())
        })
        .await
    }
}
