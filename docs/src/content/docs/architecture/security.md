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

Session tokens, API tokens, OAuth codes and tokens, made invite codes, invite links and
mailed links are 32 random bytes from a
CSPRNG, written in base64url. A fast hash is enough for values that random, so they need no
salt and no stretching. Someone who reads the database learns no token that works.

## Passwords

The server hashes passwords with argon2id at OWASP's minimum: 19 MiB of memory, 2 passes, 1
lane. Hashing runs on a thread meant for blocking work, away from the threads that answer
requests. A password may have at most 1024 bytes, which bounds the work one request costs.

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

## Cross-site requests

A request with a method other than `GET`, `HEAD` or `OPTIONS` and no `Authorization` header
must come from the server's own origin. Its `Origin` header, or else its `Referer`, must
match `server.public_url`. When that is unset, it must match the `Host` the request was sent
to. Anything else gets `403`, and the server logs the origin it refused.

Bearer requests are exempt because browsers never add an `Authorization` header on their
own. So are OAuth's registration, token and revocation endpoints, which clients in
browsers of any origin post to and which never read the session cookie, only what the
request carries. A request with neither header passes. Browsers send `Origin` on every cross-site
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
| `login:ip:` | Sign-ins from one IP. Checked first, so a client it stops costs nothing more. |
| `login:ip-email:` | Sign-ins to one address from one IP. |
| `register:ip:` | Sign-ups from one IP. |
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
| `oauth-register:source:` | OAuth clients registering themselves, from one IPv4 address or one IPv6 /64. |

One more key is looser: `login:email:` gives one address 50 sign-ins from every IP together,
then one every 6 seconds. That stops guessing from many places at once, while a stranger
cannot lock the owner out from one place. A request over a limit gets `429` with the code
`rate_limited` and a `Retry-After` header.

Mailed links have limits of their own, kept in the database: one a minute and five a day of
each kind per account, and five confirmation links per request to join.

The limits count by the client's address. The server takes it from the connection, or from
the last `X-Forwarded-For` entry when the connection comes from `server.trusted_proxies`,
which is loopback alone unless set. The Compose stack trusts the private networks, as only
its own containers reach the server there. Caddy in the web image replaces
`X-Forwarded-For` with the one address it sees, so a client cannot choose the address the
server counts. A server process counts on its own, and a second process would need a shared
counter behind the same port.

## Envelope encryption

Secrets the server must read back, such as invite codes and secret settings, are sealed with
keys from `secrets.keys`. Each value gets its own random AES-256-GCM key, which the current
master key wraps. The sealed value names its master key, so keys rotate: put a new key first
and keep the old ones listed until nothing sealed with them is left.

Each sealed value is bound to where it belongs by its associated data, `invite:<id>`,
`workspace-invite-link:<id>` or `setting:<key>`. A sealed value copied to another row does not open there. A value that does
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
  uses was a copy.
- **Every use is checked.** An access token stands for its grant's person or agent, bound
  to its workspace, with no more scopes than the grant has now. The grant is read again on
  every request: it stops when it is revoked, when its person is disabled or leaves the
  workspace, when its agent is suspended, turned off, deleted or handed to someone else, or
  when `mcp.identities` drops its mode. Agents never get `admin`, and the account's and the
  instance admins' routes refuse every OAuth token.
- **Bounded registration.** Registering is rate limited by source, an IPv4 address or an
  IPv6 /64, the clients nobody connected are bounded in number for each source and for all
  together, so that one source cannot take the room every other has, and those are removed
  after a while.
- **Small requests.** The endpoints anyone may call read no more than a request of theirs
  needs: 64 KiB of a registration, 16 KiB and 16 parameters of a form.

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

## Imports and exports

An import reads a file someone uploads, so it is bounded before it starts. The import route
takes a body of at most 32 MiB and a form around it, larger than any other route. The server
asks the rate limit and the rules before it reads the body, and holds one of four places
for imports and exports while one runs, so a few people cannot fill its memory.

A zip is read in memory, as uploaded, and none of its files is ever written to disk. Before
the server parses a zip's directory, it counts the entries the zip says it lists, taking the
largest count of every end record a reader might use, and refuses more than 2000. It refuses
a zip with an entry whose path starts at a root, `/` or a drive letter, or climbs out with
`..`, or holds a NUL, with backslashes taken as slashes. It reads only the deflated or stored
markdown files, at most 4 MiB each and 64 MiB together, and stops reading a file at its limit
whatever size the zip states. A page's document may be at most 16 MiB. The work that unpacks
and parses runs off the async threads, and both imports and exports stop after 20 seconds.
Nothing is made unless all of it can be.

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
works any more, and the clients nobody connected within `oauth.unused_client_ttl`.
