-- Agents and integrations: identities of one workspace each, made by a person there, their
-- owner, for whom they act. They are rows of users, so every column that names who made or
-- changed something, every person value and every membership names them as it names
-- people. They have no address and no password, and never sign in; deleting one keeps its
-- row, so what it made still names it.
ALTER TABLE users
    ADD COLUMN kind         text        NOT NULL DEFAULT 'human'
        CHECK (kind IN ('human', 'agent', 'integration')),
    -- An agent's or integration's workspace, which it goes with.
    ADD COLUMN workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE,
    -- The person it acts for; unset once their account is gone.
    ADD COLUMN owner_id     uuid REFERENCES users (id) ON DELETE SET NULL,
    -- When it was deleted: it is out of its workspace then, and its tokens are gone.
    ADD COLUMN deleted_at   timestamptz;
ALTER TABLE users ALTER COLUMN kind DROP DEFAULT;

ALTER TABLE users
    ALTER COLUMN email DROP NOT NULL,
    ALTER COLUMN email_normalized DROP NOT NULL,
    ALTER COLUMN password_hash DROP NOT NULL,
    -- A person has an address and a password, and no workspace, owner or deletion. An agent
    -- or integration has a workspace, and no address, password, verification, instance
    -- role or disable of its own: whether it works follows its owner.
    ADD CONSTRAINT users_kind CHECK (
        CASE kind
            WHEN 'human' THEN email IS NOT NULL AND email_normalized IS NOT NULL
                AND password_hash IS NOT NULL AND workspace_id IS NULL AND owner_id IS NULL
                AND deleted_at IS NULL
            ELSE email IS NULL AND email_normalized IS NULL AND password_hash IS NULL
                AND workspace_id IS NOT NULL AND role = 'user' AND email_verified_at IS NULL
                AND disabled_at IS NULL
        END
    ),
    -- What workspace_members refers to, so that a membership says what its member is.
    ADD CONSTRAINT users_id_kind UNIQUE (id, kind);

-- Only people verify an address, so only they are ever removed for not doing so.
DROP INDEX users_unverified;
CREATE INDEX users_unverified ON users (created_at)
    WHERE email_verified_at IS NULL AND kind = 'human';
-- A workspace's agents and integrations, deleted ones too, which its content names.
CREATE INDEX users_workspace ON users (workspace_id) WHERE workspace_id IS NOT NULL;
-- The agents a person owns, which change when they leave or are disabled.
CREATE INDEX users_owner ON users (owner_id) WHERE owner_id IS NOT NULL;

-- What each member is. An agent's or integration's role is always member.
ALTER TABLE workspace_members
    ADD COLUMN kind text NOT NULL DEFAULT 'human',
    ADD CONSTRAINT workspace_members_kind FOREIGN KEY (user_id, kind)
        REFERENCES users (id, kind) ON DELETE CASCADE,
    ADD CONSTRAINT workspace_members_delegate_role CHECK (kind = 'human' OR role = 'member');

-- Each workspace may turn its agents off, and say who makes them: its owners and admins,
-- or every member but guests.
ALTER TABLE workspaces
    ADD COLUMN agents_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN agent_creation text    NOT NULL DEFAULT 'members'
        CHECK (agent_creation IN ('admins', 'members'));

-- A token bound to one workspace works there alone. An agent's always is, to the agent's
-- own workspace.
ALTER TABLE api_tokens
    ADD COLUMN workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE;

CREATE INDEX api_tokens_workspace ON api_tokens (workspace_id) WHERE workspace_id IS NOT NULL;
