//! The admin's work on accounts, and the test mail.

use neon_domain::auth::{Principal, Role, User};
use neon_store::{sessions, users};
use tracing::info;
use uuid::Uuid;

use super::verification::recipient;
use super::{AuthError, AuthService, require_admin};
use crate::mail::{Mail, Message};

/// The most accounts one page lists, and how many it lists unless told.
pub(crate) const MAX_PAGE: i64 = 200;
pub(crate) const DEFAULT_PAGE: i64 = 50;

/// A page of accounts in the order they were made.
#[derive(Debug, Clone)]
pub struct UserPage {
    pub users: Vec<User>,
    /// Pass it as `after` for the next page; unset on the last one.
    pub next: Option<Uuid>,
}

/// What an admin changes about an account. Each part left out stays as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserUpdate {
    pub role: Option<Role>,
    pub disabled: Option<bool>,
    /// True marks the address verified, the admin's word for it. An address cannot be
    /// unverified.
    pub email_verified: Option<bool>,
}

impl AuthService {
    pub async fn list_users(
        &self,
        actor: &Principal,
        after: Option<Uuid>,
        limit: Option<i64>,
    ) -> Result<UserPage, AuthError> {
        self.observe("list_users", async {
            require_admin(actor)?;
            let limit = limit.unwrap_or(DEFAULT_PAGE);
            if !(1..=MAX_PAGE).contains(&limit) {
                return Err(AuthError::InvalidInput(format!(
                    "a page lists 1 to {MAX_PAGE} accounts"
                )));
            }
            let mut listed = users::list(self.pool(), after, limit + 1).await?;
            let more = listed.len() > usize::try_from(limit).unwrap_or(usize::MAX);
            listed.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            let next = more.then(|| listed.last().map(|user| user.id)).flatten();
            Ok(UserPage {
                users: listed,
                next,
            })
        })
        .await
    }

    /// Changes an account's role, whether it is disabled, or marks its address verified.
    /// Disabling it signs it out everywhere. The last admin who can sign in can be neither
    /// demoted nor disabled, so the instance never locks itself out: every such change
    /// locks the active admins first, so changes run one at a time and each sees the
    /// admins as the one before left them.
    ///
    /// A change of role or of the disable needs a browser session. With an API token,
    /// only verifying the address goes through, so a token that leaked can neither make
    /// an admin of an account its holder controls nor lock admins out.
    pub async fn update_user(
        &self,
        actor: &Principal,
        id: Uuid,
        update: UserUpdate,
    ) -> Result<User, AuthError> {
        self.observe("update_user", async {
            require_admin(actor)?;
            if update.email_verified == Some(false) {
                return Err(AuthError::InvalidInput(
                    "an address cannot be unverified".to_owned(),
                ));
            }

            let now = self.now();
            let mut tx = self.pool().begin().await?;
            let admins = users::lock_active_admins(&mut *tx).await?;
            let mut user = users::by_id_for_update(&mut *tx, id)
                .await?
                .ok_or(AuthError::NotFound("no such user"))?;
            let was_active_admin = user.is_active_admin();
            let changes_access = update.role.is_some_and(|role| role != user.role)
                || update
                    .disabled
                    .is_some_and(|disabled| disabled != user.is_disabled());
            if changes_access && !actor.via_session() {
                return Err(AuthError::Forbidden(
                    "changing a role, or disabling or enabling an account, needs a browser \
                     session: API tokens may only verify addresses here",
                ));
            }

            if let Some(role) = update.role {
                user.role = role;
            }
            match update.disabled {
                Some(true) if !user.is_disabled() => user.disabled_at = Some(now),
                Some(false) => user.disabled_at = None,
                _ => {}
            }
            if was_active_admin && !user.is_active_admin() && admins.len() <= 1 {
                return Err(AuthError::LastAdmin);
            }
            user.updated_at = now;
            users::update(&mut *tx, &user).await?;
            if update.email_verified == Some(true) && !user.is_verified() {
                users::set_email_verified(&mut *tx, user.id, now).await?;
                user.email_verified_at = Some(now);
            }
            let ended = if user.is_disabled() {
                sessions::delete_for_user(&mut *tx, user.id, None).await?
            } else {
                0
            };
            tx.commit().await?;
            info!(
                actor_id = %actor.user.id,
                user_id = %user.id,
                role = %user.role,
                disabled = user.is_disabled(),
                verified = user.is_verified(),
                sessions_ended = ended,
                "account updated"
            );
            if changes_access {
                self.ended(user.id);
            }
            Ok(user)
        })
        .await
    }

    /// Queues a test mail to the admin asking, to see whether mail gets through, and
    /// returns where it went. Whether it was delivered shows in the mail status, and in the
    /// admin's inbox.
    pub async fn send_test_mail(&self, actor: &Principal) -> Result<String, AuthError> {
        self.observe("send_test_mail", async {
            require_admin(actor)?;
            if !self.mail_configured() {
                return Err(AuthError::MailNotSetUp(self.mail_status().problem));
            }
            let mail = Mail {
                to: recipient(&actor.user),
                message: Message::Test {
                    sent_by: actor.user.address().to_owned(),
                },
            };
            let mut conn = self.pool().acquire().await?;
            self.queue(&mut conn, &mail).await?;
            info!(actor_id = %actor.user.id, "test mail queued");
            Ok(mail.to.address)
        })
        .await
    }
}
