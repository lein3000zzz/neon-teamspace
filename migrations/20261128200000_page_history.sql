-- Page history: each time the server saves a snapshot of a page's document that shows
-- something other than the version before, the document's blocks are kept as a version, in
-- the same transaction. A database's rows are pages, so their bodies have versions too.
-- The hourly cleanup thins them out as the history.* settings say.
CREATE TABLE page_versions (
    id               uuid PRIMARY KEY, -- UUIDv7
    page_id          uuid        NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    -- When the latest change it holds was made.
    created_at       timestamptz NOT NULL,
    -- Who changed the document since the version before, people, agents and integrations,
    -- by id: the authors of the updates its snapshot took in.
    authors          uuid[]      NOT NULL,
    -- Its blocks, in BlockNote's JSON, as the editor shows them.
    content          jsonb       NOT NULL CHECK (jsonb_typeof(content) = 'array'),
    -- How many blocks it added, changed and removed since the version before, at any depth.
    added            integer     NOT NULL CHECK (added >= 0),
    changed          integer     NOT NULL CHECK (changed >= 0),
    removed          integer     NOT NULL CHECK (removed >= 0),
    -- Set on a version a restore made: the version restored, while it is kept, and when that
    -- one was made, which stays once it is thinned out.
    restored_from    uuid REFERENCES page_versions (id) ON DELETE SET NULL,
    restored_from_at timestamptz,
    CHECK (restored_from IS NULL OR restored_from_at IS NOT NULL)
);

-- A page's versions, latest first, and the cleanup's walk through them a page at a time.
CREATE INDEX page_versions_page ON page_versions (page_id, created_at DESC, id DESC);
-- What a version restored from going clears.
CREATE INDEX page_versions_restored_from ON page_versions (restored_from)
    WHERE restored_from IS NOT NULL;
