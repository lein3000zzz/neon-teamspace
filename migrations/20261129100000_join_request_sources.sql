-- Where each request to join was sent from: an IPv4 address, or the /64 network of an IPv6
-- one. The requests nobody confirmed yet are bounded for each source, so that no one source
-- takes the room every other has to ask in; the requests waiting for the admins are bounded
-- apart, and only once confirmed. Requests sent before this have none.
ALTER TABLE join_requests ADD COLUMN source text;

CREATE INDEX join_requests_unconfirmed_source ON join_requests (source)
    WHERE confirmed_at IS NULL AND decided_at IS NULL;
