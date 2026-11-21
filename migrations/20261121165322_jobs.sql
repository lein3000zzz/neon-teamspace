-- Background jobs. A job is inserted in the same transaction as the change that caused
-- it, so a rollback leaves no job behind. Workers claim queued jobs with
-- FOR UPDATE SKIP LOCKED, so two workers never take the same one.
--
-- queued  waits for run_at; a failed attempt goes back here with a later run_at
-- running claimed by locked_by at locked_at
-- done    finished
-- dead    failed max_attempts times, or failed in a way retrying cannot fix
CREATE TABLE jobs (
    id           uuid PRIMARY KEY, -- UUIDv7
    kind         text        NOT NULL,
    payload      jsonb       NOT NULL,
    status       text        NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'done', 'dead')),
    attempts     integer     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts integer     NOT NULL CHECK (max_attempts >= 1),
    run_at       timestamptz NOT NULL DEFAULT now(),
    locked_at    timestamptz,
    locked_by    text,
    last_error   text,
    -- The W3C trace context of the request that queued the job, so its run joins the trace.
    traceparent  text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    -- Only a running job holds a lock.
    CHECK ((status = 'running') = (locked_at IS NOT NULL AND locked_by IS NOT NULL))
);

-- The claim query: queued jobs that are due, oldest first.
CREATE INDEX jobs_due ON jobs (run_at) WHERE status = 'queued';
-- Reclaiming running jobs whose worker stopped responding.
CREATE INDEX jobs_running ON jobs (locked_at) WHERE status = 'running';
-- Deleting finished jobs past their retention.
CREATE INDEX jobs_finished ON jobs (updated_at) WHERE status IN ('done', 'dead');

-- Wake listening workers as soon as a job is queued. A notification sent inside a
-- transaction is delivered on commit, and never if it rolls back. Postgres folds
-- duplicate notifications within a transaction, so a batch of inserts wakes workers once
-- per kind.
CREATE FUNCTION jobs_notify() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    PERFORM pg_notify('neon_jobs', NEW.kind);
    RETURN NULL;
END;
$$;

CREATE TRIGGER jobs_notify
    AFTER INSERT ON jobs
    FOR EACH ROW
EXECUTE FUNCTION jobs_notify();
