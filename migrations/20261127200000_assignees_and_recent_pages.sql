-- Finding what agents and people work on: the rows a person property of their database
-- names them in, and a workspace's pages by when they last changed.

-- A row's cells hold each person property's value as a list of ids, so containment finds
-- the rows naming someone in a given property.
CREATE INDEX database_rows_cells ON database_rows USING gin (cells jsonb_path_ops);

-- A workspace's pages out of the trash, most recently changed first: their title, icon,
-- cover, style or place, or their document.
CREATE INDEX pages_recent ON pages (workspace_id, (GREATEST(updated_at, content_updated_at)) DESC, id)
    WHERE trashed_at IS NULL;
