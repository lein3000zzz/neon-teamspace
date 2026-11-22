-- Instance invites: what lets people sign up while registration is by invite. Each admits
-- up to max_uses sign-ups until it expires or an admin revokes it.
CREATE TABLE invites (
    id          uuid PRIMARY KEY,
    -- SHA-256 of the code in lowercase, so codes match whatever their case.
    code_hash   bytea       NOT NULL UNIQUE,
    -- The code, sealed with the instance's secret key and bound to this row, so admins can
    -- copy it again. Unset when no key was configured as it was made.
    code_sealed bytea,
    -- The address it is bound to, which alone signs up with it; unset for any address.
    email       text,
    max_uses    integer     NOT NULL DEFAULT 1 CHECK (max_uses BETWEEN 1 AND 1000),
    uses        integer     NOT NULL DEFAULT 0,
    created_by  uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL,
    expires_at  timestamptz NOT NULL,
    revoked_at  timestamptz,
    revoked_by  uuid REFERENCES users (id) ON DELETE SET NULL,
    CONSTRAINT invites_uses CHECK (uses BETWEEN 0 AND max_uses)
);

-- The hourly cleanup removes invites expired or revoked long enough ago.
CREATE INDEX invites_expires ON invites (expires_at);

-- Who signed up with each invite. The account's address comes from users, so it goes when
-- the account does.
CREATE TABLE invite_uses (
    invite_id uuid        NOT NULL REFERENCES invites (id) ON DELETE CASCADE,
    user_id   uuid REFERENCES users (id) ON DELETE SET NULL,
    used_at   timestamptz NOT NULL
);

CREATE INDEX invite_uses_invite ON invite_uses (invite_id, used_at);
