-- Pages: a workspace's documents, which nest. A page is in a teamspace or private to one
-- member, and its subpages are in the same place. Who may see and change a page is the
-- server's permission rules' to say; the tables only keep what they read.
CREATE TABLE pages (
    id                 uuid PRIMARY KEY, -- UUIDv7
    workspace_id       uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    -- Exactly one of the two: the teamspace it is in, or the member it is private to. A
    -- member's private pages stay while they are out of the workspace, for when they come
    -- back, and go with their account.
    teamspace_id       uuid,
    owner_id           uuid REFERENCES users (id) ON DELETE CASCADE,
    -- The page it is under, in the same workspace and space; unset at the top of its space.
    parent_id          uuid,
    -- Its key among its siblings: digits 0-9A-Za-z that sort byte by byte between its
    -- neighbours' keys, hence the C collation.
    position           text COLLATE "C" NOT NULL
        CHECK (char_length(position) BETWEEN 1 AND 64 AND position ~ '^[0-9A-Za-z]+$'),
    -- Empty shows as Untitled.
    title              text        NOT NULL CHECK (char_length(title) <= 500),
    -- An emoji or a short text.
    icon               text CHECK (char_length(icon) BETWEEN 1 AND 16),
    -- One of the covers drawn from the theme's tokens.
    cover              text CHECK (cover IN ('accent', 'accent-gradient', 'red', 'orange',
                                             'yellow', 'green', 'teal', 'blue', 'violet',
                                             'pink', 'sunrise', 'lagoon', 'aurora', 'dusk')),
    font               text        NOT NULL CHECK (font IN ('default', 'serif', 'mono')),
    small_text         boolean     NOT NULL,
    full_width         boolean     NOT NULL,
    -- The text search reads, written with each snapshot of the page's document.
    search_text        text,
    created_by         uuid REFERENCES users (id) ON DELETE SET NULL,
    last_edited_by     uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at         timestamptz NOT NULL,
    -- When its title, icon, cover, style or place changed.
    updated_at         timestamptz NOT NULL,
    -- When its document changed.
    content_updated_at timestamptz NOT NULL,
    -- Set while it is in the trash. trashed_with is the page whose trashing took it there:
    -- itself, or a page above it trashed with its subpages.
    trashed_at         timestamptz,
    trashed_by         uuid REFERENCES users (id) ON DELETE SET NULL,
    trashed_with       uuid,
    CHECK ((teamspace_id IS NULL) <> (owner_id IS NULL)),
    CHECK ((trashed_at IS NULL) = (trashed_with IS NULL)),
    CHECK (parent_id <> id),
    -- What the foreign keys below refer to.
    UNIQUE (workspace_id, id),
    UNIQUE (teamspace_id, id),
    UNIQUE (owner_id, id),
    FOREIGN KEY (workspace_id, teamspace_id)
        REFERENCES teamspaces (workspace_id, id) ON DELETE CASCADE,
    -- A parent in the same workspace, and in the same teamspace or private pages: each key
    -- holds when its columns are set. Deleting a page deletes the pages under it.
    FOREIGN KEY (workspace_id, parent_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (teamspace_id, parent_id) REFERENCES pages (teamspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (owner_id, parent_id) REFERENCES pages (owner_id, id) ON DELETE CASCADE
);

-- Siblings in order: the pages under a page, at the top of a teamspace, or at the top of a
-- member's private pages, each named by the id of what they are in.
CREATE INDEX pages_siblings
    ON pages (workspace_id, (COALESCE(parent_id, teamspace_id, owner_id)), position)
    WHERE trashed_at IS NULL;
-- The walks down a tree, and deleting the pages under a deleted one.
CREATE INDEX pages_parent ON pages (parent_id);
-- A workspace's trash, latest first: the pages trashed on their own, with what went along.
CREATE INDEX pages_trash ON pages (workspace_id, trashed_at DESC, id DESC)
    WHERE trashed_at IS NOT NULL AND trashed_with = id;
-- The hourly cleanup deletes what has been in the trash long enough.
CREATE INDEX pages_trashed_at ON pages (trashed_at) WHERE trashed_at IS NOT NULL;

-- A page shared with a member or guest of its workspace, to view or to edit it and every
-- page under it. Leaving the workspace ends their shares.
CREATE TABLE page_shares (
    page_id      uuid        NOT NULL,
    workspace_id uuid        NOT NULL,
    user_id      uuid        NOT NULL,
    access       text        NOT NULL CHECK (access IN ('view', 'edit')),
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL,
    updated_at   timestamptz NOT NULL,
    PRIMARY KEY (page_id, user_id),
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, user_id)
        REFERENCES workspace_members (workspace_id, user_id) ON DELETE CASCADE
);

-- The pages shared with someone in a workspace, and the cascade when they leave it.
CREATE INDEX page_shares_user ON page_shares (workspace_id, user_id);

-- Each account's favorite pages, in the order it keeps them, by keys like a page's
-- position. Leaving a workspace drops its pages from them.
CREATE TABLE favorites (
    user_id      uuid        NOT NULL,
    page_id      uuid        NOT NULL,
    workspace_id uuid        NOT NULL,
    position     text COLLATE "C" NOT NULL
        CHECK (char_length(position) BETWEEN 1 AND 64 AND position ~ '^[0-9A-Za-z]+$'),
    created_at   timestamptz NOT NULL,
    PRIMARY KEY (user_id, page_id),
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE,
    FOREIGN KEY (workspace_id, user_id)
        REFERENCES workspace_members (workspace_id, user_id) ON DELETE CASCADE
);

CREATE INDEX favorites_order ON favorites (user_id, position);
-- The cascades from a page and from a membership.
CREATE INDEX favorites_page ON favorites (page_id);
CREATE INDEX favorites_member ON favorites (workspace_id, user_id);

-- Each page's document is a Yjs document. Its updates are logged as they come, in order,
-- until a snapshot takes them in.
CREATE TABLE page_updates (
    page_id    uuid        NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    seq        bigint GENERATED ALWAYS AS IDENTITY,
    update     bytea       NOT NULL,
    author_id  uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL,
    PRIMARY KEY (page_id, seq)
);

-- The latest snapshot of each page's document: its encoded state and state vector, and the
-- last logged update it holds.
CREATE TABLE page_snapshots (
    page_id      uuid PRIMARY KEY REFERENCES pages (id) ON DELETE CASCADE,
    state        bytea       NOT NULL,
    state_vector bytea       NOT NULL,
    last_seq     bigint      NOT NULL,
    updated_at   timestamptz NOT NULL
);
