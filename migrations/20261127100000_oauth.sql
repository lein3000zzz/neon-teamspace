-- OAuth 2.1, through which MCP clients and other programs connect: the clients that
-- registered themselves, each person's consent to one, a grant, and the codes and tokens of
-- a grant, kept as their SHA-256.

-- A client registered itself (RFC 7591): what it says of itself is its own word. One that
-- no grant uses goes a while after its registration or its last use.
CREATE TABLE oauth_clients (
    id            uuid PRIMARY KEY,
    name          text        NOT NULL,
    redirect_uris text[]      NOT NULL CHECK (cardinality(redirect_uris) BETWEEN 1 AND 10),
    created_at    timestamptz NOT NULL,
    last_used_at  timestamptz
);

CREATE INDEX oauth_clients_unused ON oauth_clients ((COALESCE(last_used_at, created_at)));

-- A person's consent: one client, acting in one workspace as one identity, the person
-- themselves or one of their agents, with scopes. Consenting again to the same client,
-- workspace and identity replaces its scopes and audience.
CREATE TABLE oauth_grants (
    id           uuid PRIMARY KEY,
    client_id    uuid        NOT NULL REFERENCES oauth_clients (id) ON DELETE CASCADE,
    -- Who consented, and answers for it.
    person_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Whom the client acts as: person_id, or one of their agents.
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    workspace_id uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    scopes       text[]      NOT NULL,
    -- The resource its tokens were asked for (RFC 8707): the MCP endpoint or the API.
    audience     text        NOT NULL CHECK (audience IN ('mcp', 'api')),
    created_at   timestamptz NOT NULL,
    updated_at   timestamptz NOT NULL,
    last_used_at timestamptz,
    CONSTRAINT oauth_grants_consent UNIQUE (client_id, person_id, user_id, workspace_id)
);

CREATE INDEX oauth_grants_person ON oauth_grants (person_id);
CREATE INDEX oauth_grants_user ON oauth_grants (user_id);
CREATE INDEX oauth_grants_workspace ON oauth_grants (workspace_id);

-- A code works once, within a minute; used_at says it was, so a second use is caught. It
-- holds what the exchange must match: the redirect URI and the PKCE challenge.
CREATE TABLE oauth_codes (
    hash         bytea PRIMARY KEY,
    grant_id     uuid        NOT NULL REFERENCES oauth_grants (id) ON DELETE CASCADE,
    redirect_uri text        NOT NULL,
    challenge    text        NOT NULL,
    scopes       text[]      NOT NULL,
    audience     text        NOT NULL CHECK (audience IN ('mcp', 'api')),
    created_at   timestamptz NOT NULL,
    expires_at   timestamptz NOT NULL,
    used_at      timestamptz
);

CREATE INDEX oauth_codes_grant ON oauth_codes (grant_id);
CREATE INDEX oauth_codes_expires ON oauth_codes (expires_at);

-- Access tokens and refresh tokens. A refresh token works once: used_at is set as it is
-- exchanged, and it stays until it expires, so one used again shows it was copied.
CREATE TABLE oauth_tokens (
    id         uuid PRIMARY KEY,
    hash       bytea       NOT NULL UNIQUE,
    grant_id   uuid        NOT NULL REFERENCES oauth_grants (id) ON DELETE CASCADE,
    kind       text        NOT NULL CHECK (kind IN ('access', 'refresh')),
    scopes     text[]      NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at    timestamptz
);

CREATE INDEX oauth_tokens_grant ON oauth_tokens (grant_id);
CREATE INDEX oauth_tokens_expires ON oauth_tokens (expires_at);
