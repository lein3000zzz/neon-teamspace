-- Requests to join: someone asks for an invite with their address and a note, confirms
-- the address by a mailed link, and an admin approves, which makes and mails an invite
-- bound to it (invite_id), or denies.
CREATE TABLE join_requests (
    id               uuid PRIMARY KEY,
    email            text        NOT NULL,
    email_normalized text        NOT NULL,
    note             text        NOT NULL DEFAULT '' CHECK (char_length(note) <= 500),
    -- The confirmation link's token hash, until the link is opened.
    token_hash       bytea UNIQUE,
    link_expires_at  timestamptz NOT NULL,
    -- How many links were mailed, and when the last was.
    mails            integer     NOT NULL DEFAULT 1,
    mailed_at        timestamptz NOT NULL,
    created_at       timestamptz NOT NULL,
    confirmed_at     timestamptz,
    decided_at       timestamptz,
    decided_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    approved         boolean,
    invite_id        uuid REFERENCES invites (id) ON DELETE SET NULL,
    CHECK ((decided_at IS NULL) = (approved IS NULL))
);

-- An address has one open request at most. Two asking at once: the second loses here.
CREATE UNIQUE INDEX join_requests_open ON join_requests (email_normalized) WHERE decided_at IS NULL;
-- Whether an address was denied lately, and the cleanup of decided requests.
CREATE INDEX join_requests_decided ON join_requests (email_normalized, decided_at);
