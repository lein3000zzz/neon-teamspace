---
title: Security model
description: How the server keeps passwords, sessions, tokens, codes, secret settings and queued mail, how it tells who a request is and what it may do in a workspace, and what it does against guessing and forgery.
---

This page covers accounts and secrets: what the database holds, how a request proves who it
is, and the limits on guessing. [Accounts and registration](/docs/guides/accounts/) covers
the same features from the side of the people using them.

## What the database holds

| Secret | Kept as |
|---|---|
| Passwords | argon2id PHC strings: `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`, a 16-byte salt and a 32-byte hash. |
| Session tokens | SHA-256 of the token. |
| API tokens | SHA-256 of the token, and its first 10 characters to show in lists. |
| OAuth codes, access tokens and refresh tokens | SHA-256 of each. |
| Invite codes | SHA-256 of the code in lowercase, and a copy sealed with the instance's key when one is configured. |
| Workspace invite links | SHA-256 of the link's token, and a copy sealed with the instance's key when one is configured. |
| Mailed links | SHA-256 of the link's token. |
| Mail waiting to go | Sealed with the instance's key and bound to its job, recipient and link included; deleted once sent or given up on. |
| Secret settings | Sealed with the instance's key. Saving one without a key is refused. |
| Outgoing webhooks' signing secrets | Sealed with the instance's key, bound to the webhook; one rotated out is kept while it still signs, a day, and forgotten by the hourly cleanup after. |
| Incoming webhooks' URL tokens | SHA-256 of the token. |
| Incoming webhooks' signing secrets | Sealed with the instance's key, bound to the webhook; one rotated out is kept while it still verifies, a day, and forgotten by the hourly cleanup after. |

Session tokens, API tokens, OAuth codes and tokens, made invite codes, invite links, mailed
links, incoming webhooks' URL tokens and webhooks' signing secrets are 32 random bytes from a
CSPRNG, written in base64url. A fast hash is enough for values that random, so they need no
salt and no stretching. Someone who reads the database learns no token that works.

## Passwords

The server hashes passwords with argon2id at OWASP's minimum: 19 MiB of memory, 2 passes, 1
lane. Hashing runs on a thread meant for blocking work, away from the threads that answer
requests. A password may have at most 1024 bytes, which bounds the work one request costs.
The server hashes or checks as many passwords at once as the machine has processors, so the
memory hashing takes stays bounded however many sign-ins arrive together. A password that
gets no turn within 2 seconds is refused with `503` and the code `busy`, with a
`Retry-After` header, and an unknown address waits for its turn and is refused the same way.
A reset link that does not work is refused before its new password is hashed.

When someone signs in with a password hashed with other parameters, the server hashes it
again with the current ones and saves that. Raising the cost later upgrades every account
as it signs in.

Signing in takes the same time and gives the same answer, `invalid_credentials`, for an
unknown address and for a wrong password. For an unknown address, the server checks the
password against a dummy hash made at start with the same parameters. A disabled or
unverified account is only told so after the right password, so guessing cannot find out
which accounts exist or are disabled.

## How a request proves who it is

A request carries one of three credentials:

- `Authorization: Bearer nw_…`, an API token. A bearer header that does not work is refused
  with `401`, without trying the cookie.
- `Authorization: Bearer nwo_…`, an OAuth access token, read the same way. It stands for the
  same principal as an API token bound to its grant's workspace, acting as the person who
  consented or the agent they chose, while the grant works; see
  [OAuth](#oauth).
- The session cookie, `neon_session`, or `__Host-neon_session` when `server.public_url` is
  https. It is `HttpOnly`, `SameSite=Lax` and `Path=/`, `Secure` over https, and lasts until
  the session's absolute expiry. The `__Host-` prefix makes browsers refuse a cookie of that
  name that a sibling subdomain or a plain-http page tries to set.

Only people sign in. Agents and integrations are rows of the same users table, so whatever
names who made something names them alike, but they have no address and no password: no
sign-in, session, mailed link or invitation finds them, the admins' account list and counts
leave them out, and their role on the instance is never admin. An agent proves who it is with
a token alone, which works only while its owner is a member of its workspace, not a guest,
who can sign in, and agents are on; see [Agents](/docs/guides/agents/).

A session expires when it goes unused for the idle time, or at its absolute expiry. Use
slides the idle expiry forward, never past the absolute one. The server writes the last use
at most once a minute, so a busy session does not cost a write per request. A session of a
disabled account stops working at once.

An unauthenticated request gets `401` with the code `unauthenticated`. A cookie that no
longer works is cleared in the same answer. A request whose role or scopes do not allow the
route gets `403`, with `forbidden` or `insufficient_scope`.

## Scopes

A browser session has every scope its user's role allows. A token has the scopes it was made
with, and its owner's role caps them: only an admin's token may have `admin`, and it stops
working there when its owner stops being an admin. Reading needs `read`, every other method
`write`, and the admin routes `admin`. Changing a password, ending sessions, and making or
revoking tokens need a browser session, so a leaked token cannot lock its owner out or make
more tokens. So do deleting a workspace and transferring its ownership, so a leaked token
cannot take a team's work away.

A token bound to one workspace works on that workspace's routes and sockets alone, is never
an instance admin's, and answers `403 token_bound` on the account's and the admin routes,
on making a workspace and on invite links. On the account's favorites it reaches its own
workspace's alone, and the others answer as pages that are no favorite. Its `admin` scope is
the workspace's admin work, for its owners and admins. An agent's token is always bound, and
never has `admin`.

An admin's token does less than the admin at a browser. Creating an account with a password,
changing a role, and disabling or enabling an account need a browser session; a token
invites people and verifies addresses instead. So does a setting that is a secret or says
where one goes, such as `mail.host`. A leaked admin token therefore cannot make an account
its holder signs in to at a browser, nor make an admin of one it got through an invite, nor
demote or disable any admin, its owner included.

## The MCP server

`/mcp` authenticates every request by its bearer token alone, as the API does for bearer
requests, and takes no cookie, so a web page a person visits cannot use their session
there. The token must be bound to one workspace, and the server refuses one across the
instance with `token_not_bound`. A request without a working token answers `401` with a
`WWW-Authenticate` header that names the resource's metadata, as RFC 9728 has it. Against
DNS rebinding, the server refuses with `403` a request whose `Origin` is not its own. Without
`server.public_url`, a browser's request must also come from `localhost` or an address: a
page that rebinds a name of its own sends it as both `Origin` and `Host`.

Each session belongs to the identity and workspace that opened it, and another identity's
request on it finds no session. The server checks the token on every request. It ends a
session as soon as an access change shows its token stopped working, its agent is
suspended, or its identity left the workspace. A refused request on a session makes it look
again at that session, so one whose token expired ends at its next request. Every five
minutes it also looks again at every session, for what no change tells of. Sessions have
bounds overall, per identity, and per person with their agents together, so one person
cannot take every session through agents they make, and end when idle. The
[MCP guide](/docs/guides/agents-over-mcp/#what-a-connection-is) lists the bounds.

## Cross-site requests

A request with a method other than `GET`, `HEAD` or `OPTIONS` and no `Authorization` header
must come from the server's own origin. Its `Origin` header, or else its `Referer`, must
match `server.public_url`. When that is unset, it must match the `Host` the request was sent
to. Anything else gets `403`, and the server logs the origin it refused.

Bearer requests are exempt because browsers never add an `Authorization` header on their
own. So are OAuth's registration, token and revocation endpoints, which clients in
browsers of any origin post to and which never read the session cookie, only what the
request carries; and posts to an incoming webhook's URL, under `/hooks/in/`: that path takes
no cookie and no `Authorization` header, only the token in it, so a page elsewhere gains
nothing by a visitor's browser posting there. Nothing else is exempt there: its rate limits,
body limit and time limit hold. A request with neither header passes. Browsers send `Origin` on every cross-site
request that changes something, and `SameSite=Lax` keeps the session cookie off cross-site
posts besides.

Those three endpoints and OAuth's metadata allow CORS from any origin, without
credentials, as the OAuth specifications expect of them. No other path sends an
`Access-Control-Allow-Origin` header, so scripts of other origins read nothing else.

A WebSocket upgrade is a `GET`, yet the connection it opens changes pages, and `SameSite=Lax`
does not keep the cookie off it. So an upgrade whose `Origin` is not the server's own gets
`403` too, as [Real-time sync](/docs/architecture/realtime/#connecting) describes. Browsers
always send `Origin` with one.

## Forms that tell nobody anything

The forms anyone can send answer the same whatever comes of them, and do the work after the
answer, so their timing reveals nothing either:

- asking for a password reset link,
- asking for the verification link again,
- asking to join.

Three answers do say whether an address has an account. Signing up with a taken address
answers `email_taken`, which needs an invite while registration is by invite. Mailing an
invite to such an address answers the same, for admins only. Inviting an address into a
workspace answers `no_account` when it has none, for the workspace's owners and admins only,
and each inviter's invitations are rate-limited.

## Rate limits

The server counts attempts in its memory, in a token bucket per key. Each key gets 10
attempts, then one more every 30 seconds:

| Key | What it counts |
|---|---|
| `login:ip:` | Sign-ins from one source. Checked first, so a client it stops costs nothing more. |
| `login:ip-email:` | Sign-ins to one address from one source. |
| `register:ip:` | Sign-ups from one source. |
| `verify:ip:` | Verifying addresses, and asking for the link again. |
| `reset:ip:` | Asking for reset links, and using them. |
| `join:ip:` | Requests to join. |
| `join-confirm:ip:` | Confirming requests to join. |
| `invite:ip:` | Looking invite codes up. |
| `password:user:` | Password changes, by user. |
| `workspace-invite:user:` | Invitations into workspaces by address, by inviter. |
| `workspace-link:user:` | Looking at invite links and joining workspaces by them, by user. |
| `page-import:user:` | Imports of pages, by user. |
| `page-export:user:` | Exports of pages and teamspaces, by user. |
| `oauth-register:source:` | OAuth clients registering themselves, from one source. |
| `oauth-token:grant:` | Exchanges of OAuth codes and refresh tokens, by grant. |
| `hook-in:ip:` | Requests to incoming webhooks from one source, before their URL is looked up: 120, then two a second. |
| `hook-in:hook:` | Requests to one incoming webhook, from anyone: 60, then one a second. |
| `webhook-test:user:` | Test events sent to outgoing webhooks, by user. |

One more key is looser: `login:email:` gives one address 50 sign-ins from every source
together, then one every 6 seconds. That stops guessing from many places at once, while a
stranger cannot lock the owner out from one source: the limits of one source let it spend
fewer attempts than the key gets back. Sign-ins from a source the account holds a session
from, one that has not expired, skip this key, so its owner still signs in from the networks
they use while guesses come from elsewhere. Guesses from many sources together can still
spend it, and then the owner waits as everyone does when signing in from a network they hold
no session on. `mcp:user:` counts the requests of one person or
agent to the MCP server, 200 at once, then four a second, since agents work in bursts.
`mcp:person:` counts them again by person, theirs and their agents' together, 400 at once,
then ten a second, so agents of one person do not multiply their rate.
`page-edit:user:` counts one person's or agent's edits of pages' content, through the
markdown API and MCP's `edit_page` together, 100 at once, then two a second, since each may
go over the whole page. `file-upload:user:` counts one person's or agent's uploads of
attachments, 60 at once, then one a second, as dropping a folder of images uploads each. A
request over a limit gets `429` with the code `rate_limited` and a `Retry-After` header.

Mailed links have limits of their own, kept in the database: one a minute and five a day of
each kind per account, and five confirmation links per request to join.

The limits that count by client count by its source: an IPv4 address, or the /64 network of
an IPv6 address, as one machine, home or office takes any address of its /64 at will. An
IPv4 address mapped into IPv6 is the IPv4 address. The server takes the client's address
from the connection, or from
the last `X-Forwarded-For` entry when the connection comes from `server.trusted_proxies`,
which is loopback alone unless set. The Compose stack trusts the private networks, as only
its own containers reach the server there. Caddy in the web image replaces
`X-Forwarded-For` with the one address it sees, so a client cannot choose the address the
server counts. With `NEON_WEB_TRUSTED_PROXIES` set, Caddy reads the header of the proxies in
front from its end and stops at the first address that is none of theirs, so the entries a
client wrote before it are never read. The server reads the header's last entry alone, as
the proxy next to it wrote it: an entry that is no address, or an address with a port, such
as `198.51.100.1:5555` or `[2001:db8::1]:443`, gives the port up, and anything else that is
no address counts the proxy itself, never an entry before it. A server process counts on its own, and a second process would need a shared
counter behind the same port.

## Envelope encryption

Secrets the server must read back, such as invite codes and secret settings, are sealed with
keys from `secrets.keys`. Each value gets its own random AES-256-GCM key, which the current
master key wraps. The sealed value names its master key, so keys rotate: put a new key first
and keep the old ones listed until nothing sealed with them is left.

Each sealed value is bound to where it belongs by its associated data, `invite:<id>`,
`workspace-invite-link:<id>`, `setting:<key>`, `webhook:<id>` or `incoming-webhook:<id>`. A sealed value copied to another row does not open there. A value that does
not open is skipped with a warning naming its row, never its contents.

[Configuration](/docs/self-hosting/configuration/#secret-keys) shows how to make keys.

## Logs, spans and errors

No password, token or code reaches a log line, a span or an error message. Inputs that carry
one wrap it in a type whose `Debug` output is `[redacted]`. The HTTP layer marks the
`Authorization`, `Cookie` and `Set-Cookie` headers sensitive, and records routes without
their query strings. Logs name accounts by id. A test runs every flow with logging at its
most verbose and checks that none of the secrets it used appears.

The auth use cases are measured in the RED metrics of the `auth` subsystem, as
`neon_auth_operations_total` and its siblings, by operation, the workspace use cases in
those of `workspaces`, and the OAuth server's in those of `oauth`. A wrong password or an expired
link counts as `ok`, as a `4xx` answer does for HTTP, and leaves its span's status unset with
the reason recorded. Only a failure of the server, such as a database that does not answer,
counts as `error`.

## Mail

A mail is queued in the transaction of the change it tells of. Its job's payload is sealed
with the instance's key, its associated data the job's id, so a payload copied to another
row does not open there, and the job is deleted as soon as it is sent or given up on. A
link exists in the clear only in the mail. Without a key, no mail is queued: the flows that
mail links stay off.

Connections to the relay use TLS 1.2 or later and check its certificate, and `starttls`
refuses a relay that does not offer the upgrade rather than going on in the clear. A
password never crosses the network unencrypted: without TLS, the settings allow signing in
only to this host. The mail password, and the settings that say where it goes, change only
from a browser session, so an API token that leaked cannot point the password, or the reset
links, at another server. Errors carry the relay's answer, never the password, and logs
name a mail by its kind, never its recipient or link. See [Mail](/docs/self-hosting/mail/).

The link that stops someone's [notification mail](/docs/guides/inbox/#notification-mail)
works without signing in, so it proves itself: its token is the person's id and an
HMAC-SHA256 of it, with a key derived from the instance's key for that purpose alone, which
the server checks in constant time and keeps nothing for. It does one thing, setting their
mail to none, which they undo in their account; it opens nothing else. The page it opens
waits for a click, so a scanner that follows links stops nothing. A notification mail says
only what its person may see when it is written.

## OAuth

MCP clients and other programs connect through an OAuth 2.1 server, which
[Connecting apps over OAuth](/docs/guides/oauth/) describes. What keeps it safe:

- **Public clients and PKCE.** Clients register themselves and get no secret. Every
  authorization needs PKCE with `S256`; `plain`, which would send the verifier itself
  through the browser, is refused. A code is exchanged only with its verifier, its client
  and the redirect URI it was sent to, within 60 seconds, once.
- **Redirect URIs.** A client registers `https` ones, `http` ones to this machine alone, or
  schemes of native apps' own, never one the browser or the system runs itself, and a
  request names one of them exactly, but for the port of a loopback address. A request
  that does not hold is shown to the person, never sent to the client's redirect URI on its
  own, so the server sends nobody's browser where a stranger registered.
- **Consent at a browser.** Approving and denying need a browser session and come from the
  server's own origin, as every request that changes something with the cookie does; no
  token approves anything. The page names the client as its own word and shows the host the
  browser goes back to in ASCII.
- **Rotation and reuse.** A refresh token works once and is replaced each time. A code or
  refresh token used a second time ends its grant and every token of it, as one of the two
  uses was a copy. Spent refresh tokens are kept until they would have expired, to catch
  that, and each grant exchanges a few at a time, so that a client refreshing in a loop
  keeps no more.
- **Every use is checked.** An access token stands for its grant's person or agent, bound
  to its workspace, with no more scopes than the grant has now. The grant is read again on
  every request: it stops when it is revoked, when its person is disabled or leaves the
  workspace, when its agent is suspended, turned off, deleted or handed to someone else, or
  when `mcp.identities` drops its mode. Agents never get `admin`, and the account's and the
  instance admins' routes refuse every OAuth token, but for the favorites under
  `/api/v1/account/favorites`, which belong to each workspace and which a token bound to
  one reaches for its own workspace alone, as
  [tokens bound to one workspace](/docs/guides/agents/#tokens-bound-to-one-workspace) have it.
- **Bounded registration.** Registering is rate limited by source, an IPv4 address or an
  IPv6 /64, the clients nobody connected are bounded in number for each source and for all
  together, so that one source cannot take the room every other has, and those are removed
  after a while.
- **Small requests.** The endpoints anyone may call read no more than a request of theirs
  needs: 64 KiB of a registration, 16 KiB and 16 parameters of a form.

## Webhooks

Outgoing webhooks connect from inside the server's network to a URL someone chose, so where
they may go is checked, at registration and again before every delivery, never only once.
The server resolves the URL's host itself, checks every address against the ranges the
[Webhooks](/docs/guides/webhooks/#where-webhooks-may-go) guide lists, refusing them all if
one is refused, and connects to an address it checked, without resolving again: DNS
rebinding cannot swap in another address between the check and the connection. IPv4 hosts
in every form a browser reads, and IPv6 addresses that carry an IPv4 one, are checked as the
address they are. Redirects are never followed. Connecting may take 5 seconds, the whole
exchange 10, and at most 64 KiB of an answer is read; resolving a host may take 5 seconds,
at registration too. An identity has at most 10 webhooks and a workspace 200, so a change
fans out to a bounded number of deliveries, which run in slots of their own in the job
worker, never in mail's. Private ranges open only with
`webhooks.allow_private`, which changes only from a browser session; metadata services and
reserved ranges never do.

Deliveries are signed with HMAC-SHA256 over a timestamp and the body, with a secret per
webhook that the server makes, shows once and keeps sealed; receivers refuse timestamps more
than 5 minutes off, which stops replays. A delivery goes out only if its identity may still
see the row, as the rules say when it is sent, and so may whoever set the webhook's URL, who
must also still be allowed to register it; otherwise nothing about the row is sent. So
registering a webhook for someone else, or pointing theirs elsewhere, reveals nothing its
setter could not read, and nobody keeps hearing through a webhook after losing the right to
it. No
log line or span holds a secret, a URL, a payload or an answer's body; the delivery log
keeps the first 256 bytes of an answer and nothing of what was sent.

An incoming webhook is reached by a 256-bit token in its URL, kept as its SHA-256, and may
check signatures in the same scheme. Its body is read only after the client's and the
webhook's rate limits, up to 256 KiB, and only while the webhook is on and its integration
works. Its integration joins no teamspace and gets its target shared with it, so it edits
its target alone, and the rules bound it by its owner's access as every agent's. Its log
keeps the client's address and the outcome of each request, never its body.

## The last admin

The last admin who can sign in can be neither demoted nor disabled. Every change that could
leave the instance without one locks the active admins' rows first, so concurrent changes
run one at a time, and each sees the admins as the one before left them.

## Workspaces

Who may do what in a workspace is decided in one function of `neon-domain`, which every
workspace use case calls before it acts; the [permissions](/docs/reference/permissions/)
page lists its rules. For an agent or integration, the same function is asked as the agent
and as its owner, on the same resource, and allows only what both may. What a caller may not know of, a workspace they are not in, a
private teamspace they are not in, or to a guest another member, answers `404 not_found`,
the same as for something that does not exist. The same function refuses API tokens what
cannot be undone, deleting a workspace and transferring its ownership. Being an instance admin counts only for the instance admins' own routes,
which show a workspace's name, owners, counts, limits and usage: inside a workspace, an
instance admin is whatever their membership makes them, which may be nothing.

Every change to a workspace's members locks the workspace's row first, and every change to
a teamspace's members its row too, so concurrent changes run one at a time: a workspace
keeps its last owner, and stays within its member limit, however many changes arrive at
once.

## Attachments

The server serves uploaded files from the app's own origin, so it keeps each one from acting
as a page of that origin, whatever its bytes are.

- **Types from the bytes.** The server reads a file's type from its first bytes, never from
  its name or the type its client sent. Each kind of block takes only the types it shows, as
  [Attachments](/docs/guides/attachments/#what-each-block-takes) lists. The server serves the
  file as that type, with `X-Content-Type-Options: nosniff`.
- **Downloads but for media.** Only images, video and audio that run no script come
  `inline`. HTML, SVG, PDF, text and every other type come as `attachment`, a download.
- **A sandbox of its own.** Every answer with a file carries
  `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`. A file a
  browser shows as a page anyway runs no script and loads nothing. It gets an origin of its
  own, never the app's, so it reads no cookie and calls no API. The web image's Caddy adds the
  app's own policy beside it, and a browser holds the page to both.
  `Cross-Origin-Resource-Policy: same-origin` keeps other sites from embedding a file.
- **SVG in `<img>` alone.** An SVG image shows in its block through `<img>`, where browsers run
  none of its script and load nothing it names; opened by its own URL, it downloads.
- **Rules on every read.** Each request for a file asks the rules about its page, as reading
  the page would, so shares, guests, the trash and agents' bounds hold. A file the caller may
  not see answers `404`, as one that does not exist. Answers say `Cache-Control: private,
  no-cache`, so no shared cache keeps one and a browser asks again each time. An `ETag`
  spares the bytes while the file is the same.
- **Bounded uploads.** The rules, the caller's rate and a full workspace refuse an upload
  before the server reads its body. The upload then streams to the store, and the server
  counts its bytes as they come, whatever the client declared, stopping at the largest file a
  setting allows. The form around the file is bounded too: its file must start within its
  first 64 KiB, so a body that never gets to its boundary or ends its part headers is refused
  with `413` once past them, rather than held. The server adds its row with the workspace's
  row locked, so uploads at once cannot pass the storage limit together. An upload may take 15 minutes, the one route whose
  time limit is longer than 30 seconds.
- **Keys of the server's own.** The server keeps files under keys it makes from the
  workspace's id and the file's, so no name a client sends reaches a path on disk or a key in
  a bucket.

Logs and spans name a file by its id, type, size and its name's extension, never its name or
its bytes. The S3 store's secret is a secret setting, sealed with the instance's key when
saved. It, and every setting that says where files go, changes only from a browser session.

## Imports and exports

An import reads a file someone uploads, so it is bounded before it starts. The import route
takes a body of at most 32 MiB and a form around it, larger than any other route. The server
asks the rate limit and the rules before it reads the body, and holds one of four places
for imports and exports while one runs, so a few people cannot fill its memory. The work
that runs off the async threads holds the place, and its person's turn, until it ends,
even when the import's deadline has passed and its answer has gone; an export holds them
until its file is sent whole. Every HTTP connection drops a client that takes nothing of
what it is sent for 30 seconds, and with it what its answer held in memory, so a client that
stops reading a large answer, an export or a page's markdown, does not keep it there.

A zip is read in memory, as uploaded, and none of its files is ever written to disk. Before
the server parses a zip's directory, it counts the entries the zip says it lists, taking the
largest count of every end record a reader might use, and refuses more than 2000. It refuses
a zip with an entry whose path starts at a root, `/` or a drive letter, or climbs out with
`..`, or holds a NUL, with backslashes taken as slashes. It reads only the deflated or stored
markdown files, at most 4 MiB each and 64 MiB together, and stops reading a file at its limit
whatever size the zip states. The other files its pages link to are unpacked as uploads,
each at most the largest file and all of them at most 256 MiB together, the budget left
bounding each one as it is read. A page's document may be at most 16 MiB, and the server
counts the parts of each page's markdown, its blocks, table rows and cells, runs of text,
line breaks and mentions, before it writes any into the page's document: at most 100,000 for
one page and 500,000 for the whole import. The work that unpacks, parses and writes runs off
the async threads, checks the deadline between pages, and both imports and exports stop
after 20 seconds. Nothing is made unless all of it can be. Work off the async threads that
is still running when the server stops gets 5 seconds after the shutdown hooks before the
process exits without it.

Exports name their files after titles with the characters file systems refuse taken out, so
a title cannot make a path either. They answer with `Cache-Control: no-store`. The full list
of limits is in [Import and export](/docs/guides/import-export/#limits).

## Cleanup

Once an hour, the server removes expired sessions, mailed links used or expired a day
before, accounts left unverified for 7 days, invites that expired or were revoked 30 days
before, requests to join decided 30 days before, and unconfirmed requests a day after their
link stopped working. It also removes workspace invitations answered, revoked or expired 30
days before, invite links that expired or were revoked 30 days before, and pages that have
been in the trash for `pages.trash_retention`, 30 days unless set, with the pages under
them. OAuth's expired codes and tokens go too, with the grants none of whose codes or tokens
works any more, and the clients nobody connected within `oauth.unused_client_ttl`. So do the
attachments no page's content or cover shows for `attachments.orphan_grace`, and the bytes of
every file deleted, as [Attachments](/docs/guides/attachments/#when-files-are-deleted)
describes.
