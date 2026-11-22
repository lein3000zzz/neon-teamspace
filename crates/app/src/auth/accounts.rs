//! Making accounts, and what their owners change about them.

use neon_domain::auth::invite::{MAX_CODE_INPUT, lookup_form};
use neon_domain::auth::{
    Email, Invite, Preferences, Principal, RegistrationMode, Role, User, display_name,
};
use neon_domain::hue::check_hue;
use neon_domain::settings::Secret;
use neon_store::{invites, is_unique_violation, sessions, users};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::info;
use uuid::Uuid;

use super::verification::recipient;
use super::{AuthError, AuthService, registration, require_admin, require_session};
use crate::crypto::hash_secret;
use crate::mail::{Mail, Message};
use crate::ratelimit::Bucket;

#[derive(Debug, Clone)]
pub struct RegisterInput {
    pub email: String,
    pub password: Secret,
    /// The local part of the address when not given.
    pub display_name: Option<String>,
    /// Needed while registration is by invite.
    pub invite_code: Option<Secret>,
    pub ip: String,
}

/// What signing up did.
#[derive(Debug, Clone)]
pub struct Registration {
    pub user: User,
    /// The account signs in only once it has opened the link mailed to it, which was
    /// queued with the account.
    pub must_verify: bool,
}

/// An account an admin, the bootstrap configuration or `neon admin create` makes.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub password: Secret,
    pub display_name: Option<String>,
    pub role: Role,
}

impl AuthService {
    /// Makes an account as the registration mode allows: anyone while it is open, those
    /// with an invite while it is by invite, nobody while it is closed. An invite bound to
    /// an address admits only that address, which counts as verified then. While sign-ups
    /// verify, any other account must open a mailed link before it signs in. An account of
    /// the address that never verified is replaced, so nobody holds an address that is not
    /// theirs. The invite's use, the account and its link's mail are saved together.
    pub async fn register(&self, input: RegisterInput) -> Result<Registration, AuthError> {
        self.observe("register", self.register_inner(input)).await
    }

    async fn register_inner(&self, input: RegisterInput) -> Result<Registration, AuthError> {
        self.limit(Bucket::Register, &input.ip).await?;
        let now = self.now();
        let invite = match registration(&self.settings()) {
            RegistrationMode::Closed => return Err(AuthError::RegistrationClosed),
            RegistrationMode::Invite => {
                Some(self.open_invite(input.invite_code.as_ref(), now).await?)
            }
            RegistrationMode::Open => None,
        };

        let email = Email::parse(&input.email)?;
        let name =
            display_name(input.display_name.as_deref(), &email).map_err(AuthError::invalid)?;
        self.policy().check(input.password.expose())?;
        let bound = invite.as_ref().and_then(|invite| invite.email.as_deref());
        if bound.is_some_and(|bound| !email.same_as(bound)) {
            return Err(AuthError::InviteForOtherEmail);
        }
        let hash = self.hash_password(&input.password).await?;

        // A mailed invite proved the address already.
        let must_verify = self.verifying() && bound.is_none();
        let user = User {
            id: Uuid::now_v7(),
            email,
            display_name: name,
            role: Role::User,
            hue_override: None,
            email_verified_at: (!must_verify).then_some(now),
            disabled_at: None,
            created_at: now,
            updated_at: now,
        };

        let mut tx = self.pool().begin().await?;
        replace_unverified(&mut tx, &user.email).await?;
        users::insert(&mut *tx, &user, &hash)
            .await
            .map_err(email_taken)?;
        if let Some(invite) = &invite
            && !invites::consume(&mut tx, invite.id, user.id, now).await?
        {
            return Err(AuthError::InvalidInvite);
        }
        if must_verify {
            self.queue_verification(&mut tx, &user).await?;
        }
        tx.commit().await?;
        info!(
            user_id = %user.id,
            ip = %input.ip,
            invite_id = ?invite.as_ref().map(|invite| invite.id),
            must_verify,
            "account registered"
        );
        Ok(Registration { user, must_verify })
    }

    async fn open_invite(
        &self,
        code: Option<&Secret>,
        now: OffsetDateTime,
    ) -> Result<Invite, AuthError> {
        let code = code
            .map(|code| code.expose().trim())
            .filter(|code| !code.is_empty() && code.len() <= MAX_CODE_INPUT)
            .ok_or(AuthError::InvalidInvite)?;
        invites::open_by_code(self.pool(), &hash_secret(&lookup_form(code)), now)
            .await?
            .ok_or(AuthError::InvalidInvite)
    }

    /// Makes an account for an admin, whatever the registration mode. It counts as
    /// verified: the admin vouches for the address. Only from a browser session: an API
    /// token invites people instead, so a token that leaked cannot make an account its
    /// holder signs in to at a browser, where the settings that say where secrets go
    /// change.
    pub async fn create_user(
        &self,
        actor: &Principal,
        account: NewAccount,
    ) -> Result<User, AuthError> {
        self.observe("create_user", async {
            require_admin(actor)?;
            if !actor.via_session() {
                return Err(AuthError::Forbidden(
                    "API tokens invite people rather than set their passwords: make an \
                     invite, or create the account signed in at a browser",
                ));
            }
            let user = self.create_account(account).await?;
            info!(actor_id = %actor.user.id, user_id = %user.id, role = %user.role, "account created");
            Ok(user)
        })
        .await
    }

    /// Makes an admin unless an admin who can sign in exists, for the first start. `None`
    /// when one exists.
    pub async fn ensure_admin(
        &self,
        email: &str,
        password: &Secret,
    ) -> Result<Option<User>, AuthError> {
        self.observe("ensure_admin", async {
            if users::count_active_admins(self.pool()).await? > 0 {
                return Ok(None);
            }
            let user = self
                .create_account(NewAccount {
                    email: email.to_owned(),
                    password: password.clone(),
                    display_name: None,
                    role: Role::Admin,
                })
                .await?;
            info!(user_id = %user.id, "first admin created");
            Ok(Some(user))
        })
        .await
    }

    /// Makes an admin with no acting user, for operators: `neon admin create`.
    pub async fn bootstrap_admin(&self, account: NewAccount) -> Result<User, AuthError> {
        self.observe("bootstrap_admin", async {
            let user = self
                .create_account(NewAccount {
                    role: Role::Admin,
                    ..account
                })
                .await?;
            info!(user_id = %user.id, "admin created");
            Ok(user)
        })
        .await
    }

    async fn create_account(&self, account: NewAccount) -> Result<User, AuthError> {
        let email = Email::parse(&account.email)?;
        let name =
            display_name(account.display_name.as_deref(), &email).map_err(AuthError::invalid)?;
        self.policy().check(account.password.expose())?;
        let hash = self.hash_password(&account.password).await?;
        let now = self.now();
        let user = User {
            id: Uuid::now_v7(),
            email,
            display_name: name,
            role: account.role,
            hue_override: None,
            email_verified_at: Some(now),
            disabled_at: None,
            created_at: now,
            updated_at: now,
        };
        users::insert(self.pool(), &user, &hash)
            .await
            .map_err(email_taken)?;
        Ok(user)
    }

    /// Changes the principal's password, given the current one, and signs out every other
    /// session. The account is told by mail, queued with the change. Only from a browser
    /// session.
    pub async fn change_password(
        &self,
        principal: &Principal,
        current: &Secret,
        next: &Secret,
    ) -> Result<(), AuthError> {
        self.observe("change_password", async {
            require_session(principal)?;
            let user = &principal.user;
            self.limit(Bucket::PasswordChange, &user.id.to_string())
                .await?;
            let hash = users::password_hash(self.pool(), user.id)
                .await?
                .ok_or(AuthError::Unauthenticated)?;
            if !self.verify_password(current, hash).await?.matches {
                return Err(AuthError::InvalidCredentials);
            }
            self.policy().check(next.expose())?;
            let hash = self.hash_password(next).await?;

            let now = self.now();
            let mut tx = self.pool().begin().await?;
            users::set_password_hash(&mut *tx, user.id, &hash, now).await?;
            let ended =
                sessions::delete_for_user(&mut *tx, user.id, principal.session_id()).await?;
            self.tell_password_changed(&mut tx, user, now).await?;
            tx.commit().await?;
            info!(user_id = %user.id, other_sessions_ended = ended, "password changed");
            Ok(())
        })
        .await
    }

    /// Queues a mail on `conn` telling an account its password changed, while mail is set
    /// up.
    pub(super) async fn tell_password_changed(
        &self,
        conn: &mut PgConnection,
        user: &User,
        at: OffsetDateTime,
    ) -> Result<(), AuthError> {
        if !self.mail_configured() {
            return Ok(());
        }
        let mail = Mail {
            to: recipient(user),
            message: Message::PasswordChanged { at },
        };
        self.queue(conn, &mail).await
    }

    /// Renames the principal's account, and returns it as it is now. Only the name is
    /// written: the principal was read as the request began, and its role and disable may
    /// have changed since.
    pub async fn update_profile(
        &self,
        principal: &Principal,
        name: &str,
    ) -> Result<User, AuthError> {
        self.observe("update_profile", async {
            let user = &principal.user;
            let name = display_name(Some(name), &user.email).map_err(AuthError::invalid)?;
            users::rename(self.pool(), user.id, &name, self.now())
                .await?
                .ok_or(AuthError::Unauthenticated)
        })
        .await
    }

    /// Pins the hue the principal is drawn in, 0 to 359, or with `None` goes back to the one
    /// their id hashes to, and returns their account as it is now.
    pub async fn set_hue(
        &self,
        principal: &Principal,
        hue: Option<i64>,
    ) -> Result<User, AuthError> {
        self.observe("set_hue", async {
            let hue = hue.map(check_hue).transpose().map_err(AuthError::invalid)?;
            users::set_hue(self.pool(), principal.user.id, hue, self.now())
                .await?
                .ok_or(AuthError::Unauthenticated)
        })
        .await
    }

    /// The principal's preferences; defaults for what they never set.
    pub async fn preferences(&self, principal: &Principal) -> Result<Preferences, AuthError> {
        self.observe("preferences", async {
            let stored = users::preferences(self.pool(), principal.user.id)
                .await?
                .ok_or(AuthError::Unauthenticated)?;
            Ok(Preferences::from_stored(&stored))
        })
        .await
    }

    pub async fn set_preferences(
        &self,
        principal: &Principal,
        preferences: Preferences,
    ) -> Result<Preferences, AuthError> {
        self.observe("set_preferences", async {
            users::set_preferences(
                self.pool(),
                principal.user.id,
                &preferences.to_stored(),
                self.now(),
            )
            .await?;
            Ok(preferences)
        })
        .await
    }
}

/// Deletes the account of an address that never verified it, in the transaction that
/// makes the new one, so signing up again takes the address back.
pub(super) async fn replace_unverified(
    tx: &mut PgConnection,
    email: &Email,
) -> Result<(), AuthError> {
    if let Some((old, _)) = users::by_email(&mut *tx, email.normalized()).await?
        && !old.is_verified()
        && users::delete_unverified(&mut *tx, old.id).await?
    {
        info!(user_id = %old.id, "unverified account replaced");
    }
    Ok(())
}

/// An insert that broke the unique address is an address taken.
pub(super) fn email_taken(error: sqlx::Error) -> AuthError {
    if is_unique_violation(&error, users::EMAIL_TAKEN) {
        AuthError::EmailTaken
    } else {
        AuthError::Database(error)
    }
}
