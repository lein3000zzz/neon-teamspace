//! Signing in and out, and recognising a session or an API token on each request.

use neon_domain::auth::email::{self, MAX_EMAIL_LENGTH};
use neon_domain::auth::password::MAX_PASSWORD_BYTES;
use neon_domain::auth::session::{TOUCH_INTERVAL, truncate_user_agent};
use neon_domain::auth::token::{API_TOKEN_PREFIX, effective_scopes};
use neon_domain::auth::{Credential, Principal, Session, User};
use neon_domain::settings::Secret;
use neon_store::{api_tokens, sessions, users};
use tracing::{info, warn};
use uuid::Uuid;

use super::{AuthError, AuthService, require_session};
use crate::crypto::{hash_secret, new_secret};
use crate::ratelimit::Bucket;

/// The longest session or API token read. Longer text matches nothing.
const MAX_CREDENTIAL: usize = 128;

#[derive(Debug, Clone)]
pub struct LoginInput {
    pub email: String,
    pub password: Secret,
    /// The client's address, for rate limits and the session list.
    pub ip: String,
    pub user_agent: String,
}

/// A new session. Its token exists only here and in the cookie it goes into.
#[derive(Debug, Clone)]
pub struct LoginOutcome {
    pub user: User,
    pub session: Session,
    pub token: Secret,
}

impl AuthService {
    /// Checks the credentials and opens a session. An unknown address and a wrong
    /// password give the same error and take the same time; a disabled or unverified
    /// account is only told so after the right password.
    pub async fn login(&self, input: LoginInput) -> Result<LoginOutcome, AuthError> {
        self.observe("login", self.login_inner(input)).await
    }

    async fn login_inner(&self, input: LoginInput) -> Result<LoginOutcome, AuthError> {
        let email = email::normalize(&input.email);
        // The IP first, so a client it stops costs nothing more, nor fills the limiter.
        self.limit(Bucket::LoginIp, &input.ip).await?;
        if email.len() > MAX_EMAIL_LENGTH || input.password.expose().len() > MAX_PASSWORD_BYTES {
            // No account has them.
            return Err(AuthError::InvalidCredentials);
        }
        // Few guesses at an address from one IP, more from every IP together: a stranger
        // cannot lock its owner out, nor guess on from many places.
        self.limit(Bucket::LoginIpEmail, &format!("{} {email}", input.ip))
            .await?;
        self.limit(Bucket::LoginEmail, &email).await?;

        let Some((user, hash)) = users::by_email(self.pool(), &email).await? else {
            self.verify_dummy(&input.password).await;
            warn!(ip = %input.ip, reason = "unknown email", "sign-in failed");
            return Err(AuthError::InvalidCredentials);
        };
        let verified = self.verify_password(&input.password, hash.clone()).await?;
        if !verified.matches {
            warn!(ip = %input.ip, user_id = %user.id, reason = "wrong password", "sign-in failed");
            return Err(AuthError::InvalidCredentials);
        }
        if user.is_disabled() {
            return Err(AuthError::UserDisabled);
        }
        if !user.is_verified() {
            info!(user_id = %user.id, "sign-in refused: the address is not verified");
            return Err(AuthError::EmailNotVerified);
        }
        // The hash the password was checked against. The writes below happen only while
        // the account still has it, so a reset or change of the password in the meantime
        // is neither undone by the upgrade nor outlived by the new session.
        let hash = if verified.needs_rehash {
            self.rehash(&user, &input.password, hash).await
        } else {
            hash
        };

        let token = new_secret("");
        let now = self.now();
        let expiry = self.session_policy().open(now);
        let session = Session {
            id: Uuid::now_v7(),
            user_id: user.id,
            user_agent: truncate_user_agent(&input.user_agent),
            ip: input.ip,
            created_at: now,
            last_seen_at: now,
            expires_at: expiry.expires_at,
            absolute_expires_at: expiry.absolute_expires_at,
        };
        if !sessions::insert(self.pool(), &session, &hash_secret(&token), &hash).await? {
            warn!(
                ip = %session.ip,
                user_id = %user.id,
                reason = "the password changed or the account was disabled meanwhile",
                "sign-in failed"
            );
            return Err(AuthError::InvalidCredentials);
        }
        info!(user_id = %user.id, session_id = %session.id, ip = %session.ip, "signed in");
        Ok(LoginOutcome {
            user,
            session,
            token: Secret::new(token),
        })
    }

    /// Hashes a password again with the current parameters, in place of `old` if the
    /// account still has it, and returns the hash the account has now as far as this
    /// sign-in knows. A failure only costs the upgrade, so it is logged rather than failing
    /// the sign-in.
    async fn rehash(&self, user: &User, password: &Secret, old: String) -> String {
        let new = match self.hash_password(password).await {
            Ok(new) => new,
            Err(error) => {
                warn!(user_id = %user.id, %error, "upgrading the password hash failed");
                return old;
            }
        };
        match users::upgrade_password_hash(self.pool(), user.id, &old, &new).await {
            Ok(true) => {
                info!(user_id = %user.id, "password hash upgraded");
                new
            }
            Ok(false) => {
                info!(user_id = %user.id, "password hash not upgraded: the password changed");
                old
            }
            Err(error) => {
                warn!(user_id = %user.id, %error, "upgrading the password hash failed");
                old
            }
        }
    }

    /// Ends a session. One already gone is not an error.
    pub async fn logout(&self, session_id: Uuid) -> Result<(), AuthError> {
        self.observe("logout", async {
            if sessions::delete(self.pool(), session_id).await? {
                info!(%session_id, "signed out");
            }
            Ok(())
        })
        .await
    }

    /// The principal of a session token, sliding its idle expiry forward. An unknown or
    /// expired session, and one whose account is disabled, are all just unauthenticated.
    pub async fn authenticate_session(&self, token: &str) -> Result<Principal, AuthError> {
        self.observe("authenticate_session", async {
            if token.is_empty() || token.len() > MAX_CREDENTIAL {
                return Err(AuthError::Unauthenticated);
            }
            let found = sessions::by_token_hash(self.pool(), &hash_secret(token)).await?;
            let Some((session, user)) = found else {
                return Err(AuthError::Unauthenticated);
            };
            let now = self.now();
            if session.is_expired(now) {
                if let Err(error) = sessions::delete(self.pool(), session.id).await {
                    warn!(session_id = %session.id, %error, "deleting an expired session failed");
                }
                return Err(AuthError::Unauthenticated);
            }
            if user.is_disabled() {
                return Err(AuthError::Unauthenticated);
            }
            if let Some(expires_at) = self.session_policy().touch(&session, now)
                && let Err(error) = sessions::touch(self.pool(), session.id, now, expires_at).await
            {
                warn!(session_id = %session.id, %error, "recording a session's use failed");
            }
            Ok(Principal {
                user,
                credential: Credential::Session { id: session.id },
            })
        })
        .await
    }

    /// The principal of an API token, with the scopes its owner's role allows now.
    pub async fn authenticate_token(&self, token: &str) -> Result<Principal, AuthError> {
        self.observe("authenticate_token", async {
            if !token.starts_with(API_TOKEN_PREFIX) || token.len() > MAX_CREDENTIAL {
                return Err(AuthError::Unauthenticated);
            }
            let found = api_tokens::by_token_hash(self.pool(), &hash_secret(token)).await?;
            let Some((api_token, user)) = found else {
                return Err(AuthError::Unauthenticated);
            };
            let now = self.now();
            if api_token.is_expired(now) || user.is_disabled() {
                return Err(AuthError::Unauthenticated);
            }
            let due = api_token
                .last_used_at
                .is_none_or(|last| now - last >= TOUCH_INTERVAL);
            if due && let Err(error) = api_tokens::touch(self.pool(), api_token.id, now).await {
                warn!(token_id = %api_token.id, %error, "recording a token's use failed");
            }
            let scopes = effective_scopes(&api_token.scopes, user.role);
            Ok(Principal {
                user,
                credential: Credential::Token {
                    id: api_token.id,
                    name: api_token.name,
                    scopes,
                },
            })
        })
        .await
    }

    /// The principal's sessions that have not expired, newest first.
    pub async fn list_sessions(&self, principal: &Principal) -> Result<Vec<Session>, AuthError> {
        self.observe("list_sessions", async {
            Ok(sessions::list_for_user(self.pool(), principal.user.id, self.now()).await?)
        })
        .await
    }

    /// Ends one of the principal's sessions, the current one included.
    pub async fn revoke_session(&self, principal: &Principal, id: Uuid) -> Result<(), AuthError> {
        self.observe("revoke_session", async {
            require_session(principal)?;
            if !sessions::delete_of_user(self.pool(), principal.user.id, id).await? {
                return Err(AuthError::NotFound("no such session"));
            }
            info!(user_id = %principal.user.id, session_id = %id, "session revoked");
            Ok(())
        })
        .await
    }

    /// Ends every session of the principal but the current one, and says how many ended.
    pub async fn revoke_other_sessions(&self, principal: &Principal) -> Result<u64, AuthError> {
        self.observe("revoke_other_sessions", async {
            require_session(principal)?;
            let ended =
                sessions::delete_for_user(self.pool(), principal.user.id, principal.session_id())
                    .await?;
            info!(user_id = %principal.user.id, ended, "other sessions revoked");
            Ok(ended)
        })
        .await
    }
}
