-- Incoming webhooks. Each acts as an integration of its workspace, owned by the person who
-- made it, on one target: a database, whose rows it makes and changes, or a page, which it
-- adds to. It is reached at a secret URL, of which the server keeps the SHA-256, and may
-- also check an HMAC signature with a secret it keeps sealed. Its name is its
-- integration's.
CREATE TABLE incoming_webhooks (
    id                      uuid PRIMARY KEY, -- UUIDv7
    workspace_id            uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    integration_id          uuid        NOT NULL UNIQUE REFERENCES users (id) ON DELETE CASCADE,
    -- A page of the workspace. Not a foreign key: once the page is gone the hook stays,
    -- refusing what is sent, until someone deletes it and so its integration.
    target_id               uuid        NOT NULL,
    target_kind             text        NOT NULL CHECK (target_kind IN ('database', 'page')),
    token_hash              bytea       NOT NULL UNIQUE,
    -- Sealed; unset while it takes requests without a signature.
    signing_secret          bytea,
    -- The secret before the last rotation, which still verifies until it expires.
    previous_signing_secret bytea,
    previous_expires_at     timestamptz,
    disabled_at             timestamptz,
    created_by              uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at              timestamptz NOT NULL,
    updated_at              timestamptz NOT NULL,
    CHECK ((previous_signing_secret IS NULL) = (previous_expires_at IS NULL))
);

-- A workspace's incoming webhooks, as its settings list them.
CREATE INDEX incoming_webhooks_workspace ON incoming_webhooks (workspace_id, created_at, id);

-- The last requests each incoming webhook took, for its log: when, from where, what it
-- asked for and how it was answered, never what it sent. The server keeps the last 50 of
-- each, and none of the requests it refused for their rate.
CREATE TABLE incoming_webhook_requests (
    id          uuid PRIMARY KEY, -- UUIDv7
    hook_id     uuid        NOT NULL REFERENCES incoming_webhooks (id) ON DELETE CASCADE,
    at          timestamptz NOT NULL,
    status_code integer     NOT NULL,
    -- ok, or the error code it was answered with.
    outcome     text        NOT NULL,
    -- create_row, update_row or append, once its payload read.
    action      text,
    client_ip   text        NOT NULL,
    latency_ms  integer     NOT NULL CHECK (latency_ms >= 0)
);

CREATE INDEX incoming_webhook_requests_log
    ON incoming_webhook_requests (hook_id, at DESC, id DESC);
