-- Search: Postgres's full-text search with the simple configuration, which does not stem
-- words and so reads every language alike, over pages' titles, their documents' text and
-- the values of rows' text and URL properties; and pg_trgm's trigrams, which match titles
-- typed loosely. Everything search reads changes in the same transaction as what it comes
-- from: a title as it is saved, a document's text with its snapshot, a row's values with
-- them, through the triggers below. Nothing goes through the job queue.
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- The values of a row's text and URL properties, one a line, in the properties' order.
ALTER TABLE pages ADD COLUMN property_text text;

CREATE FUNCTION row_property_text(database uuid, cells jsonb) RETURNS text
    LANGUAGE sql STABLE AS
$$
    SELECT string_agg(cells ->> p.id::text, E'\n' ORDER BY p.position, p.id)
    FROM database_properties p
    WHERE p.database_id = database AND p.type IN ('text', 'url') AND cells ? p.id::text
$$;

UPDATE pages p SET property_text = row_property_text(r.database_id, r.cells)
FROM database_rows r
WHERE p.id = r.page_id;

-- A row's values changed, or a row was added.
CREATE FUNCTION database_rows_property_text() RETURNS trigger
    LANGUAGE plpgsql AS
$$
DECLARE
    now_text text := row_property_text(NEW.database_id, NEW.cells);
BEGIN
    UPDATE pages SET property_text = now_text
    WHERE id = NEW.page_id AND property_text IS DISTINCT FROM now_text;
    RETURN NULL;
END;
$$;

CREATE TRIGGER database_rows_property_text
    AFTER INSERT OR UPDATE OF cells ON database_rows
    FOR EACH ROW
EXECUTE FUNCTION database_rows_property_text();

-- A property became, or stopped being, text or a URL: every row of its database again. A
-- property deleted takes its cells out of every row, which the trigger above sees.
CREATE FUNCTION database_properties_property_text() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    UPDATE pages p SET property_text = row_property_text(r.database_id, r.cells)
    FROM database_rows r
    WHERE r.database_id = NEW.database_id AND p.id = r.page_id
      AND p.property_text IS DISTINCT FROM row_property_text(r.database_id, r.cells);
    RETURN NULL;
END;
$$;

CREATE TRIGGER database_properties_property_text
    AFTER UPDATE OF type ON database_properties
    FOR EACH ROW
    WHEN (OLD.type IS DISTINCT FROM NEW.type)
EXECUTE FUNCTION database_properties_property_text();

-- What full-text search matches, weighted: the title most, then the values of a row's
-- properties, then the document's text. At most the first 100000 characters of each of the
-- two count, which keeps a large page within what a tsvector holds. The values are read
-- again split at every character but letters and digits, as the parser keeps a URL whole:
-- so `zebra` finds https://example.com/zebra as well as `example.com` does.
ALTER TABLE pages ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
    setweight(to_tsvector('simple', title), 'A')
    || setweight(to_tsvector('simple', left(COALESCE(property_text, ''), 100000)), 'B')
    || setweight(to_tsvector('simple', regexp_replace(left(COALESCE(property_text, ''), 100000),
                                                      '[^[:alnum:]]+', ' ', 'g')), 'B')
    || setweight(to_tsvector('simple', left(COALESCE(search_text, ''), 100000)), 'C')
) STORED;

CREATE INDEX pages_search ON pages USING gin (search_vector);
-- Titles by their trigrams, for the loose match.
CREATE INDEX pages_title_trigrams ON pages USING gin (title gin_trgm_ops);
