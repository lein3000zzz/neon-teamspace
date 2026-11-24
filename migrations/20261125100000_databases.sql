-- Databases: a database is a page of kind 'database', and its rows are its subpages, of kind
-- 'row', each with the values of the database's properties. Which property types exist and
-- what their values hold is the server's domain rules' to say; the tables keep what they
-- read.

-- What a page is, kept from its making on.
ALTER TABLE pages
    ADD COLUMN kind text NOT NULL DEFAULT 'page' CHECK (kind IN ('page', 'database', 'row'));
ALTER TABLE pages ALTER COLUMN kind DROP DEFAULT;

-- A database page's own data. Its rows take their numbers from next_number, which only
-- grows, so a number is never given twice, even once its row is deleted.
CREATE TABLE databases (
    page_id      uuid PRIMARY KEY,
    workspace_id uuid    NOT NULL,
    -- Embedded in the page it is under, rather than a page of its own there.
    inline       boolean NOT NULL,
    next_number  bigint  NOT NULL CHECK (next_number >= 1),
    FOREIGN KEY (workspace_id, page_id) REFERENCES pages (workspace_id, id) ON DELETE CASCADE
);

-- A database's properties, in order. type duplicates config's own, for the constraints
-- below: exactly one title, which the server makes with the database and never removes,
-- and at most one unique id.
CREATE TABLE database_properties (
    id          uuid PRIMARY KEY, -- UUIDv7
    database_id uuid  NOT NULL REFERENCES databases (page_id) ON DELETE CASCADE,
    position    text COLLATE "C" NOT NULL
        CHECK (char_length(position) BETWEEN 1 AND 64 AND position ~ '^[0-9A-Za-z]+$'),
    name        text  NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    type        text  NOT NULL CHECK (type IN (
        'title', 'text', 'number', 'select', 'multi_select', 'status', 'date', 'person',
        'checkbox', 'url', 'unique_id', 'created_time', 'created_by', 'last_edited_time',
        'last_edited_by')),
    -- The type with its settings and options, as the domain rules write it.
    config      jsonb NOT NULL CHECK (jsonb_typeof(config) = 'object' AND config ->> 'type' = type)
);

CREATE INDEX database_properties_order ON database_properties (database_id, position);
-- Names are told apart ignoring case.
CREATE UNIQUE INDEX database_properties_name ON database_properties (database_id, lower(name));
CREATE UNIQUE INDEX database_properties_title ON database_properties (database_id)
    WHERE type = 'title';
CREATE UNIQUE INDEX database_properties_unique_id ON database_properties (database_id)
    WHERE type = 'unique_id';

-- A database's views, in order.
CREATE TABLE database_views (
    id          uuid PRIMARY KEY, -- UUIDv7
    database_id uuid  NOT NULL REFERENCES databases (page_id) ON DELETE CASCADE,
    position    text COLLATE "C" NOT NULL
        CHECK (char_length(position) BETWEEN 1 AND 64 AND position ~ '^[0-9A-Za-z]+$'),
    name        text  NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    layout      text  NOT NULL CHECK (layout IN ('table', 'board', 'list')),
    -- Its filter, sorts, grouping and properties, as the domain rules write them.
    config      jsonb NOT NULL CHECK (jsonb_typeof(config) = 'object')
);

CREATE INDEX database_views_order ON database_views (database_id, position);

-- A row's own data: its number in the database, and its cells, the values of the
-- database's stored properties by property id. Its title, place and times are its page's.
-- A change sets or removes single keys of cells, so changes to different properties never
-- undo one another.
CREATE TABLE database_rows (
    page_id     uuid PRIMARY KEY REFERENCES pages (id) ON DELETE CASCADE,
    database_id uuid   NOT NULL REFERENCES databases (page_id) ON DELETE CASCADE,
    number      bigint NOT NULL CHECK (number >= 1),
    cells       jsonb  NOT NULL CHECK (jsonb_typeof(cells) = 'object'),
    -- Finding a row by its unique id, and listing a database's rows.
    UNIQUE (database_id, number)
);
