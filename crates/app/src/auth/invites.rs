//! Instance invites: made, mailed, listed, copied again, revoked, and looked up by the
//! sign-up form.

use std::collections::HashMap;

use neon_domain::auth::invite::{
    GENERATED_PREFIX, MAX_CODE_INPUT, check_expiry, check_uses, code_problem, lookup_form,
};
use neon_domain::auth::{Email, Invite, InviteUse, Principal};
use neon_domain::settings::Secret;
use neon_store::{invites, is_unique_violation, users};
use sqlx::PgConnection;
use time::OffsetDateTime;
use tracing::info;
use uuid::Uuid;

use super::{AuthError, AuthService, require_admin};
use crate::crypto::{OpenError, hash_secret, new_secret};
use crate::mail::{Mail, Message, Recipient};
use crate::ratelimit::Bucket;

/// What an admin asks of a new invite. Each part left out takes its default: a code the
/// server makes, one sign-up, an end `auth.invite_ttl` from now.
#[derive(Debug, Clone, Default)]
pub struct InviteSpec {
    pub code: Option<Secret>,
    pub max_uses: Option<i32>,
    pub expires_at: Option<OffsetDateTime>,
}

/// A new invite with its code, shown to the admin who made it.
#[derive(Debug, Clone)]
pub struct CreatedInvite {
    pub invite: Invite,
    pub code: Secret,
}

/// An invite as admins list it: without its code, with who signed up with it.
#[derive(Debug, Clone)]
pub struct InviteEntry {
    pub invite: Invite,
    /// Oldest first.
    pub signups: Vec<InviteUse>,
}

impl AuthService {
    /// Makes an invite to hand out as a code or a link.
    pub async fn create_invite(
        &self,
        actor: &Principal,
        spec: InviteSpec,
    ) -> Result<CreatedInvite, AuthError> {
        self.observe("create_invite", async {
            require_admin(actor)?;
            let code = self.invite_code(spec.code.as_ref()).await?;
            let uses = check_uses(spec.max_uses).map_err(AuthError::invalid)?;
            let now = self.now();
            let expires_at = check_expiry(spec.expires_at, self.invite_ttl(), now)
                .map_err(AuthError::invalid)?;
            let mut conn = self.pool().acquire().await?;
            self.new_invite(&mut conn, actor, code, None, uses, expires_at)
                .await
        })
        .await
    }

    /// Makes an invite that only `email` signs up with, once, and mails it there, naming
    /// the admin, the mail queued with the invite. An address with a verified account is
    /// refused; one whose account never verified is not, as signing up replaces that.
    pub async fn invite_by_email(
        &self,
        actor: &Principal,
        email: &str,
    ) -> Result<CreatedInvite, AuthError> {
        self.observe("invite_by_email", async {
            require_admin(actor)?;
            if !self.invite_email_available() {
                return Err(AuthError::InviteEmailOff);
            }
            let email = Email::parse(email)?;
            self.refuse_existing_account(&email).await?;
            let now = self.now();
            let code = Secret::new(new_secret(GENERATED_PREFIX));
            let mut tx = self.pool().begin().await?;
            let created = self
                .new_invite(
                    &mut tx,
                    actor,
                    code,
                    Some(email.as_str().to_owned()),
                    1,
                    now + self.invite_ttl(),
                )
                .await?;
            self.queue_invite(&mut tx, actor, &email, &created).await?;
            tx.commit().await?;
            info!(actor_id = %actor.user.id, invite_id = %created.invite.id, "invite mailed");
            Ok(created)
        })
        .await
    }

    /// Says what keeps a typed code from making an invite, as the admin types it: its form,
    /// or another invite having it. `None` when it can, or when none was typed.
    pub async fn check_invite_code(
        &self,
        actor: &Principal,
        code: &Secret,
    ) -> Result<Option<&'static str>, AuthError> {
        self.observe("check_invite_code", async {
            require_admin(actor)?;
            let code = code.expose().trim();
            if code.is_empty() {
                return Ok(None);
            }
            if let Some(problem) = code_problem(code) {
                return Ok(Some(problem));
            }
            let taken = invites::code_taken(self.pool(), &hash_secret(&lookup_form(code))).await?;
            Ok(taken.then_some("another invite has this code"))
        })
        .await
    }

    /// Every invite, newest first, with who signed up with it.
    pub async fn list_invites(&self, actor: &Principal) -> Result<Vec<InviteEntry>, AuthError> {
        self.observe("list_invites", async {
            require_admin(actor)?;
            let listed = invites::list(self.pool()).await?;
            let mut signups: HashMap<Uuid, Vec<InviteUse>> = HashMap::new();
            for (invite_id, signup) in invites::uses(self.pool()).await? {
                signups.entry(invite_id).or_default().push(signup);
            }
            Ok(listed
                .into_iter()
                .map(|invite| InviteEntry {
                    signups: signups.remove(&invite.id).unwrap_or_default(),
                    invite,
                })
                .collect())
        })
        .await
    }

    /// An invite's code, opened from its sealed copy, for an admin to copy again.
    pub async fn invite_code_of(&self, actor: &Principal, id: Uuid) -> Result<Secret, AuthError> {
        self.observe("invite_code", async {
            require_admin(actor)?;
            let sealed = invites::sealed_code(self.pool(), id)
                .await?
                .ok_or(AuthError::NotFound("no such invite"))?
                .ok_or_else(|| {
                    AuthError::CodeUnavailable(
                        "the code of this invite was not kept: no secret key was configured when \
                         it was made"
                            .to_owned(),
                    )
                })?;
            let code =
                self.inner.envelope.open(&sealed, &invite_aad(id)).map_err(
                    |error| match error {
                        OpenError::NoKey | OpenError::UnknownKey(_) => AuthError::CodeUnavailable(
                            format!("the code cannot be opened: {error}"),
                        ),
                        OpenError::Malformed | OpenError::Decrypt => AuthError::Internal(format!(
                            "the sealed code of invite {id} does not open: {error}"
                        )),
                    },
                )?;
            let code = String::from_utf8(code.to_vec())
                .map_err(|_| AuthError::Internal(format!("the code of invite {id} is not text")))?;
            info!(actor_id = %actor.user.id, invite_id = %id, "invite code copied");
            Ok(Secret::new(code))
        })
        .await
    }

    /// Ends an invite: it admits no more sign-ups, and stays listed.
    pub async fn revoke_invite(&self, actor: &Principal, id: Uuid) -> Result<(), AuthError> {
        self.observe("revoke_invite", async {
            require_admin(actor)?;
            if !invites::revoke(self.pool(), id, actor.user.id, self.now()).await? {
                return Err(AuthError::NotFound("no such invite"));
            }
            info!(actor_id = %actor.user.id, invite_id = %id, "invite revoked");
            Ok(())
        })
        .await
    }

    /// The invite of a code while it admits sign-ups, for the sign-up form to fill in the
    /// address it is bound to.
    pub async fn invite_for(&self, code: &Secret, ip: &str) -> Result<Invite, AuthError> {
        self.observe("invite_for", async {
            self.limit_source(Bucket::InviteLookup, ip).await?;
            let code = code.expose().trim();
            if code.is_empty() || code.len() > MAX_CODE_INPUT {
                return Err(AuthError::InvalidInvite);
            }
            invites::open_by_code(self.pool(), &hash_secret(&lookup_form(code)), self.now())
                .await?
                .ok_or(AuthError::InvalidInvite)
        })
        .await
    }

    /// The code a new invite gets: the one typed, unless its form is wrong or another
    /// invite has it, or a new one.
    async fn invite_code(&self, typed: Option<&Secret>) -> Result<Secret, AuthError> {
        let typed = typed
            .map(|code| code.expose().trim())
            .filter(|code| !code.is_empty());
        let Some(code) = typed else {
            return Ok(Secret::new(new_secret(GENERATED_PREFIX)));
        };
        if let Some(problem) = code_problem(code) {
            return Err(AuthError::InvalidInput(problem.to_owned()));
        }
        if invites::code_taken(self.pool(), &hash_secret(&lookup_form(code))).await? {
            return Err(AuthError::InviteCodeTaken);
        }
        Ok(Secret::new(code))
    }

    /// Saves an invite on `conn`, its code sealed when a secret key is configured.
    pub(super) async fn new_invite(
        &self,
        conn: &mut PgConnection,
        actor: &Principal,
        code: Secret,
        email: Option<String>,
        max_uses: i32,
        expires_at: OffsetDateTime,
    ) -> Result<CreatedInvite, AuthError> {
        let id = Uuid::now_v7();
        let envelope = &self.inner.envelope;
        let sealed = if envelope.has_key() {
            let sealed = envelope
                .seal(code.expose().as_bytes(), &invite_aad(id))
                .map_err(|error| AuthError::Internal(format!("sealing an invite code: {error}")))?;
            Some(sealed)
        } else {
            None
        };
        let invite = Invite {
            id,
            created_by: Some(actor.user.id),
            email,
            created_at: self.now(),
            expires_at,
            max_uses,
            uses: 0,
            revoked_at: None,
            revoked_by: None,
            code_kept: sealed.is_some(),
        };
        invites::insert(
            &mut *conn,
            &invite,
            &hash_secret(&lookup_form(code.expose())),
            sealed.as_deref(),
        )
        .await
        .map_err(|error| {
            if is_unique_violation(&error, invites::CODE_TAKEN) {
                AuthError::InviteCodeTaken
            } else {
                AuthError::Database(error)
            }
        })?;
        info!(
            actor_id = %actor.user.id,
            invite_id = %invite.id,
            bound = invite.email.is_some(),
            max_uses,
            code_kept = invite.code_kept,
            "invite created"
        );
        Ok(CreatedInvite { invite, code })
    }

    /// Queues a bound invite's mail on `conn`, naming the admin.
    pub(super) async fn queue_invite(
        &self,
        conn: &mut PgConnection,
        actor: &Principal,
        to: &Email,
        created: &CreatedInvite,
    ) -> Result<(), AuthError> {
        let mail = Mail {
            to: Recipient::new(to.as_str(), None),
            message: Message::Invite {
                inviter_name: actor.user.display_name.clone(),
                inviter_email: actor.user.address().to_owned(),
                code: created.code.clone(),
                expires_at: created.invite.expires_at,
            },
        };
        self.queue(conn, &mail).await
    }

    /// Refuses an address whose account is verified.
    pub(super) async fn refuse_existing_account(&self, email: &Email) -> Result<(), AuthError> {
        match users::by_email(self.pool(), email.normalized()).await? {
            Some((user, _)) if user.is_verified() => Err(AuthError::EmailTaken),
            _ => Ok(()),
        }
    }
}

/// Binds a sealed code to its invite, so it does not open as another's.
fn invite_aad(id: Uuid) -> Vec<u8> {
    format!("invite:{id}").into_bytes()
}
