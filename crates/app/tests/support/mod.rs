//! What the auth, workspace and page tests share: the services on a fresh database with
//! fakes for their ports, a clock that moves by hand, cheap password hashes, the mail
//! queued, and the access changes told.

#![allow(dead_code, reason = "each test binary uses a part of it")]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neon_app::auth::{AuthService, LoginInput, ProfileUpdate};
use neon_app::clock::ManualTime;
use neon_app::crypto::{Argon2Params, Envelope, Keyring, PasswordHasher};
use neon_app::databases::DatabaseService;
use neon_app::jobs::Job;
use neon_app::mail::{KIND, Mail, MailQueue, Message};
use neon_app::pages::{AccessChange, AccessChanges, PageService};
use neon_app::ratelimit::{Bucket, Limited, RateLimiter};
use neon_app::settings::{Change, SettingsService, registry};
use neon_app::sync::{Broadcaster, LocalBroadcaster};
use neon_app::workspaces::{NewWorkspace, WorkspaceDetails, WorkspaceService};
use neon_domain::auth::{Email, Principal, Role, User};
use neon_domain::settings::{Pinned, Secret};
use neon_domain::workspaces::WorkspaceRole;
use serde_json::{Value as Json, json};
use sqlx::{PgPool, Row as _};
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

/// A change to one's own account that renames it alone.
pub fn renaming(name: &str) -> ProfileUpdate {
    ProfileUpdate {
        display_name: Some(name.to_owned()),
        ..ProfileUpdate::default()
    }
}

/// 2026-11-22 12:00 UTC.
pub fn start() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_795_348_800).expect("a valid time")
}

/// Where mail links point in the tests.
pub const PUBLIC_URL: &str = "https://neon.example.com";

/// The settings that set mail up, which every harness starts with.
pub fn mail_settings() -> Vec<(&'static str, Json)> {
    vec![
        ("mail.enabled", json!(true)),
        ("mail.host", json!("smtp.example.com")),
        ("mail.from", json!("neon@example.com")),
    ]
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

/// Keeps every access change told, in order.
#[derive(Default)]
pub struct ToldChanges(Mutex<Vec<AccessChange>>);

impl ToldChanges {
    /// The changes told since the last call, oldest first.
    pub fn take(&self) -> Vec<AccessChange> {
        std::mem::take(&mut *self.0.lock().expect("lock"))
    }
}

impl AccessChanges for ToldChanges {
    fn access_changed(&self, change: AccessChange) {
        self.0.lock().expect("lock").push(change);
    }
}

/// A service on a fresh database, with its fakes.
pub struct Harness {
    pub pool: PgPool,
    pub settings: Arc<SettingsService>,
    pub auth: AuthService,
    pub workspaces: WorkspaceService,
    pub pages: PageService,
    pub databases: DatabaseService,
    pub mail: MailQueue,
    pub limiter: Arc<ScriptedLimiter>,
    pub changes: Arc<ToldChanges>,
    /// Where the changes of databases go, for tests to join its rooms.
    pub broadcaster: Arc<LocalBroadcaster>,
    pub time: ManualTime,
}

pub struct Options {
    pub envelope: Envelope,
    pub params: Argon2Params,
    /// Saved after [`mail_settings`].
    pub settings: Vec<(&'static str, Json)>,
    /// Where mail links point. Without it, mail cannot go, though the settings set it up.
    pub public_url: Option<&'static str>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            envelope: envelope(),
            params: CHEAP,
            settings: Vec::new(),
            public_url: Some(PUBLIC_URL),
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
        for batch in [mail_settings(), options.settings] {
            settings
                .update(
                    batch
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), Change::Set(value))),
                    None,
                )
                .await
                .expect("valid settings");
        }
        let time = ManualTime::new(start());
        let limiter = Arc::new(ScriptedLimiter::default());
        let mail = MailQueue::new(
            Arc::clone(&settings),
            options.envelope.clone(),
            options.public_url,
            time.clock(),
        );
        let changes = Arc::new(ToldChanges::default());
        let workspaces = WorkspaceService::builder(pool.clone(), Arc::clone(&settings))
            .limiter(Arc::clone(&limiter) as Arc<dyn RateLimiter>)
            .envelope(options.envelope.clone())
            .access_changes(Arc::clone(&changes) as Arc<dyn AccessChanges>)
            .clock(time.clock())
            .build();
        let broadcaster = Arc::new(LocalBroadcaster::new());
        let pages = PageService::builder(pool.clone(), Arc::clone(&settings))
            .access_changes(Arc::clone(&changes) as Arc<dyn AccessChanges>)
            .broadcaster(Arc::clone(&broadcaster) as Arc<dyn Broadcaster>)
            .clock(time.clock())
            .build();
        let databases = DatabaseService::builder(pages.clone()).build();
        let auth = AuthService::builder(pool.clone(), Arc::clone(&settings))
            .password_params(options.params)
            .limiter(Arc::clone(&limiter) as Arc<dyn RateLimiter>)
            .mail(mail.clone())
            .envelope(options.envelope)
            .clock(time.clock())
            .access_changes(Arc::clone(&changes) as Arc<dyn AccessChanges>)
            .build()
            .await
            .expect("the service starts");
        Self {
            pool,
            settings,
            auth,
            workspaces,
            pages,
            databases,
            mail,
            limiter,
            changes,
            broadcaster,
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
            hue_override: None,
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

    /// A verified user's session.
    pub async fn person(&self, email: &str) -> Principal {
        self.account(email, Role::User).await;
        self.sign_in(email).await.0
    }

    /// A workspace `owner` creates, named `name`.
    pub async fn workspace(&self, owner: &Principal, name: &str) -> WorkspaceDetails {
        self.workspaces
            .create_workspace(
                owner,
                NewWorkspace {
                    name: name.to_owned(),
                    ..NewWorkspace::default()
                },
            )
            .await
            .expect("the workspace is created")
    }

    /// Brings `who` into a workspace with `role`: `by` invites them, and they accept.
    pub async fn bring_in(
        &self,
        by: &Principal,
        workspace_id: Uuid,
        who: &Principal,
        role: WorkspaceRole,
    ) {
        let sent = self
            .workspaces
            .invite(by, workspace_id, who.user.email.as_str(), role)
            .await
            .expect("invited");
        self.workspaces
            .accept_invitation(who, sent.invitation.id)
            .await
            .expect("accepted");
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

    /// The mail queued since the last call, oldest first, opened from the jobs, which are
    /// then deleted. Waits for the work done after answers first.
    pub async fn mails(&self) -> Vec<Mail> {
        self.auth.settle().await;
        let rows = sqlx::query(
            "DELETE FROM jobs WHERE kind = $1
             RETURNING id, kind, payload, attempts, max_attempts, traceparent, created_at",
        )
        .bind(KIND)
        .fetch_all(&self.pool)
        .await
        .expect("the queued mail");
        let mut jobs: Vec<Job> = rows
            .into_iter()
            .map(|row| Job {
                id: row.get("id"),
                kind: row.get("kind"),
                payload: row.get("payload"),
                attempt: row.get("attempts"),
                max_attempts: row.get("max_attempts"),
                traceparent: row.get("traceparent"),
                created_at: row.get("created_at"),
            })
            .collect();
        jobs.sort_by_key(|job| job.id);
        jobs.iter()
            .map(|job| self.mail.open(job).expect("queued mail opens").mail)
            .collect()
    }

    /// The token or code of the last mail of `kind` queued since the last call.
    pub async fn token_of(&self, kind: &str) -> Secret {
        self.mails()
            .await
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
