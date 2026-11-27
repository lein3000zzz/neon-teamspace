//! Verifying addresses by mailed links, and the links themselves.

use neon_domain::auth::email::normalize;
use neon_domain::auth::email_token::{LINK_GAP, LINK_WINDOW, LINKS_PER_WINDOW, MAX_LINK_TOKEN};
use neon_domain::auth::{EmailTokenPurpose, Principal, User};
use neon_domain::settings::Secret;
use neon_store::{email_tokens, users};
use sqlx::PgConnection;
use tracing::{info, warn};
use uuid::Uuid;

use super::{AuthError, AuthService, require_admin};
use crate::crypto::{hash_secret, new_secret};
use crate::mail::{Mail, Message, Recipient};
use crate::ratelimit::Bucket;

/// Who a mail to an account goes to.
pub(super) fn recipient(user: &User) -> Recipient {
    Recipient::new(user.address(), Some(user.display_name.clone()))
}

impl AuthService {
    /// Makes a link of `purpose` for `user` on `conn` and returns its token: at most one a
    /// minute and five a day of each purpose per account.
    pub(super) async fn new_link(
        &self,
        conn: &mut PgConnection,
        user: &User,
        purpose: EmailTokenPurpose,
    ) -> Result<Secret, AuthError> {
        let now = self.now();
        for (window, most) in [(LINK_GAP, 1), (LINK_WINDOW, LINKS_PER_WINDOW)] {
            let sent =
                email_tokens::count_since(&mut *conn, user.id, purpose, now - window).await?;
            if sent >= most {
                return Err(AuthError::RateLimited {
                    retry_after: window,
                });
            }
        }
        let token = new_secret("");
        email_tokens::insert(
            &mut *conn,
            &hash_secret(&token),
            user.id,
            purpose,
            now,
            now + purpose.lifetime(),
        )
        .await?;
        Ok(Secret::new(token))
    }

    /// Makes a link verifying `user`'s address and queues it to them, both on `conn`.
    pub(super) async fn queue_verification(
        &self,
        conn: &mut PgConnection,
        user: &User,
    ) -> Result<(), AuthError> {
        let token = self.new_link(conn, user, EmailTokenPurpose::Verify).await?;
        let mail = Mail {
            to: recipient(user),
            message: Message::Verification {
                token,
                lasts: EmailTokenPurpose::Verify.lifetime(),
            },
        };
        self.queue(conn, &mail).await
    }

    /// Verifies the address of the account a mailed link is for, and returns the address
    /// for the sign-in form. The link works once, and the account's other verification
    /// links stop working with it. It signs nobody in: the password is still needed.
    pub async fn verify_email(&self, token: &Secret, ip: &str) -> Result<String, AuthError> {
        self.observe("verify_email", async {
            self.limit_source(Bucket::Verify, ip).await?;
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
            Ok(user.address().to_owned())
        })
        .await
    }

    /// Mails a new verification link to the account of an address that must still verify
    /// it. It answers the same whether there is one or not, and as fast: the account is
    /// looked up and mailed after the answer.
    pub async fn resend_verification(&self, email: &str, ip: &str) -> Result<(), AuthError> {
        self.observe("resend_verification", async {
            self.limit_source(Bucket::Verify, ip).await?;
            if !self.mail_configured() {
                return Err(AuthError::MailNotSetUp(None));
            }
            let service = self.clone();
            let email = normalize(email);
            self.background("resend_verification", async move {
                if let Err(error) = service.mail_verification_again(&email).await {
                    warn!(%error, "no verification link mailed");
                }
            });
            Ok(())
        })
        .await
    }

    async fn mail_verification_again(&self, email: &str) -> Result<(), AuthError> {
        let mut tx = self.pool().begin().await?;
        match users::by_email(&mut *tx, email).await? {
            Some((user, _)) if !user.is_verified() && !user.is_disabled() => {
                match self.queue_verification(&mut tx, &user).await {
                    // An account past its links for now is sent nothing more.
                    Err(AuthError::RateLimited { .. }) => return Ok(()),
                    queued => queued?,
                }
                tx.commit().await?;
                info!(user_id = %user.id, "verification link mailed again");
            }
            _ => {}
        }
        Ok(())
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
                return Err(AuthError::MailNotSetUp(self.mail_status().problem));
            }
            let mut tx = self.pool().begin().await?;
            let user = users::by_id(&mut *tx, user_id)
                .await?
                .ok_or(AuthError::NotFound("no such user"))?;
            if user.is_verified() {
                return Err(AuthError::InvalidInput(
                    "the address is verified already".to_owned(),
                ));
            }
            self.queue_verification(&mut tx, &user).await?;
            tx.commit().await?;
            info!(actor_id = %actor.user.id, %user_id, "verification link mailed");
            Ok(())
        })
        .await
    }
}
