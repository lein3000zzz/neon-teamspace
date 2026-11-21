//! The background job queue table.
//!
//! [`enqueue`] runs on the caller's executor, so a job commits or rolls back with the
//! change that caused it. Workers [`claim`] due jobs with `FOR UPDATE SKIP LOCKED`, and
//! record each outcome with the [`Lease`] they got, so a worker that lost its claim
//! cannot overwrite what another worker did since.

use std::time::Duration;

use neon_observability::propagation;
use serde_json::Value as Json;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

/// The channel the database notifies when a job is queued. Its payload is the job's kind.
pub const CHANNEL: &str = "neon_jobs";

/// How many times a job runs before it is marked dead, unless it says otherwise.
pub const DEFAULT_MAX_ATTEMPTS: i32 = 10;

/// Every status a job can have, as the table's check constraint lists them.
pub const STATUSES: [&str; 4] = ["queued", "running", "done", "dead"];

/// A job to queue.
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    kind: String,
    payload: Json,
    max_attempts: i32,
    run_at: Option<OffsetDateTime>,
    traceparent: Option<String>,
}

impl NewJob {
    /// A job of `kind`, due now, run up to [`DEFAULT_MAX_ATTEMPTS`] times.
    pub fn new(kind: impl Into<String>, payload: Json) -> Self {
        Self {
            kind: kind.into(),
            payload,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            run_at: None,
            traceparent: None,
        }
    }

    /// At least 1; the database refuses less.
    pub fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Delays the first run.
    pub fn run_at(mut self, run_at: OffsetDateTime) -> Self {
        self.run_at = Some(run_at);
        self
    }

    /// The W3C `traceparent` of the trace the job's run joins. Without one, [`enqueue`]
    /// stores the current span's.
    pub fn traceparent(mut self, traceparent: Option<String>) -> Self {
        self.traceparent = traceparent;
        self
    }
}

/// Queues a job and returns its id. Workers listening on [`CHANNEL`] wake when the
/// caller's transaction commits.
///
/// A job given no `traceparent` gets the current span's, so its run continues the trace
/// of the request that queued it. Outside a span it has none and its run starts a trace.
pub async fn enqueue(executor: impl PgExecutor<'_>, job: &NewJob) -> sqlx::Result<Uuid> {
    let id = Uuid::now_v7();
    let traceparent = job
        .traceparent
        .clone()
        .or_else(propagation::current_traceparent);
    sqlx::query!(
        "INSERT INTO jobs (id, kind, payload, max_attempts, run_at, traceparent)
         VALUES ($1, $2, $3, $4, COALESCE($5, now()), $6)",
        id,
        job.kind,
        job.payload,
        job.max_attempts,
        job.run_at,
        traceparent,
    )
    .execute(executor)
    .await?;
    Ok(id)
}

/// How many jobs have each status, for the queue depth gauge. Statuses no job has are
/// left out. Done and dead jobs count until they are deleted.
pub async fn count_by_status(executor: impl PgExecutor<'_>) -> sqlx::Result<Vec<(String, i64)>> {
    let rows = sqlx::query!(r#"SELECT status, count(*) AS "count!" FROM jobs GROUP BY status"#)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.status, row.count))
        .collect())
}

/// A claimed job, ready to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: Uuid,
    pub kind: String,
    pub payload: Json,
    /// Which run this is, counting from 1.
    pub attempt: i32,
    pub max_attempts: i32,
    pub traceparent: Option<String>,
    pub created_at: OffsetDateTime,
}

impl Job {
    /// The proof of this claim, for recording its outcome.
    pub fn lease<'a>(&self, worker: &'a str) -> Lease<'a> {
        Lease {
            id: self.id,
            worker,
            attempt: self.attempt,
        }
    }
}

/// One claim of one job: the job, the worker that claimed it and the attempt it is on.
/// Every claim increments the attempt, so a stale lease matches nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lease<'a> {
    pub id: Uuid,
    pub worker: &'a str,
    pub attempt: i32,
}

/// Claims up to `limit` due jobs of the given kinds for `worker`, oldest first. Rows
/// another transaction has locked are skipped rather than waited for.
pub async fn claim(
    executor: impl PgExecutor<'_>,
    worker: &str,
    kinds: &[String],
    limit: i64,
) -> sqlx::Result<Vec<Job>> {
    sqlx::query_as!(
        Job,
        r#"WITH due AS (
               SELECT id FROM jobs
               WHERE status = 'queued' AND run_at <= now() AND kind = ANY($1)
               ORDER BY run_at, id
               LIMIT $2
               FOR UPDATE SKIP LOCKED
           )
           UPDATE jobs
           SET status = 'running', attempts = jobs.attempts + 1,
               locked_at = now(), locked_by = $3, updated_at = now()
           FROM due
           WHERE jobs.id = due.id
           RETURNING jobs.id, jobs.kind, jobs.payload, jobs.attempts AS attempt,
                     jobs.max_attempts, jobs.traceparent, jobs.created_at"#,
        kinds,
        limit,
        worker,
    )
    .fetch_all(executor)
    .await
}

/// Marks a job done. False when the lease no longer holds.
pub async fn complete(executor: impl PgExecutor<'_>, lease: Lease<'_>) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE jobs
         SET status = 'done', locked_at = NULL, locked_by = NULL, updated_at = now()
         WHERE id = $1 AND status = 'running' AND locked_by = $2 AND attempts = $3",
        lease.id,
        lease.worker,
        lease.attempt,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Queues a failed job again after `delay`. False when the lease no longer holds.
pub async fn retry(
    executor: impl PgExecutor<'_>,
    lease: Lease<'_>,
    delay: Duration,
    error: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE jobs
         SET status = 'queued', run_at = now() + make_interval(secs => $4),
             last_error = $5, locked_at = NULL, locked_by = NULL, updated_at = now()
         WHERE id = $1 AND status = 'running' AND locked_by = $2 AND attempts = $3",
        lease.id,
        lease.worker,
        lease.attempt,
        delay.as_secs_f64(),
        error,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Gives up on a job, keeping its last error. False when the lease no longer holds.
pub async fn mark_dead(
    executor: impl PgExecutor<'_>,
    lease: Lease<'_>,
    error: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE jobs
         SET status = 'dead', last_error = $4, locked_at = NULL, locked_by = NULL,
             updated_at = now()
         WHERE id = $1 AND status = 'running' AND locked_by = $2 AND attempts = $3",
        lease.id,
        lease.worker,
        lease.attempt,
        error,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Queues jobs a stopping worker could not finish, to run again at once. The interrupted
/// attempt is not counted: the job did not fail, the worker went away.
pub async fn release(
    executor: impl PgExecutor<'_>,
    worker: &str,
    ids: &[Uuid],
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE jobs
         SET status = 'queued', attempts = attempts - 1, run_at = now(),
             locked_at = NULL, locked_by = NULL, updated_at = now()
         WHERE id = ANY($1) AND status = 'running' AND locked_by = $2",
        ids,
        worker,
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Takes back jobs whose worker has held them longer than `lease_timeout`, which means
/// it crashed or lost the database. They are queued again, or marked dead when that was
/// their last attempt.
pub async fn reclaim_expired(
    executor: impl PgExecutor<'_>,
    lease_timeout: Duration,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "UPDATE jobs
         SET status = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'queued' END,
             last_error = 'lease expired: worker ' || locked_by || ' did not finish',
             run_at = now(), locked_at = NULL, locked_by = NULL, updated_at = now()
         WHERE status = 'running' AND locked_at < now() - make_interval(secs => $1)",
        lease_timeout.as_secs_f64(),
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes done and dead jobs that finished more than `retention` ago.
pub async fn delete_finished(
    executor: impl PgExecutor<'_>,
    retention: Duration,
) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM jobs
         WHERE status IN ('done', 'dead') AND updated_at < now() - make_interval(secs => $1)",
        retention.as_secs_f64(),
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
