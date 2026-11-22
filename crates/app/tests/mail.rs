//! Mail through the job queue: queued with the change it tells of, sealed while it waits,
//! written and sent by the mail job, retried or dropped as the mail server answers, and
//! never kept once done.

mod support;

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use neon_app::auth::AuthError;
use neon_app::crypto::Envelope;
use neon_app::jobs::{Handler as _, Job, Worker, WorkerConfig};
use neon_app::mail::{
    KIND, Mail, MailJob, MailQueue, Mailer, Message, Outgoing, QueueError, Recipient, SendError,
};
use neon_domain::auth::Role;
use neon_domain::jobs::Backoff;
use neon_domain::settings::Secret;
use serde_json::{Value as Json, json};
use sqlx::PgPool;
use support::{Harness, Options, PUBLIC_URL};

/// Records what it was handed, and answers as told.
#[derive(Default)]
struct FakeMailer {
    answer: Mutex<Option<SendError>>,
    sent: Mutex<Vec<Outgoing>>,
    tries: Mutex<usize>,
}

impl FakeMailer {
    fn answering(error: SendError) -> Arc<Self> {
        let mailer = Self::default();
        *mailer.answer.lock().expect("lock") = Some(error);
        Arc::new(mailer)
    }

    fn sent(&self) -> Vec<Outgoing> {
        self.sent.lock().expect("lock").clone()
    }

    fn tries(&self) -> usize {
        *self.tries.lock().expect("lock")
    }
}

#[async_trait]
impl Mailer for FakeMailer {
    async fn send(&self, mail: &Outgoing) -> Result<(), SendError> {
        *self.tries.lock().expect("lock") += 1;
        if let Some(error) = self.answer.lock().expect("lock").clone() {
            return Err(error);
        }
        self.sent.lock().expect("lock").push(mail.clone());
        Ok(())
    }
}

fn reset(token: &str) -> Mail {
    Mail {
        to: Recipient::new("ada@example.com", Some("Ada".into())),
        message: Message::PasswordReset {
            token: Secret::new(token),
            lasts: Duration::from_secs(3600),
        },
    }
}

async fn queue(h: &Harness, mail: &Mail) -> uuid::Uuid {
    let mut conn = h.pool.acquire().await.expect("a connection");
    h.mail.queue(&mut conn, mail).await.expect("queued")
}

async fn jobs(pool: &PgPool) -> Vec<(uuid::Uuid, Json, i32)> {
    sqlx::query_as("SELECT id, payload, attempts FROM jobs WHERE kind = $1 ORDER BY id")
        .bind(KIND)
        .fetch_all(pool)
        .await
        .expect("the jobs")
}

async fn claim(pool: &PgPool) -> Job {
    neon_store::jobs::claim(pool, "tests", &[KIND.to_owned()], 1)
        .await
        .expect("claims")
        .pop()
        .expect("a job")
}

fn worker_config() -> WorkerConfig {
    WorkerConfig {
        poll_interval: Duration::from_millis(50),
        run_timeout: Duration::from_secs(5),
        lease_timeout: Duration::from_secs(10),
        maintenance_interval: Duration::from_millis(100),
        backoff: Backoff {
            base: Duration::from_millis(20),
            max: Duration::from_millis(50),
        },
        stop_timeout: Duration::from_secs(5),
        ..WorkerConfig::default()
    }
}

/// Waits up to ten seconds for `check` to hold.
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mail_commits_and_rolls_back_with_the_change_it_tells_of(pool: PgPool) {
    let h = Harness::new(pool).await;

    let mut tx = h.pool.begin().await.expect("a transaction");
    h.mail
        .queue(&mut tx, &reset("rolled-back"))
        .await
        .expect("queued");
    tx.rollback().await.expect("rolls back");
    assert!(jobs(&h.pool).await.is_empty());

    let mut tx = h.pool.begin().await.expect("a transaction");
    h.mail
        .queue(&mut tx, &reset("committed"))
        .await
        .expect("queued");
    tx.commit().await.expect("commits");
    assert_eq!(jobs(&h.pool).await.len(), 1);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn queued_mail_is_sealed_and_bound_to_its_job(pool: PgPool) {
    let h = Harness::new(pool).await;
    let id = queue(&h, &reset("hunter2-reset-token")).await;
    let other = queue(&h, &reset("another-token")).await;

    let stored = jobs(&h.pool).await;
    let text = stored[0].1.to_string();
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("ada@example.com"), "{text}");
    assert!(stored[0].1["sealed"].is_string());

    // Moved onto another job's row, it does not open.
    sqlx::query("UPDATE jobs SET payload = $2 WHERE id = $1")
        .bind(other)
        .bind(&stored[0].1)
        .execute(&h.pool)
        .await
        .expect("moved");
    let mut opened = Vec::new();
    while let Some(job) = neon_store::jobs::claim(&h.pool, "tests", &[KIND.to_owned()], 1)
        .await
        .expect("claims")
        .pop()
    {
        opened.push((job.id, h.mail.open(&job).map(|queued| queued.mail)));
    }
    assert_eq!(opened[0].0, id);
    assert_eq!(opened[0].1, Ok(reset("hunter2-reset-token")));
    assert_eq!(opened[1].0, other);
    let refused = opened[1].1.clone().expect_err("bound to its own job");
    assert!(
        refused.starts_with("the payload does not open"),
        "{refused}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn every_kind_of_mail_survives_the_queue(pool: PgPool) {
    let h = Harness::new(pool).await;
    let at = h.time.clock().now();
    let messages = [
        Message::Verification {
            token: Secret::new("t1"),
            lasts: Duration::from_secs(86_400),
        },
        Message::PasswordReset {
            token: Secret::new("t2"),
            lasts: Duration::from_secs(3600),
        },
        Message::PasswordChanged { at },
        Message::Invite {
            inviter_name: "Grace".into(),
            inviter_email: "grace@example.com".into(),
            code: Secret::new("inv_x"),
            expires_at: at,
        },
        Message::JoinConfirmation {
            token: Secret::new("t3"),
            lasts: Duration::from_secs(86_400),
        },
        Message::JoinDenied {
            ask_again_after: Duration::from_secs(30 * 86_400),
        },
        Message::JoinRequested {
            requester: "linus@example.com".into(),
            note: "hello".into(),
        },
        Message::Test {
            sent_by: "grace@example.com".into(),
        },
    ];
    let sent: Vec<Mail> = messages
        .into_iter()
        .map(|message| Mail {
            to: Recipient::new("ada@example.com", None),
            message,
        })
        .collect();
    for mail in &sent {
        queue(&h, mail).await;
    }

    // Times keep whole seconds in the queue, which is what mail shows.
    let whole = |mail: Mail| match mail.message {
        Message::PasswordChanged { at } => Mail {
            message: Message::PasswordChanged {
                at: at.replace_nanosecond(0).expect("valid"),
            },
            ..mail
        },
        Message::Invite {
            inviter_name,
            inviter_email,
            code,
            expires_at,
        } => Mail {
            message: Message::Invite {
                inviter_name,
                inviter_email,
                code,
                expires_at: expires_at.replace_nanosecond(0).expect("valid"),
            },
            ..mail
        },
        _ => mail,
    };
    let received: Vec<Mail> = h.mails().await;
    assert_eq!(received, sent.into_iter().map(whole).collect::<Vec<_>>());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_job_writes_the_mail_and_sends_it(pool: PgPool) {
    let h = Harness::new(pool).await;
    h.set("instance.name", json!("Hackathon")).await;
    let mailer = Arc::new(FakeMailer::default());
    let job = MailJob::new(h.mail.clone(), Arc::clone(&mailer) as Arc<dyn Mailer>);
    queue(&h, &reset("the-token")).await;

    job.run(&claim(&h.pool).await).await.expect("sent");

    let sent = mailer.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].to,
        Recipient::new("ada@example.com", Some("Ada".into()))
    );
    assert_eq!(sent[0].subject, "Set a new Hackathon password");
    let link = format!("{PUBLIC_URL}/reset?token=the-token");
    assert!(sent[0].text.contains(&link), "{}", sent[0].text);
    assert!(sent[0].html.contains(&format!("href=\"{link}\"")));
    assert!(h.mail.status().deliveries.last_sent_at.is_some());
    assert_eq!(job.kind(), KIND);
    assert!(!job.keeps_finished());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn sent_mail_leaves_no_job_behind(pool: PgPool) {
    let h = Harness::new(pool).await;
    let mailer = Arc::new(FakeMailer::default());
    queue(&h, &reset("the-token")).await;

    let worker = Worker::new(h.pool.clone(), worker_config())
        .handler(MailJob::new(
            h.mail.clone(),
            Arc::clone(&mailer) as Arc<dyn Mailer>,
        ))
        .start()
        .await
        .expect("starts");
    eventually("the mail to go and its job to go with it", || async {
        mailer.sent().len() == 1 && jobs(&h.pool).await.is_empty()
    })
    .await;
    worker.stop().await;
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_refused_address_ends_the_job_at_once(pool: PgPool) {
    let h = Harness::new(pool).await;
    let mailer = FakeMailer::answering(SendError::Rejected("550 5.1.1 No such user here".into()));
    queue(&h, &reset("the-token")).await;

    let worker = Worker::new(h.pool.clone(), worker_config())
        .handler(MailJob::new(
            h.mail.clone(),
            Arc::clone(&mailer) as Arc<dyn Mailer>,
        ))
        .start()
        .await
        .expect("starts");
    eventually("the refused mail's job to go", || async {
        mailer.tries() == 1 && jobs(&h.pool).await.is_empty()
    })
    .await;
    worker.stop().await;

    assert_eq!(mailer.tries(), 1, "never tried again");
    let failure = h.mail.status().deliveries.last_failure.expect("recorded");
    assert_eq!(failure.kind, "password_reset");
    assert!(failure.permanent);
    assert!(
        failure.error.contains("550 5.1.1 No such user here"),
        "{failure:?}"
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn other_failures_are_retried_sealed_then_dropped(pool: PgPool) {
    let h = Harness::new(pool).await;
    let mailer = FakeMailer::answering(SendError::Failed("connection refused".into()));
    queue(&h, &reset("hunter2-reset-token")).await;
    let job = MailJob::new(h.mail.clone(), Arc::clone(&mailer) as Arc<dyn Mailer>);

    let claimed = claim(&h.pool).await;
    let error = job.run(&claimed).await.expect_err("fails");
    assert!(!error.is_permanent());
    assert_eq!(error.message(), "connection refused");
    neon_store::jobs::release(&h.pool, "tests", &[claimed.id])
        .await
        .expect("queued again");

    let worker = Worker::new(h.pool.clone(), worker_config())
        .handler(job)
        .start()
        .await
        .expect("starts");
    eventually("a retry to be queued", || async {
        jobs(&h.pool)
            .await
            .first()
            .is_some_and(|(_, payload, attempts)| {
                *attempts >= 2 && !payload.to_string().contains("hunter2")
            })
    })
    .await;
    eventually("the job to die and go", || async {
        jobs(&h.pool).await.is_empty()
    })
    .await;
    worker.stop().await;

    assert!(mailer.tries() > 2, "tried until its attempts ran out");
    let failure = h.mail.status().deliveries.last_failure.expect("recorded");
    assert!(!failure.permanent);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mail_whose_link_expired_while_it_waited_is_not_sent(pool: PgPool) {
    let h = Harness::new(pool).await;
    let mailer = Arc::new(FakeMailer::default());
    let job = MailJob::new(h.mail.clone(), Arc::clone(&mailer) as Arc<dyn Mailer>);
    queue(&h, &reset("the-token")).await;
    let claimed = claim(&h.pool).await;
    h.time.advance(Duration::from_secs(3600));

    let error = job.run(&claimed).await.expect_err("expired");

    assert!(error.is_permanent());
    assert!(error.message().contains("expired"), "{}", error.message());
    assert_eq!(mailer.tries(), 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn a_payload_that_does_not_open_ends_the_job(pool: PgPool) {
    let h = Harness::new(pool).await;
    let mailer = Arc::new(FakeMailer::default());
    let job = MailJob::new(h.mail.clone(), Arc::clone(&mailer) as Arc<dyn Mailer>);
    neon_store::jobs::enqueue(
        &h.pool,
        &neon_store::jobs::NewJob::new(KIND, json!({"to": "ada@example.com"})),
    )
    .await
    .expect("queued");

    let error = job.run(&claim(&h.pool).await).await.expect_err("bad");

    assert!(error.is_permanent());
    assert_eq!(mailer.tries(), 0);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn the_status_says_what_mail_is_missing(pool: PgPool) {
    let h = Harness::new(pool).await;
    assert!(h.mail.configured());
    assert_eq!(h.mail.status().problem, None);

    let without = |envelope: Envelope, public_url: Option<&str>| {
        MailQueue::new(
            Arc::clone(&h.settings),
            envelope,
            public_url,
            h.time.clock(),
        )
        .status()
        .problem
        .expect("a problem")
    };
    assert!(without(Envelope::default(), Some(PUBLIC_URL)).starts_with("secrets.keys is not set"));
    assert!(without(support::envelope(), None).starts_with("server.public_url is not set"));

    h.set("mail.from", json!("")).await;
    assert_eq!(
        h.mail.status().problem.as_deref(),
        Some("mail.from is not set")
    );
    h.set("mail.host", json!("")).await;
    assert_eq!(
        h.mail.status().problem.as_deref(),
        Some("mail.host is not set")
    );
    h.set("mail.enabled", json!(false)).await;
    assert_eq!(
        h.mail.status().problem.as_deref(),
        Some("mail.enabled is off")
    );
    assert!(!h.mail.configured());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn without_a_secret_key_nothing_is_queued(pool: PgPool) {
    let h = Harness::new(pool).await;
    let unsealed = MailQueue::new(
        Arc::clone(&h.settings),
        Envelope::default(),
        Some(PUBLIC_URL),
        h.time.clock(),
    );
    let mut conn = h.pool.acquire().await.expect("a connection");

    let error = unsealed
        .queue(&mut conn, &reset("the-token"))
        .await
        .expect_err("no key");

    assert!(matches!(error, QueueError::Seal(_)), "{error:?}");
    assert!(jobs(&h.pool).await.is_empty());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn admins_send_themselves_a_test_mail(pool: PgPool) {
    let h = Harness::new(pool).await;
    let admin = h.admin().await;
    h.account("ada@example.com", Role::User).await;
    let (ada, _) = h.sign_in("ada@example.com").await;

    let to = h.auth.send_test_mail(&admin).await.expect("queued");

    assert_eq!(to, "admin@example.com");
    let mails = h.mails().await;
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to.address, "admin@example.com");
    assert_eq!(
        mails[0].message,
        Message::Test {
            sent_by: "admin@example.com".into()
        }
    );
    assert!(matches!(
        h.auth.send_test_mail(&ada).await,
        Err(AuthError::Forbidden(_))
    ));
    h.set("mail.enabled", json!(false)).await;
    let off = h.auth.send_test_mail(&admin).await.expect_err("off");
    assert_eq!(off.to_string(), "mail is not set up: mail.enabled is off");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn confirmed_requests_notify_admins_only_while_notifications_are_on(pool: PgPool) {
    let h = Harness::with(
        pool,
        Options {
            settings: vec![
                ("auth.requests", json!(true)),
                ("mail.notifications", json!(false)),
            ],
            ..Options::default()
        },
    )
    .await;
    h.admin().await;
    h.auth
        .request_to_join("ada@example.com", "", support::IP)
        .await
        .expect("taken");
    let token = h.token_of("join_confirmation").await;

    h.auth
        .confirm_join(&token, support::IP)
        .await
        .expect("confirms");

    assert!(h.mails().await.is_empty());
}
