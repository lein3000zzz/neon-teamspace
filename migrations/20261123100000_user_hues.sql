-- The hue a user pinned for themselves, drawn instead of the one their id hashes to.
ALTER TABLE users ADD COLUMN hue smallint CHECK (hue BETWEEN 0 AND 359);
