-- Outgoing webhooks. Each belongs to one identity of a workspace, a person, an agent or an
-- integration, and is told of the events it names about the rows that identity is assigned
-- to, signed with a secret of its own. It goes with the identity's membership: once they
-- are out of the workspace, it is gone. Its URL is also bound to whoever set it last: it is
-- told only of rows they may see too, and only while they may still register it.
CREATE TABLE webhooks (
    id                  uuid PRIMARY KEY, -- UUIDv7
    workspace_id        uuid        NOT NULL,
    -- The identity whose events it delivers.
    user_id             uuid        NOT NULL,
    url                 text        NOT NULL CHECK (octet_length(url) <= 2048),
    -- Event types such as row.assigned. Left open, as more come later; the server checks
    -- the ones it is given.
    events              text[]      NOT NULL CHECK (cardinality(events) > 0),
    -- Whether it is told of the identity's own changes too.
    include_own         boolean     NOT NULL DEFAULT false,
    -- The signing secret, sealed with the instance's key and bound to the webhook.
    secret              bytea       NOT NULL,
    -- The secret before the last rotation, which deliveries are signed with too, beside the
    -- new one, until it expires.
    previous_secret     bytea,
    previous_expires_at timestamptz,
    -- Set while it is off: turned off by hand, or after too many deliveries failed in a row.
    disabled_at         timestamptz,
    disabled_reason     text,
    -- Deliveries given up on since the last one that went through.
    failures            integer     NOT NULL DEFAULT 0 CHECK (failures >= 0),
    created_by          uuid REFERENCES users (id) ON DELETE SET NULL,
    -- Who set its URL last, when it was made or since. Once their account is gone, it is
    -- turned off at its next delivery.
    url_set_by          uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at          timestamptz NOT NULL,
    updated_at          timestamptz NOT NULL,
    FOREIGN KEY (workspace_id, user_id)
        REFERENCES workspace_members (workspace_id, user_id) ON DELETE CASCADE,
    CHECK ((previous_secret IS NULL) = (previous_expires_at IS NULL)),
    CHECK ((disabled_at IS NULL) = (disabled_reason IS NULL))
);

-- The webhooks a change to a row is told to, by whom it concerns.
CREATE INDEX webhooks_subscribers ON webhooks (workspace_id, user_id) WHERE disabled_at IS NULL;
-- The webhooks whose URL someone set, to check again when their place in the workspace
-- changes.
CREATE INDEX webhooks_url_set_by ON webhooks (workspace_id, url_set_by) WHERE disabled_at IS NULL;
-- A workspace's webhooks, as its settings list them.
CREATE INDEX webhooks_workspace ON webhooks (workspace_id, created_at, id);

-- The last attempts at each webhook's deliveries, for its log: how each went, never what
-- was sent, and of the answer only its status and a short excerpt. The server keeps the
-- last 50 of each webhook.
CREATE TABLE webhook_attempts (
    id          uuid PRIMARY KEY, -- UUIDv7
    webhook_id  uuid        NOT NULL REFERENCES webhooks (id) ON DELETE CASCADE,
    -- The delivery, the job that tries it; a test event's has none.
    delivery_id uuid,
    event_id    uuid        NOT NULL,
    event_type  text        NOT NULL,
    attempt     integer     NOT NULL CHECK (attempt >= 1),
    -- delivered: the receiver answered 2xx. retrying: it failed, and is tried again later.
    -- failed: it failed for the last time. skipped: it was not sent, as the identity can no
    -- longer see the row, or the webhook is off.
    outcome     text        NOT NULL
        CHECK (outcome IN ('delivered', 'retrying', 'failed', 'skipped')),
    status_code integer,
    latency_ms  integer CHECK (latency_ms >= 0),
    error_kind  text,
    excerpt     text CHECK (octet_length(excerpt) <= 256),
    at          timestamptz NOT NULL
);

CREATE INDEX webhook_attempts_log ON webhook_attempts (webhook_id, at DESC, id DESC);
