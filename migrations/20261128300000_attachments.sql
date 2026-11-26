-- Attachments: files uploaded to a page, for its image, video, audio and file blocks and its
-- cover. Their bytes are in the blob store the settings name, local disk or an S3 bucket;
-- these rows say what each one is. A file belongs to the page it was uploaded to, whose
-- access it follows, and goes with it.
CREATE TABLE files (
    id                 uuid PRIMARY KEY, -- UUIDv7
    workspace_id       uuid        NOT NULL,
    page_id            uuid        NOT NULL,
    uploaded_by        uuid REFERENCES users (id) ON DELETE SET NULL,
    -- What it was uploaded for: the kind of block, or an image for a cover.
    kind               text        NOT NULL CHECK (kind IN ('image', 'video', 'audio', 'file')),
    -- Where its bytes are in the blob store: made for this file alone, never reused.
    storage_key        text        NOT NULL UNIQUE CHECK (char_length(storage_key) <= 200),
    -- The name it was uploaded with, which a download is saved as.
    name               text        NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255),
    -- Its MIME type as its bytes say, never as its client said.
    content_type       text        NOT NULL CHECK (char_length(content_type) <= 100),
    size_bytes         bigint      NOT NULL CHECK (size_bytes >= 0),
    -- The SHA-256 of its bytes.
    sha256             bytea       NOT NULL CHECK (octet_length(sha256) = 32),
    -- An image's size in pixels, when its header says.
    width              integer CHECK (width > 0),
    height             integer CHECK (height > 0),
    created_at         timestamptz NOT NULL,
    -- Since when no page's document and no cover has shown it, as the hourly cleanup found;
    -- unset while one does. It is deleted once this is older than attachments.orphan_grace.
    unreferenced_since timestamptz,
    CHECK ((width IS NULL) = (height IS NULL)),
    -- What a page's cover refers to, so that it is one of the page's own files.
    UNIQUE (page_id, id),
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE
);

-- A workspace's storage, the sum of its files' sizes, against its limit.
CREATE INDEX files_workspace ON files (workspace_id) INCLUDE (size_bytes);
-- The cleanup's files that no document shows.
CREATE INDEX files_unreferenced ON files (unreferenced_since) WHERE unreferenced_since IS NOT NULL;

-- A page's cover image: one of its own files, in place of a cover drawn from the theme.
-- Deleting the file takes the cover off.
ALTER TABLE pages
    ADD COLUMN cover_file_id uuid,
    ADD CONSTRAINT pages_one_cover CHECK (cover IS NULL OR cover_file_id IS NULL),
    ADD CONSTRAINT pages_cover_file FOREIGN KEY (id, cover_file_id)
        REFERENCES files (page_id, id) ON DELETE SET NULL (cover_file_id);

-- The cleanup's files that a cover shows.
CREATE INDEX pages_cover_file ON pages (cover_file_id) WHERE cover_file_id IS NOT NULL;

-- The files each page's document showed when its last snapshot was saved, by the ids in
-- their paths: of its own files, or of other pages' copied into it. The hourly cleanup keeps
-- what any page shows.
CREATE TABLE page_file_refs (
    page_id uuid NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    file_id uuid NOT NULL,
    PRIMARY KEY (page_id, file_id)
);

CREATE INDEX page_file_refs_file ON page_file_refs (file_id);

-- Blobs to delete from the store: those of files deleted, however they went, due at once,
-- and those of uploads still writing, in case they never finish, due once no upload could
-- still be running. The hourly cleanup deletes each blob, then its row.
CREATE TABLE blob_deletions (
    storage_key text PRIMARY KEY,
    due_at      timestamptz NOT NULL
);

CREATE INDEX blob_deletions_due ON blob_deletions (due_at);

-- A file's row going, with its page, its workspace or on its own, leaves its blob to delete
-- at once.
CREATE FUNCTION files_forget_blob() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    INSERT INTO blob_deletions (storage_key, due_at)
    VALUES (OLD.storage_key, '-infinity')
    ON CONFLICT (storage_key) DO UPDATE SET due_at = '-infinity';
    RETURN NULL;
END;
$$;

CREATE TRIGGER files_forget_blob
    AFTER DELETE ON files
    FOR EACH ROW
EXECUTE FUNCTION files_forget_blob();
