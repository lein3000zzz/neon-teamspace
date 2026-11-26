-- Where each OAuth client registered from: an IPv4 address, or the /64 network of an IPv6
-- one. The clients nobody connected are bounded for each source, so that no one source
-- takes the room every other has to register in.
ALTER TABLE oauth_clients ADD COLUMN source text NOT NULL DEFAULT '';
ALTER TABLE oauth_clients ALTER COLUMN source DROP DEFAULT;

CREATE INDEX oauth_clients_source ON oauth_clients (source);
