-- Accounts and how they sign in. Every secret that proves who someone is, session and API
-- token alike, and every mailed link, is kept only as the SHA-256 hash of its token; the
-- password as an argon2id PHC string.
CREATE TABLE users (
    id                uuid PRIMARY KEY, -- UUIDv7
    -- The address as typed, and the lowercase form it is found by.
    email             text        NOT NULL,
    email_normalized  text        NOT NULL UNIQUE,
    display_name      text        NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 100),
    password_hash     text        NOT NULL,
    role              text        NOT NULL CHECK (role IN ('admin', 'user')),
    -- The web app's look, as the user set it; fields missing or invalid take their default.
    preferences       jsonb       NOT NULL DEFAULT '{}',
    -- Unset until the address is verified, and for accounts that must verify it.
    email_verified_at timestamptz,
    disabled_at       timestamptz,
    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL
);

-- The hourly cleanup removes accounts that never verified.
CREATE INDEX users_unverified ON users (created_at) WHERE email_verified_at IS NULL;
-- The admins that can sign in, which the last-admin check locks.
CREATE INDEX users_active_admins ON users (id) WHERE role = 'admin' AND disabled_at IS NULL;

-- Browser sessions.
CREATE TABLE sessions (
    id                  uuid PRIMARY KEY,
    user_id             uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash          bytea       NOT NULL UNIQUE,
    user_agent          text        NOT NULL DEFAULT '',
    ip                  text        NOT NULL DEFAULT '',
    created_at          timestamptz NOT NULL,
    -- Written at most once a minute.
    last_seen_at        timestamptz NOT NULL,
    -- Slides forward with use, never past absolute_expires_at.
    expires_at          timestamptz NOT NULL,
    absolute_expires_at timestamptz NOT NULL
);

CREATE INDEX sessions_user ON sessions (user_id);
CREATE INDEX sessions_expires ON sessions (expires_at);

-- API tokens: programmatic access as their user, limited to their scopes.
CREATE TABLE api_tokens (
    id           uuid PRIMARY KEY,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         text        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 64),
    -- The token's first characters, to recognise it in lists.
    prefix       text        NOT NULL,
    token_hash   bytea       NOT NULL UNIQUE,
    scopes       text[]      NOT NULL
        CHECK (cardinality(scopes) > 0 AND scopes <@ ARRAY['read', 'write', 'admin']),
    created_at   timestamptz NOT NULL,
    -- Unset for a token that never expires.
    expires_at   timestamptz,
    last_used_at timestamptz
);

CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- Links mailed to an account: to verify its address, or to set a new password. Each works
-- once, until it expires. Used and expired ones stay a day, so they still count towards
-- the limit of links a day, then the hourly cleanup removes them.
CREATE TABLE email_tokens (
    token_hash bytea PRIMARY KEY,
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    purpose    text        NOT NULL CHECK (purpose IN ('verify', 'reset')),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at    timestamptz
);

CREATE INDEX email_tokens_user ON email_tokens (user_id, purpose, created_at);
