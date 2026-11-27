---
title: Troubleshooting
description: The usual failures of an instance by what you see, with the server's own messages and the fix, for a server that does not start, mail that does not go, sign-in and the public URL, live editing through a proxy, OAuth metadata, storage permissions, a full disk, and rate limits behind a proxy.
---

Each section starts from what you see. The server logs one JSON object per line on standard
output, its `message` and its `fields` apart:

```sh
docker compose -f deployments/docker-compose.yml logs server
```

Answers to the API carry `{"error": {"code", "message"}}`; a browser's developer tools show
them under the request, in the network tab.

## The server does not start, or never turns healthy

**A configuration error.** The server checks its configuration first, and prints every
problem at once, each with its key and its variable, then exits:

```txt
Error: invalid configuration:
  server.public_url (NEON_SERVER__PUBLIC_URL): must be only a scheme, host and port, like https://neon.example.com
```

```txt
Error: invalid configuration:
  secrets.keys (NEON_SECRETS__KEYS): key "2026": want 32 bytes in standard base64, such as `openssl rand -base64 32` prints
```

`neon config check` runs the same check without starting anything, and prints the
configuration it read, with secrets redacted:
`docker compose -f deployments/docker-compose.yml run --rm --no-deps server config check`.

**The database refuses the password.**

```txt
Error: connecting to postgres://neon:redacted@postgres:5432/neon

Caused by:
    0: error returned from database: password authentication failed for user "neon"
```

Postgres reads `POSTGRES_PASSWORD` once, when its volume is created. Changed in
`deployments/.env` afterwards, it no longer matches. Put the old one back, or change it in
the database as well:
`docker compose -f deployments/docker-compose.yml exec postgres psql -U neon -d neon -c "ALTER USER neon PASSWORD '<the new one>'"`.

**A migration fails**, or the database was migrated by a newer release: the error starts
with `applying database migrations`, and [Upgrading](/docs/self-hosting/upgrading/#migrations)
covers both.

**Not ready.** The container's health check is `neon health`, which asks the admin
listener's `/readyz` and prints why it is not ready:
`/readyz on 127.0.0.1:9090 answered 503: starting` while the server migrates and starts,
or `the database is unreachable: …` when Postgres stops answering. Run it by hand with
`docker compose -f deployments/docker-compose.yml exec server neon health`.

One warning at start is expected in the Docker stack, `admin listener is bound to a
non-loopback address`: the stack binds it to its own network, for Prometheus, and never
publishes it.

## Mail is not sent

The admin page's **Mail** card, and `mail` in `GET /api/v1/admin/settings`, say whether mail
can go, and the server logs it as it starts: `mail goes out through mail.host`, or
`mail is set up but cannot go` with the problem. The problems, in the order they are checked:

| `problem` | Fix |
|---|---|
| `mail.enabled is off` | Turn `mail.enabled` on. |
| `mail.host is not set` | Set `mail.host`, the relay's host name. |
| `mail.from is not set` | Set `mail.from`, an address the relay lets the account send as. |
| `secrets.keys is not set: mail waits in the queue encrypted, as it carries one-time links` | Set `NEON_SECRETS__KEYS`; see [Secret keys](/docs/self-hosting/configuration/#secret-keys). |
| `server.public_url is not set: the links in mail point there` | Set `NEON_SERVER__PUBLIC_URL`. |

Once mail can go, **Send a test mail** queues one to you. When the relay does not take it,
`last_failure` holds why, and the log has `job failed`, with the error and when it is tried
again. A relay that cannot be reached:

```json
{"kind": "test", "error": "Connection error: failed to lookup address information: Name or service not known", "permanent": false}
```

A wrong host is that one; a `535` answer is a wrong username or password; a certificate
error is a relay whose certificate is self-signed or names another host; a timeout is often
port 587 or 465 blocked between the two. A mail is tried ten times over about an hour and a
half, then logged as `job failed for the last time`. A mail whose link expired while it
waited is dropped with `mail dropped: its link expired before it could be sent`. Mail the
relay took that never arrives went to spam or was refused further on:
[Mail](/docs/self-hosting/mail/#deliverability) has the DNS records receivers check.

## Signing in, or saving anything, fails

**`403 forbidden`, `cross-origin requests that change something are refused`**, and the
server logs `cross-origin request refused` with the `origin` the browser sent. Every request
that changes something must come from `server.public_url`'s origin: scheme, host and port,
as the browser's address bar shows them. `https://neon.example.com` does not match
`http://neon.example.com`, nor `https://www.neon.example.com`, nor the machine's address.
Set `NEON_SERVER__PUBLIC_URL` to the address people use, and only that one.

**Signing in seems to work, then lands on the sign-in page again.** With an `https` public
URL, the session cookie is `__Host-neon_session` and `Secure`, and browsers keep it only
over HTTPS. Reached over plain HTTP, at `http://127.0.0.1:3000/` say, the browser drops it.
Use the instance at its public URL, or leave `server.public_url` unset for plain HTTP on one
machine.

## Live editing does not connect through a proxy

The editor shows **Connecting** or **Reconnecting** and stays there, and a database's views
show **Reconnecting**, while the rest of the app works. Pages and database feeds are
WebSocket connections to `/ws/`, and a proxy in front of Caddy must pass the upgrade on.
In the browser's network tab, the request to `/ws/pages/…` then answers:

- `426 upgrade_required`, `this route opens a WebSocket connection`: the upgrade reached the
  server as a plain `GET`, because a proxy dropped the `Upgrade` and `Connection` headers.
- `403 forbidden`, `WebSocket connections from another origin are refused`, with
  `cross-origin WebSocket upgrade refused` in the log: the page's origin is not
  `server.public_url`'s, as in [the section above](#signing-in-or-saving-anything-fails).

A proxy passes upgrades on over HTTP/1.1 with both headers. With nginx in front of the
stack's `NEON_HTTP_PORT`, for example, in its `http` block and the instance's `server` block:

```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

location / {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection $connection_upgrade;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;
}

location /mcp {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_buffering off;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

`X-Forwarded-For` is set to the client's address alone, for Caddy to believe once
`NEON_WEB_TRUSTED_PROXIES` names the proxy; see [below](#429-rate_limited-behind-a-proxy).
The server pings each connection every 30 seconds, so a proxy that closes connections idle
for longer, as nginx does after 60 seconds, keeps them. `/mcp` answers with server-sent
events, which a proxy must pass on as they come rather than buffer, hence
`proxy_buffering off`.

## MCP and OAuth clients do not connect

A client that reaches `/mcp` without a token gets `401` with
`WWW-Authenticate: Bearer resource_metadata="<origin>/.well-known/oauth-protected-resource"`,
and follows that URL to the OAuth metadata. Ask for it as the client does:

```sh
curl -s https://neon.example.com/.well-known/oauth-authorization-server
```

Its `issuer` and every endpoint it names must be the address the client reached, over
`https` in production. They are `server.public_url`. Unset, the server names itself after
the `Host` of each request, over `https` only when a proxy it trusts says so in
`X-Forwarded-Proto`. Behind TLS that ends in front of Caddy, Caddy says the scheme it was
reached by, `http`, unless it trusts the proxy in front, so the metadata names `http://`
URLs and clients refuse them, or reach the server elsewhere than where they started. Set
`NEON_SERVER__PUBLIC_URL`. The [Claude Code plugin](/docs/guides/claude-code-plugin/) page
has the rest for that client.

## Uploads fail, or files do not show

| What you see | What the log says | Why, and the fix |
|---|---|---|
| `503 storage_unavailable`, `the file store did not answer: try again in a moment` | `the blob store failed`, with `Permission denied (os error 13)` | The server, user 65532, cannot write in `attachments` under its data directory: files put back as another user, or a host directory mounted in its place. In the stack: `docker run --rm -v neon_server-data:/data busybox:1.37 chown -R 65532:65532 /data`. From source, give the directory to the user that runs `neon`. |
| The same | `the blob store failed`, with `No space left on device (os error 28)` | The disk is full; see [below](#a-full-disk). |
| The same | `the blob store failed`, with `SignatureDoesNotMatch` or `lacked the necessary privileges` | The S3 store refuses the access key or its secret: `attachments.s3.access_key_id` and `attachments.s3.secret_access_key` must be the pair the store takes, with the seaweedfs profile `SEAWEEDFS_ACCESS_KEY` and `SEAWEEDFS_SECRET_KEY` of `deployments/.env`. |
| `404 file_missing` | `a file's bytes are missing from the blob store`, `no such blob` | The database names a file the store lacks: a restore without the files, or files left in a store the server no longer points at. See [Backups](/docs/self-hosting/backups/#what-a-partial-restore-loses), and [switching stores](/docs/self-hosting/storage/). |
| `409 storage_limit` | | The workspace has used its storage limit, which an instance admin sets; the disk has nothing to do with it. |

At start, the warning `the blob store attachments.storage names cannot be opened`, with
why, says the settings name a store the server cannot set up.

## A full disk

Postgres, the attachments on local disk and SeaweedFS all write to the disk Docker keeps its
volumes on, `/var/lib/docker` unless set.

- **Attachments** fail to upload with `503 storage_unavailable`, and the server logs
  `No space left on device (os error 28)`. Pages and editing go on.
- **Postgres** refuses writes, and requests that write answer `500 internal`, with
  Postgres's `No space left on device` in the server's log. Once it cannot write its
  write-ahead log either, it stops, and `/readyz` answers `503`,
  `the database is unreachable`.
- **SeaweedFS** turns its volumes read-only when less than 1% of the disk is free, and
  uploads fail as above.

`docker system df` shows what Docker's images, containers and volumes take. Images of
earlier releases pile up with each upgrade, and a clone that builds keeps a build cache:
`docker image prune` and `docker builder prune` free them. Never delete a volume to make
room. Once there is room, Postgres starts again by itself, under `restart: unless-stopped`,
and the server answers again.

## `429 rate_limited` behind a proxy

Signing in answers `429 rate_limited`, `too many attempts: try again in` and a wait of up to
30 seconds, for everyone at once, though nobody tried often. Each client address gets ten
sign-ins, then one more every 30 seconds. When the server counts every request as coming
from one address, the proxy's, everyone shares those ten.

The server counts the address it works out:

- In the Docker stack, Caddy's `X-Forwarded-For`, which holds the address Caddy saw: the
  client's, or, behind a load balancer or another proxy in front of Caddy, that proxy's.
  `NEON_WEB_TRUSTED_PROXIES` in `deployments/.env` names the proxies whose
  `X-Forwarded-For` Caddy believes, such as `10.0.0.0/8`, or `private_ranges` for every
  private range.
- Run from source behind a proxy, `server.trusted_proxies` names the proxies whose last
  `X-Forwarded-For` entry the server believes, loopback unless set; see
  [The client address](/docs/self-hosting/configuration/#the-client-address).

Each session on the account page lists the address it signed in from. When they all show
one address that is not a person's, the proxy in front is not trusted. Name as few
addresses as possible: a client inside a trusted range that reaches Caddy, or the server,
directly could choose the address it is counted by.
