-- The files each version of a page shows, by the ids in their paths, as page_file_refs holds
-- them for the page's last snapshot: written with the version, in its transaction, and gone
-- with it as the history thins it out. The hourly cleanup keeps what any kept version shows,
-- so restoring one brings its files back.
CREATE TABLE version_file_refs (
    version_id uuid NOT NULL REFERENCES page_versions (id) ON DELETE CASCADE,
    file_id    uuid NOT NULL,
    PRIMARY KEY (version_id, file_id)
);

CREATE INDEX version_file_refs_file ON version_file_refs (file_id);

-- The versions kept before: the paths of their file blocks' URLs and of their links.
INSERT INTO version_file_refs (version_id, file_id)
SELECT DISTINCT v.id, substr(found.path, 15)::uuid
FROM page_versions v
CROSS JOIN LATERAL (
    SELECT u #>> '{}' AS path FROM jsonb_path_query(v.content, 'strict $.**.url') AS u
    UNION
    SELECT h #>> '{}' FROM jsonb_path_query(v.content, 'strict $.**.href') AS h
) found
WHERE found.path ~ '^/api/v1/files/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$';
