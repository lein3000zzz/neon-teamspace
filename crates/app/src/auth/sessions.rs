//! Signing in and out, and recognising a session, an API token or an OAuth access token on
//! each request.

use neon_domain::agents::IdentityMode;
use neon_domain::auth::email::{self, MAX_EMAIL_LENGTH};
use neon_domain::auth::password::MAX_PASSWORD_BYTES;
use neon_domain::auth::session::{TOUCH_INTERVAL, truncate_user_agent};
use neon_domain::auth::token::API_TOKEN_PREFIX;
use neon_domain::auth::{Credential, Principal, Session, User};
use neon_domain::oauth::ACCESS_TOKEN_PREFIX;
use neon_domain::settings::Secret;
use neon_store::api_tokens::Found;
use neon_store::{api_tokens, sessions, users};
use tracing::{info, warn};
use uuid::Uuid;

use super::{AuthError, AuthService, require_session};
use crate::crypto::{hash_secret, new_secret};
use crate::oauth::access;
use crate::ratelimit::Bucket;
use crate::settings::mcp;

/// The longest session, API token or OAuth access token read. Longer text matches nothing.
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
            if let Some(user_id) = sessions::delete(self.pool(), session_id).await? {
                info!(%session_id, "signed out");
                self.ended(user_id);
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

    /// The principal as its session or API token stands now, read again: with its
    /// account's current role and, for a token, the scopes that role allows. Answers
    /// unauthenticated once the session or token ended or expired, the account was
    /// disabled, or an agent's token stopped working. It does not count as a use: a
    /// session's idle expiry stays.
    pub async fn refresh(&self, principal: &Principal) -> Result<Principal, AuthError> {
        self.observe("refresh", async {
            let now = self.now();
            match &principal.credential {
                Credential::Session { id } => {
                    let found = sessions::by_id(self.pool(), *id).await?;
                    match found {
                        Some((session, user))
                            if !session.is_expired(now) && !user.is_disabled() =>
                        {
                            Ok(Principal {
                                user,
                                credential: Credential::Session { id: session.id },
                            })
                        }
                        _ => Err(AuthError::Unauthenticated),
                    }
                }
                Credential::Token { id, .. } => match api_tokens::by_id(self.pool(), *id).await? {
                    Some(found) if self.token_works(&found, now) => Ok(token_principal(found)),
                    Some(_) => Err(AuthError::Unauthenticated),
                    // Not an API token: an OAuth access token, or nothing any more.
                    None => access::recheck(self.pool(), &self.settings(), *id, now)
                        .await?
                        .ok_or(AuthError::Unauthenticated),
                },
            }
        })
        .await
    }

    /// The principal of an API token, with the scopes its owner's role allows now: a
    /// person, or an agent, acting as itself for its owner. An agent's token works only
    /// while the agent does: not deleted, its owner a member of its workspace who can sign
    /// in, and agents on, on the instance and in the workspace.
    ///
    /// An OAuth access token gives the same principal as an API token bound to its grant's
    /// workspace, acting as the person who consented or the agent they chose, while the
    /// grant works; see [`oauth`](crate::oauth).
    pub async fn authenticate_token(&self, token: &str) -> Result<Principal, AuthError> {
        self.observe("authenticate_token", async {
            if token.len() > MAX_CREDENTIAL {
                return Err(AuthError::Unauthenticated);
            }
            if token.starts_with(ACCESS_TOKEN_PREFIX) {
                return access::authenticate(self.pool(), &self.settings(), token, self.now())
                    .await?
                    .ok_or(AuthError::Unauthenticated);
            }
            if !token.starts_with(API_TOKEN_PREFIX) {
                return Err(AuthError::Unauthenticated);
            }
            let found = api_tokens::by_token_hash(self.pool(), &hash_secret(token)).await?;
            let Some(found) = found else {
                return Err(AuthError::Unauthenticated);
            };
            let now = self.now();
            if !self.token_works(&found, now) {
                return Err(AuthError::Unauthenticated);
            }
            let token = &found.token;
            let due = token
                .last_used_at
                .is_none_or(|last| now - last >= TOUCH_INTERVAL);
            if due && let Err(error) = api_tokens::touch(self.pool(), token.id, now).await {
                warn!(token_id = %token.id, %error, "recording a token's use failed");
            }
            Ok(token_principal(found))
        })
        .await
    }

    /// Whether a token works now: unexpired, its person not disabled, and its agent
    /// working, bound to the agent's own workspace.
    fn token_works(&self, found: &Found, now: time::OffsetDateTime) -> bool {
        if found.token.is_expired(now) || found.user.is_disabled() {
            return false;
        }
        if found.user.kind.is_human() {
            return true;
        }
        let agents_on = IdentityMode::Agent.among(self.settings().get(mcp::IDENTITIES));
        found.token.workspace_id.is_some()
            && found.token.workspace_id == found.user.workspace_id
            && found.standing.status(found.user.kind, agents_on).works()
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
            self.ended(principal.user.id);
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
            if ended > 0 {
                self.ended(principal.user.id);
            }
            Ok(ended)
        })
        .await
    }
}

/// The principal of a token that works, with the scopes its owner's role allows now.
fn token_principal(found: Found) -> Principal {
    let scopes = found.token.scopes_now(found.user.role);
    Principal {
        user: found.user,
        credential: Credential::Token {
            id: found.token.id,
            name: found.token.name,
            scopes,
            workspace_id: found.token.workspace_id,
        },
    }
}
