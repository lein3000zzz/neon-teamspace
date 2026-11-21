use std::time::Duration;

use neon_store::jobs::{self, CHANNEL, NewJob};
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use uuid::Uuid;

const WORKER: &str = "worker-a";

fn kinds() -> Vec<String> {
    vec!["mail.send".to_owned()]
}

fn mail() -> NewJob {
    NewJob::new("mail.send", json!({"to": "ada@example.com"}))
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct Row {
    status: String,
    attempts: i32,
    last_error: Option<String>,
    locked_by: Option<String>,
}

async fn row(pool: &PgPool, id: Uuid) -> Row {
    sqlx::query_as("SELECT status, attempts, last_error, locked_by FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("job exists")
}

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(pool)
        .await
        .expect("count")
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn enqueue_commits_and_rolls_back_with_the_caller(pool: PgPool) {
    let mut tx = pool.begin().await.unwrap();
    jobs::enqueue(&mut *tx, &mail()).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(count(&pool).await, 0);

    let mut tx = pool.begin().await.unwrap();
    let id = jobs::enqueue(&mut *tx, &mail().traceparent(Some("00-abc-def-01".into())))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let traceparent: Option<String> =
        sqlx::query_scalar("SELECT traceparent FROM jobs WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(traceparent.as_deref(), Some("00-abc-def-01"));
    assert_eq!(id.get_version_num(), 7);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn claim_takes_due_jobs_of_the_given_kinds(pool: PgPool) {
    let due = jobs::enqueue(&pool, &mail()).await.unwrap();
    let later = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
    jobs::enqueue(&pool, &mail().run_at(later)).await.unwrap();
    jobs::enqueue(&pool, &NewJob::new("export.run", json!({})))
        .await
        .unwrap();

    let claimed = jobs::claim(&pool, WORKER, &kinds(), 10).await.unwrap();

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, due);
    assert_eq!(claimed[0].attempt, 1);
    assert_eq!(claimed[0].payload, json!({"to": "ada@example.com"}));
    assert_eq!(
        row(&pool, due).await,
        Row {
            status: "running".into(),
            attempts: 1,
            last_error: None,
            locked_by: Some(WORKER.into()),
        }
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn concurrent_claims_skip_locked_rows(pool: PgPool) {
    for _ in 0..5 {
        jobs::enqueue(&pool, &mail()).await.unwrap();
    }

    // The first claim keeps its rows locked until its transaction ends.
    let mut tx = pool.begin().await.unwrap();
    let first = jobs::claim(&mut *tx, "worker-a", &kinds(), 3)
        .await
        .unwrap();
    let second = jobs::claim(&pool, "worker-b", &kinds(), 10).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 2);
    assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn complete_needs_the_current_lease(pool: PgPool) {
    let id = jobs::enqueue(&pool, &mail()).await.unwrap();
    let job = jobs::claim(&pool, WORKER, &kinds(), 1)
        .await
        .unwrap()
        .remove(0);

    assert!(!jobs::complete(&pool, job.lease("worker-b")).await.unwrap());
    assert!(jobs::complete(&pool, job.lease(WORKER)).await.unwrap());
    assert!(!jobs::complete(&pool, job.lease(WORKER)).await.unwrap());
    assert_eq!(row(&pool, id).await.status, "done");
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn retry_queues_the_job_after_the_delay(pool: PgPool) {
    let id = jobs::enqueue(&pool, &mail()).await.unwrap();
    let job = jobs::claim(&pool, WORKER, &kinds(), 1)
        .await
        .unwrap()
        .remove(0);

    let retried = jobs::retry(
        &pool,
        job.lease(WORKER),
        Duration::from_secs(3600),
        "smtp down",
    )
    .await
    .unwrap();

    assert!(retried);
    assert_eq!(
        row(&pool, id).await,
        Row {
            status: "queued".into(),
            attempts: 1,
            last_error: Some("smtp down".into()),
            locked_by: None,
        }
    );
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM run_at - now())::float8 FROM jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((3590.0..=3600.0).contains(&wait), "run_at is {wait}s away");
    assert!(
        jobs::claim(&pool, WORKER, &kinds(), 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn mark_dead_keeps_the_last_error(pool: PgPool) {
    let id = jobs::enqueue(&pool, &mail()).await.unwrap();
    let job = jobs::claim(&pool, WORKER, &kinds(), 1)
        .await
        .unwrap()
        .remove(0);

    assert!(
        jobs::mark_dead(&pool, job.lease(WORKER), "bad address")
            .await
            .unwrap()
    );
    assert_eq!(
        row(&pool, id).await,
        Row {
            status: "dead".into(),
            attempts: 1,
            last_error: Some("bad address".into()),
            locked_by: None,
        }
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn reclaim_takes_back_expired_leases_only(pool: PgPool) {
    let stale = jobs::enqueue(&pool, &mail()).await.unwrap();
    let last_try = jobs::enqueue(&pool, &mail().max_attempts(1)).await.unwrap();
    let fresh = jobs::enqueue(&pool, &mail()).await.unwrap();
    let claimed = jobs::claim(&pool, "gone", &kinds(), 3).await.unwrap();
    assert_eq!(claimed.len(), 3);
    sqlx::query("UPDATE jobs SET locked_at = now() - interval '1 hour' WHERE id = ANY($1)")
        .bind([stale, last_try])
        .execute(&pool)
        .await
        .unwrap();

    let reclaimed = jobs::reclaim_expired(&pool, Duration::from_secs(600))
        .await
        .unwrap();

    assert_eq!(reclaimed, 2);
    let expired = Some("lease expired: worker gone did not finish".to_owned());
    assert_eq!(
        row(&pool, stale).await,
        Row {
            status: "queued".into(),
            attempts: 1,
            last_error: expired.clone(),
            locked_by: None,
        }
    );
    assert_eq!(row(&pool, last_try).await.status, "dead");
    assert_eq!(row(&pool, last_try).await.last_error, expired);
    assert_eq!(row(&pool, fresh).await.status, "running");

    // The stale worker's lease no longer holds, even after the job is claimed again.
    let old = claimed.iter().find(|job| job.id == stale).unwrap();
    let again = jobs::claim(&pool, "gone", &kinds(), 1).await.unwrap();
    assert_eq!(again[0].id, stale);
    assert!(!jobs::complete(&pool, old.lease("gone")).await.unwrap());
    assert!(jobs::complete(&pool, again[0].lease("gone")).await.unwrap());
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn release_does_not_count_the_attempt(pool: PgPool) {
    let id = jobs::enqueue(&pool, &mail()).await.unwrap();
    jobs::claim(&pool, WORKER, &kinds(), 1).await.unwrap();

    assert_eq!(jobs::release(&pool, "worker-b", &[id]).await.unwrap(), 0);
    assert_eq!(jobs::release(&pool, WORKER, &[id]).await.unwrap(), 1);
    assert_eq!(
        row(&pool, id).await,
        Row {
            status: "queued".into(),
            attempts: 0,
            last_error: None,
            locked_by: None,
        }
    );
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn delete_finished_keeps_recent_and_unfinished_jobs(pool: PgPool) {
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(jobs::enqueue(&pool, &mail()).await.unwrap());
    }
    let [old_done, old_dead, recent_done, old_queued] = ids[..] else {
        unreachable!()
    };
    sqlx::query(
        "UPDATE jobs SET status = CASE WHEN id = $2 THEN 'dead' ELSE 'done' END
         WHERE id = ANY($1)",
    )
    .bind([old_done, old_dead, recent_done])
    .bind(old_dead)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE jobs SET updated_at = now() - interval '8 days' WHERE id <> $1")
        .bind(recent_done)
        .execute(&pool)
        .await
        .unwrap();

    let deleted = jobs::delete_finished(&pool, Duration::from_secs(7 * 24 * 3600))
        .await
        .unwrap();

    assert_eq!(deleted, 2);
    let left: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM jobs ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, [recent_done, old_queued]);
}

#[sqlx::test(migrator = "neon_store::MIGRATOR")]
async fn enqueue_notifies_listeners_on_commit_only(pool: PgPool) {
    let mut listener = PgListener::connect_with(&pool).await.unwrap();
    listener.listen(CHANNEL).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    jobs::enqueue(&mut *tx, &NewJob::new("export.run", json!({})))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    jobs::enqueue(&mut *tx, &mail()).await.unwrap();
    tx.commit().await.unwrap();

    let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv())
        .await
        .expect("a notification")
        .unwrap();
    assert_eq!(notification.payload(), "mail.send");
}
