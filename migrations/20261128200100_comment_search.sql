-- Comments in search: a page is found by the words of its comments too, weighted least,
-- below its own text. Each comment keeps its words without markup, which the server writes
-- with it, and each page the words of its comments together, which the triggers below keep
-- in the same transaction as the comments they come from.

-- A comment's words: its text without styles or link targets, a mention as `@` and its
-- label. Comments written before this have their markdown, without link targets.
ALTER TABLE comments ADD COLUMN search_text text;
UPDATE comments SET search_text = regexp_replace(body, '\]\([^)]*\)', ']', 'g');
ALTER TABLE comments ALTER COLUMN search_text SET NOT NULL;

-- The words of a page's comments, the newest first, one a line, at most 100000 characters.
-- Only the newest comments whose words come to 400000 bytes or less are read, which their
-- sizes tell without reading them; a comment past that is left out whole.
CREATE FUNCTION page_comment_text(page uuid) RETURNS text
    LANGUAGE sql STABLE AS
$$
    SELECT left(string_agg(s.search_text, E'\n' ORDER BY s.id DESC), 100000)
    FROM (
        SELECT c.id, c.search_text,
               sum(octet_length(c.search_text) + 1) OVER (ORDER BY c.id DESC) AS upto
        FROM comment_threads t
        JOIN comments c ON c.thread_id = t.id
        WHERE t.page_id = page
    ) s
    WHERE s.upto <= 400000
$$;

ALTER TABLE pages ADD COLUMN comment_text text;

-- Comments written, changed or deleted: their pages' words again, once a statement. Those
-- deleted with their threads have no thread left to name their page by; the trigger on
-- threads below takes their pages.
CREATE FUNCTION comments_comment_text() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    WITH pages_of AS (
        SELECT DISTINCT t.page_id AS id
        FROM changed c
        JOIN comment_threads t ON t.id = c.thread_id
    ), texts AS (
        SELECT o.id, page_comment_text(o.id) AS text FROM pages_of o
    )
    UPDATE pages p SET comment_text = x.text
    FROM texts x
    WHERE p.id = x.id AND p.comment_text IS DISTINCT FROM x.text;
    RETURN NULL;
END;
$$;

CREATE TRIGGER comments_inserted_comment_text
    AFTER INSERT ON comments
    REFERENCING NEW TABLE AS changed
    FOR EACH STATEMENT
EXECUTE FUNCTION comments_comment_text();

CREATE TRIGGER comments_updated_comment_text
    AFTER UPDATE ON comments
    REFERENCING NEW TABLE AS changed
    FOR EACH STATEMENT
EXECUTE FUNCTION comments_comment_text();

CREATE TRIGGER comments_deleted_comment_text
    AFTER DELETE ON comments
    REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT
EXECUTE FUNCTION comments_comment_text();

-- Threads deleted, with their comments, as their pages are too: the words of the pages that
-- are still there again.
CREATE FUNCTION comment_threads_comment_text() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    WITH texts AS (
        SELECT o.id, page_comment_text(o.id) AS text
        FROM (SELECT DISTINCT g.page_id AS id FROM gone g) o
    )
    UPDATE pages p SET comment_text = x.text
    FROM texts x
    WHERE p.id = x.id AND p.comment_text IS DISTINCT FROM x.text;
    RETURN NULL;
END;
$$;

CREATE TRIGGER comment_threads_deleted_comment_text
    AFTER DELETE ON comment_threads
    REFERENCING OLD TABLE AS gone
    FOR EACH STATEMENT
EXECUTE FUNCTION comment_threads_comment_text();

-- What full-text search matches, as before, with the words of the page's comments last and
-- weighted least. The column is made again, its index with it.
ALTER TABLE pages DROP COLUMN search_vector;

UPDATE pages p SET comment_text = page_comment_text(p.id)
WHERE EXISTS (SELECT 1 FROM comment_threads t WHERE t.page_id = p.id);

ALTER TABLE pages ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
    setweight(to_tsvector('simple', title), 'A')
    || setweight(to_tsvector('simple', left(COALESCE(property_text, ''), 100000)), 'B')
    || setweight(to_tsvector('simple', regexp_replace(left(COALESCE(property_text, ''), 100000),
                                                      '[^[:alnum:]]+', ' ', 'g')), 'B')
    || setweight(to_tsvector('simple', left(COALESCE(search_text, ''), 100000)), 'C')
    || setweight(to_tsvector('simple', COALESCE(comment_text, '')), 'D')
) STORED;

CREATE INDEX pages_search ON pages USING gin (search_vector);
