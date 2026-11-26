-- Comments, mentions and the inbox.

-- A page may be shared to comment on it: more than to view, less than to edit.
ALTER TABLE page_shares
    DROP CONSTRAINT page_shares_access_check,
    ADD CONSTRAINT page_shares_access_check CHECK (access IN ('view', 'comment', 'edit'));

-- Threads of comments on a page: on the page as a whole, or, inline, on a range of its text
-- between two Yjs relative positions, as Yjs encodes them, with the text it held then.
CREATE TABLE comment_threads (
    id           uuid PRIMARY KEY, -- UUIDv7
    workspace_id uuid        NOT NULL,
    page_id      uuid        NOT NULL,
    anchor_start bytea CHECK (octet_length(anchor_start) BETWEEN 1 AND 256),
    anchor_end   bytea CHECK (octet_length(anchor_end) BETWEEN 1 AND 256),
    quote        text CHECK (char_length(quote) BETWEEN 1 AND 1000),
    -- Set while it is resolved, by whom unless their account is gone.
    resolved_at  timestamptz,
    resolved_by  uuid REFERENCES users (id) ON DELETE SET NULL,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL,
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE,
    CHECK ((anchor_start IS NULL) = (anchor_end IS NULL)),
    CHECK ((anchor_start IS NULL) = (quote IS NULL)),
    CHECK (resolved_by IS NULL OR resolved_at IS NOT NULL)
);

-- A page's threads, oldest first.
CREATE INDEX comment_threads_page ON comment_threads (page_id, id);

-- The comments of a thread, in order. Its first starts it; the others are replies. A
-- comment is markdown limited to inline content, which its author alone changes.
CREATE TABLE comments (
    id         uuid PRIMARY KEY, -- UUIDv7
    thread_id  uuid        NOT NULL REFERENCES comment_threads (id) ON DELETE CASCADE,
    author_id  uuid REFERENCES users (id) ON DELETE SET NULL,
    body       text        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 10000),
    created_at timestamptz NOT NULL,
    -- When its author last changed it.
    edited_at  timestamptz
);

CREATE INDEX comments_thread ON comments (thread_id, id);

-- Who follows a page's comments beyond those who do by default, its creator and those who
-- commented on it or were mentioned on it, and who of those stopped: false.
CREATE TABLE page_follows (
    page_id      uuid        NOT NULL,
    workspace_id uuid        NOT NULL,
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    follows      boolean     NOT NULL,
    updated_at   timestamptz NOT NULL,
    PRIMARY KEY (page_id, user_id),
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE
);

-- Whom a page's text mentions, as its latest snapshot holds it, each with the first block
-- it does so in, so a mention written again later is not told twice.
CREATE TABLE page_mentions (
    page_id  uuid NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    user_id  uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    block_id text CHECK (char_length(block_id) <= 128),
    PRIMARY KEY (page_id, user_id)
);

-- The inbox: what someone is told of, each made in the transaction of what it tells of.
-- What it is about is asked again whenever it is shown, so one whose page its person can
-- no longer see is not shown.
CREATE TABLE notifications (
    id            uuid PRIMARY KEY, -- UUIDv7
    user_id       uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    workspace_id  uuid        NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    kind          text        NOT NULL
        CHECK (kind IN ('mention', 'reply', 'comment', 'assigned', 'invitation')),
    -- Who did what it tells of.
    actor_id      uuid REFERENCES users (id) ON DELETE SET NULL,
    page_id       uuid,
    thread_id     uuid REFERENCES comment_threads (id) ON DELETE CASCADE,
    comment_id    uuid REFERENCES comments (id) ON DELETE CASCADE,
    -- For a mention in a page's text, the block it is in.
    block_id      text CHECK (char_length(block_id) <= 128),
    -- For an assignment, the person property; it may be deleted since.
    property_id   uuid,
    invitation_id uuid REFERENCES workspace_invitations (id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL,
    read_at       timestamptz,
    archived_at   timestamptz,
    -- When a mail told of it, on its own or in a digest.
    mailed_at     timestamptz,
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE,
    CHECK ((kind = 'invitation') = (invitation_id IS NOT NULL)),
    CHECK (kind = 'invitation' OR page_id IS NOT NULL),
    CHECK (kind NOT IN ('reply', 'comment') OR comment_id IS NOT NULL)
);

-- One notification per comment for each person, whichever rule reaches them first.
CREATE UNIQUE INDEX notifications_comment ON notifications (user_id, comment_id)
    WHERE comment_id IS NOT NULL;
-- An inbox, newest first, and its unread count.
CREATE INDEX notifications_inbox ON notifications (user_id, id DESC);
CREATE INDEX notifications_unread ON notifications (user_id)
    WHERE read_at IS NULL AND archived_at IS NULL;
-- What a digest or a mail may still tell of.
CREATE INDEX notifications_unmailed ON notifications (user_id)
    WHERE mailed_at IS NULL AND read_at IS NULL AND archived_at IS NULL;
-- The cascades from a page, a thread, a comment and an invitation.
CREATE INDEX notifications_page ON notifications (workspace_id, page_id)
    WHERE page_id IS NOT NULL;
CREATE INDEX notifications_thread ON notifications (thread_id) WHERE thread_id IS NOT NULL;
CREATE INDEX notifications_comment_id ON notifications (comment_id)
    WHERE comment_id IS NOT NULL;
CREATE INDEX notifications_invitation ON notifications (invitation_id)
    WHERE invitation_id IS NOT NULL;

-- How each person wants mail of their inbox, once they chose, and when their last digest
-- went.
CREATE TABLE notification_preferences (
    user_id        uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    mail           text        NOT NULL CHECK (mail IN ('none', 'each', 'digest')),
    digest_sent_at timestamptz,
    updated_at     timestamptz NOT NULL
);
