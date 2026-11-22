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
| Invite codes | SHA-256 of the code in lowercase, and a copy sealed with the instance's key when one is configured. |
| Workspace invite links | SHA-256 of the link's token, and a copy sealed with the instance's key when one is configured. |
| Mailed links | SHA-256 of the link's token. |
| Mail waiting to go | Sealed with the instance's key and bound to its job, recipient and link included; deleted once sent or given up on. |
| Secret settings | Sealed with the instance's key. Saving one without a key is refused. |

Session tokens, API tokens, made invite codes, invite links and mailed links are 32 random
bytes from a
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

A request carries one of two credentials:

- `Authorization: Bearer nw_…`, an API token. A bearer header that does not work is refused
  with `401`, without trying the cookie.
- The session cookie, `neon_session`, or `__Host-neon_session` when `server.public_url` is
  https. It is `HttpOnly`, `SameSite=Lax` and `Path=/`, `Secure` over https, and lasts until
  the session's absolute expiry. The `__Host-` prefix makes browsers refuse a cookie of that
  name that a sibling subdomain or a plain-http page tries to set.

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
own. A request with neither header passes. Browsers send `Origin` on every cross-site
request that changes something, and `SameSite=Lax` keeps the session cookie off cross-site
posts besides.

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
`neon_auth_operations_total` and its siblings, by operation, and the workspace use cases in
those of `workspaces`. A wrong password or an expired
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

## The last admin

The last admin who can sign in can be neither demoted nor disabled. Every change that could
leave the instance without one locks the active admins' rows first, so concurrent changes
run one at a time, and each sees the admins as the one before left them.

## Workspaces

Who may do what in a workspace is decided in one function of `neon-domain`, which every
workspace use case calls before it acts; the [permissions](/docs/reference/permissions/)
page lists its rules. What a caller may not know of, a workspace they are not in, a
private teamspace they are not in, or to a guest another member, answers `404 not_found`,
the same as for something that does not exist. The same function refuses API tokens what
cannot be undone, deleting a workspace and transferring its ownership. Being an instance admin counts only for the instance admins' own routes,
which show a workspace's name, owners, counts, limits and usage: inside a workspace, an
instance admin is whatever their membership makes them, which may be nothing.

Every change to a workspace's members locks the workspace's row first, and every change to
a teamspace's members its row too, so concurrent changes run one at a time: a workspace
keeps its last owner, and stays within its member limit, however many changes arrive at
once.

## Cleanup

Once an hour, the server removes expired sessions, mailed links used or expired a day
before, accounts left unverified for 7 days, invites that expired or were revoked 30 days
before, requests to join decided 30 days before, and unconfirmed requests a day after their
link stopped working. It also removes workspace invitations answered, revoked or expired 30
days before, and invite links that expired or were revoked 30 days before.
