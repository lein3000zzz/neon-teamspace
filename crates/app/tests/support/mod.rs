//! What the auth tests share: a service on a fresh database with fakes for its ports, a
//! clock that moves by hand, and cheap password hashes.

#![allow(dead_code, reason = "each test binary uses a part of it")]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_app::auth::{AuthService, LoginInput};
use neon_app::clock::ManualTime;
use neon_app::crypto::{Argon2Params, Envelope, Keyring, PasswordHasher};
use neon_app::mail::{Mail, MailError, Mailer, Message};
use neon_app::ratelimit::{Bucket, Limited, RateLimiter};
use neon_app::settings::{Change, SettingsService, registry};
use neon_domain::auth::{Email, Principal, Role, User};
use neon_domain::settings::{Pinned, Secret};
use serde_json::Value as Json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// Every test account's password.
pub const PASSWORD: &str = "correct horse battery";

/// Hashes cheap enough for tests to make many.
pub const CHEAP: Argon2Params = Argon2Params {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
    output_len: 32,
    salt_len: 16,
};

pub const IP: &str = "203.0.113.7";

pub fn secret(text: &str) -> Secret {
    Secret::new(text)
}

/// 2026-11-22 12:00 UTC.
pub fn start() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a valid time")
}

/// Records what it was asked to send, and sends nothing.
#[derive(Default)]
pub struct FakeMailer {
    configured: AtomicBool,
    failing: AtomicBool,
    sent: Mutex<Vec<Mail>>,
}

impl FakeMailer {
    pub fn configured() -> Arc<Self> {
        let mailer = Self::default();
        mailer.configured.store(true, Ordering::SeqCst);
        Arc::new(mailer)
    }

    pub fn set_configured(&self, configured: bool) {
        self.configured.store(configured, Ordering::SeqCst);
    }

    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// The mails sent since the last call.
    pub fn take(&self) -> Vec<Mail> {
        std::mem::take(&mut *self.sent.lock().expect("mail lock"))
    }
}

#[async_trait]
impl Mailer for FakeMailer {
    fn configured(&self) -> bool {
        self.configured.load(Ordering::SeqCst)
    }

    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(MailError::Failed("connection refused".into()));
        }
        self.sent.lock().expect("mail lock").push(mail);
        Ok(())
    }
}

/// Allows every attempt except in the buckets told to refuse, and records each key.
#[derive(Default)]
pub struct ScriptedLimiter {
    refused: Mutex<HashSet<Bucket>>,
    seen: Mutex<Vec<(Bucket, String)>>,
}

impl ScriptedLimiter {
    pub fn refuse(&self, bucket: Bucket) {
        self.refused.lock().expect("lock").insert(bucket);
    }

    pub fn seen(&self) -> Vec<(Bucket, String)> {
        self.seen.lock().expect("lock").clone()
    }
}

#[async_trait]
impl RateLimiter for ScriptedLimiter {
    async fn check(&self, bucket: Bucket, key: &str) -> Result<(), Limited> {
        self.seen
            .lock()
            .expect("lock")
            .push((bucket, key.to_owned()));
        if self.refused.lock().expect("lock").contains(&bucket) {
            Err(Limited {
                retry_after: Duration::from_secs(30),
            })
        } else {
            Ok(())
        }
    }
}

pub fn envelope() -> Envelope {
    let entry = format!("k1:{}", STANDARD.encode([7u8; 32]));
    Envelope::new(Keyring::parse(&[entry]).expect("a valid key"))
}

/// A service on a fresh database, with its fakes.
pub struct Harness {
    pub pool: PgPool,
    pub settings: Arc<SettingsService>,
    pub auth: AuthService,
    pub mailer: Arc<FakeMailer>,
    pub limiter: Arc<ScriptedLimiter>,
    pub time: ManualTime,
}

pub struct Options {
    pub mailer: Arc<FakeMailer>,
    pub envelope: Envelope,
    pub params: Argon2Params,
    pub settings: Vec<(&'static str, Json)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mailer: FakeMailer::configured(),
            envelope: envelope(),
            params: CHEAP,
            settings: Vec::new(),
        }
    }
}

impl Harness {
    pub async fn new(pool: PgPool) -> Self {
        Self::with(pool, Options::default()).await
    }

    pub async fn with(pool: PgPool, options: Options) -> Self {
        let (settings, warnings) = SettingsService::load(
            pool.clone(),
            registry(),
            Pinned::default(),
            options.envelope.clone(),
        )
        .await
        .expect("settings load");
        assert!(warnings.is_empty(), "{warnings:?}");
        let settings = Arc::new(settings);
        if !options.settings.is_empty() {
            settings
                .update(
                    options
                        .settings
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), Change::Set(value))),
                    None,
                )
                .await
                .expect("valid settings");
        }
        let time = ManualTime::new(start());
        let limiter = Arc::new(ScriptedLimiter::default());
        let auth = AuthService::builder(pool.clone(), Arc::clone(&settings))
            .password_params(options.params)
            .limiter(Arc::clone(&limiter) as Arc<dyn RateLimiter>)
            .mailer(Arc::clone(&options.mailer) as Arc<dyn Mailer>)
            .envelope(options.envelope)
            .clock(time.clock())
            .build()
            .await
            .expect("the service starts");
        Self {
            pool,
            settings,
            auth,
            mailer: options.mailer,
            limiter,
            time,
        }
    }

    pub async fn set(&self, key: &str, value: Json) {
        self.settings
            .update([(key.to_owned(), Change::Set(value))], None)
            .await
            .expect("a valid setting");
    }

    /// A verified account with [`PASSWORD`], hashed with [`CHEAP`].
    pub async fn account(&self, email: &str, role: Role) -> User {
        let hash = PasswordHasher::new(CHEAP)
            .and_then(|hasher| hasher.hash(PASSWORD))
            .expect("hashes");
        let email = Email::parse(email).expect("a valid address");
        let now = self.time.clock().now();
        let user = User {
            id: Uuid::now_v7(),
            display_name: email.local_part().to_owned(),
            email,
            role,
            email_verified_at: Some(now),
            disabled_at: None,
            created_at: now,
            updated_at: now,
        };
        neon_store::users::insert(&self.pool, &user, &hash)
            .await
            .expect("the account is saved");
        user
    }

    /// Signs in with [`PASSWORD`] and returns the principal of the new session and its
    /// token.
    pub async fn sign_in(&self, email: &str) -> (Principal, Secret) {
        let outcome = self
            .auth
            .login(LoginInput {
                email: email.to_owned(),
                password: secret(PASSWORD),
                ip: IP.to_owned(),
                user_agent: "tests".to_owned(),
            })
            .await
            .expect("signs in");
        let principal = self
            .auth
            .authenticate_session(outcome.token.expose())
            .await
            .expect("the session works");
        (principal, outcome.token)
    }

    /// An admin's session.
    pub async fn admin(&self) -> Principal {
        self.account("admin@example.com", Role::Admin).await;
        self.sign_in("admin@example.com").await.0
    }

    pub async fn scalar<T>(&self, query: &'static str) -> T
    where
        T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
    {
        sqlx::query_scalar(query)
            .fetch_one(&self.pool)
            .await
            .expect("the query runs")
    }

    /// The token or code of the last mail of `kind` sent since the last call.
    pub fn token_of(&self, kind: &str) -> Secret {
        self.mailer
            .take()
            .into_iter()
            .rev()
            .filter(|mail| mail.message.kind() == kind)
            .find_map(|mail| match mail.message {
                Message::Verification { token, .. }
                | Message::PasswordReset { token, .. }
                | Message::JoinConfirmation { token, .. } => Some(token),
                Message::Invite { code, .. } => Some(code),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {kind} mail was sent"))
    }
}
