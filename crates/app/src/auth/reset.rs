//! Setting a new password with a mailed link.

use neon_domain::auth::EmailTokenPurpose;
use neon_domain::auth::email::normalize;
use neon_domain::auth::email_token::MAX_LINK_TOKEN;
use neon_domain::settings::Secret;
use neon_store::{email_tokens, sessions, users};
use tracing::{info, warn};

use super::verification::recipient;
use super::{AuthError, AuthService};
use crate::crypto::hash_secret;
use crate::mail::{Mail, Message};
use crate::ratelimit::Bucket;

impl AuthService {
    /// Mails a link setting a new password to the account of an address. It answers the
    /// same whether there is one or not, and as fast: the account is looked up and mailed
    /// after the answer. A disabled account is sent nothing, and an account past its links
    /// for the day gets none more.
    pub async fn request_password_reset(&self, email: &str, ip: &str) -> Result<(), AuthError> {
        self.observe("request_password_reset", async {
            self.limit(Bucket::Reset, ip).await?;
            if !self.password_reset_available() {
                return Err(AuthError::PasswordResetOff);
            }
            let service = self.clone();
            let email = normalize(email);
            self.background("password_reset_mail", async move {
                if let Err(error) = service.mail_reset(&email).await {
                    warn!(%error, "no reset link mailed");
                }
            });
            Ok(())
        })
        .await
    }

    async fn mail_reset(&self, email: &str) -> Result<(), AuthError> {
        let mut tx = self.pool().begin().await?;
        let user = match users::by_email(&mut *tx, email).await? {
            Some((user, _)) if !user.is_disabled() => user,
            _ => return Ok(()),
        };
        let token = match self
            .new_link(&mut tx, &user, EmailTokenPurpose::Reset)
            .await
        {
            Ok(token) => token,
            // An account past its links for now is sent nothing more.
            Err(AuthError::RateLimited { .. }) => return Ok(()),
            Err(error) => return Err(error),
        };
        let mail = Mail {
            to: recipient(&user),
            message: Message::PasswordReset {
                token,
                lasts: EmailTokenPurpose::Reset.lifetime(),
            },
        };
        self.queue(&mut tx, &mail).await?;
        tx.commit().await?;
        info!(user_id = %user.id, "password reset link mailed");
        Ok(())
    }

    /// Sets the password of the account a mailed link is for, and returns its address for
    /// the sign-in form. The link works once. It verifies the address, signs the account
    /// out of every session, and the account is told by mail, queued with the change. A
    /// weak password leaves the link working.
    pub async fn reset_password(
        &self,
        token: &Secret,
        password: &Secret,
        ip: &str,
    ) -> Result<String, AuthError> {
        self.observe("reset_password", async {
            self.limit(Bucket::Reset, ip).await?;
            if !self.password_reset_available() {
                return Err(AuthError::PasswordResetOff);
            }
            let token = token.expose().trim();
            if token.is_empty() || token.len() > MAX_LINK_TOKEN {
                return Err(AuthError::InvalidLink);
            }
            self.policy().check(password.expose())?;
            let hash = self.hash_password(password).await?;

            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let user_id =
                email_tokens::take(&mut *tx, &hash_secret(token), EmailTokenPurpose::Reset, now)
                    .await?
                    .ok_or(AuthError::InvalidLink)?;
            let user = users::by_id(&mut *tx, user_id)
                .await?
                .filter(|user| !user.is_disabled())
                .ok_or(AuthError::InvalidLink)?;
            users::set_password_hash(&mut *tx, user_id, &hash, now).await?;
            users::set_email_verified(&mut *tx, user_id, now).await?;
            email_tokens::use_all(&mut *tx, user_id, EmailTokenPurpose::Reset, now).await?;
            let ended = sessions::delete_for_user(&mut *tx, user_id, None).await?;
            self.tell_password_changed(&mut tx, &user, now).await?;
            tx.commit().await?;
            info!(%user_id, ip, sessions_ended = ended, "password reset");
            if ended > 0 {
                self.ended(user_id);
            }
            Ok(user.address().to_owned())
        })
        .await
    }
}
