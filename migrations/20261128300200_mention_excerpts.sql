-- For a mention in a page's text, the text of the block it is in when it was told, as the
-- inbox and its mail quote it. A comment's notifications quote the comment as it is now.
ALTER TABLE notifications ADD COLUMN excerpt text CHECK (char_length(excerpt) <= 200);
