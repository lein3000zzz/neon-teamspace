-- Workspaces: a team's space, with its members and teamspaces. Accounts belong to the
-- instance, and a person can be in many workspaces. Deleting a workspace deletes all that
-- is in it.
CREATE TABLE workspaces (
    id                            uuid PRIMARY KEY, -- UUIDv7
    name                          text        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    -- For URLs: lowercase letters, digits and single dashes, unique on the instance.
    slug                          text        NOT NULL UNIQUE
        CHECK (char_length(slug) <= 60 AND slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    -- An emoji or a short text.
    icon                          text CHECK (char_length(icon) BETWEEN 1 AND 16),
    -- Unset, it is drawn in the hue its id hashes to.
    hue                           smallint CHECK (hue BETWEEN 0 AND 359),
    members_can_create_teamspaces boolean     NOT NULL DEFAULT true,
    -- Limits an instance admin set for this workspace. Unset follows the instance's
    -- default, from the settings; 0 is no limit.
    limit_members                 bigint CHECK (limit_members >= 0),
    limit_storage_bytes           bigint CHECK (limit_storage_bytes >= 0),
    limit_agents                  bigint CHECK (limit_agents >= 0),
    created_by                    uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at                    timestamptz NOT NULL,
    updated_at                    timestamptz NOT NULL
);

-- Who is in each workspace, and as what. A workspace keeps at least one owner, and stays
-- within its member limit: every change to its members locks the workspace's row first, so
-- such changes run one at a time and each sees the members as the one before left them.
CREATE TABLE workspace_members (
    workspace_id uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role         text        NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'guest')),
    joined_at    timestamptz NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

-- A user's workspaces, for the switcher.
CREATE INDEX workspace_members_user ON workspace_members (user_id);
-- The owners, whom the instance admins' list names.
CREATE INDEX workspace_members_owners ON workspace_members (workspace_id) WHERE role = 'owner';

-- Teamspaces group a workspace's members. Each workspace has one default teamspace, made
-- with it, which everyone but guests is in.
CREATE TABLE teamspaces (
    id           uuid PRIMARY KEY,
    workspace_id uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         text        NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    icon         text CHECK (char_length(icon) BETWEEN 1 AND 16),
    hue          smallint CHECK (hue BETWEEN 0 AND 359),
    access       text        NOT NULL CHECK (access IN ('default', 'open', 'closed', 'private')),
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL,
    updated_at   timestamptz NOT NULL,
    -- What teamspace_members refers to, so that a teamspace's members are its workspace's.
    UNIQUE (workspace_id, id)
);

-- One default teamspace per workspace.
CREATE UNIQUE INDEX teamspaces_one_default ON teamspaces (workspace_id) WHERE access = 'default';

-- Who is in each teamspace. Leaving a workspace leaves its teamspaces.
CREATE TABLE teamspace_members (
    teamspace_id uuid        NOT NULL,
    workspace_id uuid        NOT NULL,
    user_id      uuid        NOT NULL,
    role         text        NOT NULL CHECK (role IN ('owner', 'member')),
    joined_at    timestamptz NOT NULL,
    PRIMARY KEY (teamspace_id, user_id),
    FOREIGN KEY (workspace_id, teamspace_id)
        REFERENCES teamspaces (workspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, user_id)
        REFERENCES workspace_members (workspace_id, user_id) ON DELETE CASCADE
);

-- A member's teamspaces in a workspace, and the cascade when they leave it.
CREATE INDEX teamspace_members_user ON teamspace_members (workspace_id, user_id);

-- Invitations into a workspace. Each names an account that exists, which accepts or
-- declines it; nothing is joined until it accepts.
CREATE TABLE workspace_invitations (
    id           uuid PRIMARY KEY,
    workspace_id uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Owners are made by transferring ownership, never invited.
    role         text        NOT NULL CHECK (role IN ('admin', 'member', 'guest')),
    invited_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL,
    expires_at   timestamptz NOT NULL,
    state        text        NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'accepted', 'declined', 'revoked')),
    -- When it was accepted, declined or revoked.
    decided_at   timestamptz,
    CHECK ((state = 'pending') = (decided_at IS NULL))
);

-- One pending invitation per workspace and account. An expired one is set aside when a new
-- one is made.
CREATE UNIQUE INDEX workspace_invitations_pending
    ON workspace_invitations (workspace_id, user_id) WHERE state = 'pending';
-- The invitations an account waits on.
CREATE INDEX workspace_invitations_user ON workspace_invitations (user_id) WHERE state = 'pending';
-- The hourly cleanup removes invitations answered or expired long enough ago.
CREATE INDEX workspace_invitations_expires ON workspace_invitations (expires_at);

-- Invite links: whoever opens one while signed in joins, up to max_uses people, until it
-- expires or is revoked.
CREATE TABLE workspace_invite_links (
    id           uuid PRIMARY KEY,
    workspace_id uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    role         text        NOT NULL CHECK (role IN ('member', 'guest')),
    -- SHA-256 of the token.
    token_hash   bytea       NOT NULL UNIQUE,
    -- The token, sealed with the instance's secret key and bound to this row, so it can be
    -- copied again. Unset when no key was configured as it was made.
    token_sealed bytea,
    max_uses     integer     NOT NULL CHECK (max_uses BETWEEN 1 AND 1000),
    uses         integer     NOT NULL DEFAULT 0,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL,
    expires_at   timestamptz NOT NULL,
    revoked_at   timestamptz,
    revoked_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    CONSTRAINT workspace_invite_links_uses CHECK (uses BETWEEN 0 AND max_uses)
);

CREATE INDEX workspace_invite_links_workspace ON workspace_invite_links (workspace_id, created_at);
-- The hourly cleanup removes links expired or revoked long enough ago.
CREATE INDEX workspace_invite_links_expires ON workspace_invite_links (expires_at);
