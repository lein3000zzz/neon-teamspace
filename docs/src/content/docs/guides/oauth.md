---
title: Connecting apps over OAuth
description: The OAuth 2.1 server MCP clients and other programs connect through - discovery, registration, the code flow with PKCE, consent and identities, token lifetimes and rotation, revocation and connected apps.
---

MCP clients, and any other program, connect to Neon Teamspace with OAuth 2.1 instead of a
token someone copies by hand. The program registers itself, sends the person to a consent
page, and gets tokens that act in the one workspace the person chose, as them or as one of
their [agents](/docs/guides/agents/), with the scopes they allowed. Everything the tokens do
goes through the same rules as everything else: an agent may do only what both it and its
owner may.

## How a client connects

1. **Discovery.** A client that calls `/mcp` without a token gets `401` with
   `WWW-Authenticate: Bearer resource_metadata="<public URL>/.well-known/oauth-protected-resource"`.
   That document (RFC 9728) names the resource, `<public URL>/mcp`, and the authorization
   server, the public URL itself, whose metadata is at
   `/.well-known/oauth-authorization-server` (RFC 8414).
2. **Registration.** The client registers itself at the registration endpoint (RFC 7591)
   with its name and its redirect URIs, and gets a `client_id`. Every client is public, with
   no secret: PKCE stands in for one.
3. **Authorization.** The client sends the browser to the authorization endpoint with
   `response_type=code`, its `client_id`, one of its `redirect_uri`s, a PKCE
   `code_challenge` with `code_challenge_method=S256`, and optionally `scope`, `state` and
   `resource`. The endpoint checks them and sends the browser on to the consent page.
4. **Consent.** The person signs in if they have not, then chooses the workspace, whom the
   client acts as and its scopes, and allows or denies it. The browser goes back to the
   redirect URI with a `code`, or with `error=access_denied`, and with `state` and `iss`
   either way.
5. **Tokens.** The client exchanges the code at the token endpoint, with the
   `code_verifier`, for an access token and a refresh token, and sends the access token as
   `Authorization: Bearer nwo_…` to [`/mcp`](/docs/guides/agents-over-mcp/) or to the REST
   API. When it expires, the refresh token gets a new pair.

## Endpoints

| Endpoint | What it is |
|---|---|
| `GET /.well-known/oauth-authorization-server` | The authorization server's metadata (RFC 8414). |
| `GET /.well-known/oauth-protected-resource` | The MCP endpoint's metadata (RFC 9728), also at `/.well-known/oauth-protected-resource/mcp`. |
| `GET /.well-known/oauth-protected-resource/api/v1` | The REST API's metadata. |
| `POST /api/v1/oauth/register` | Dynamic client registration (RFC 7591), JSON. |
| `GET /api/v1/oauth/authorize` | The authorization endpoint, which sends the browser to the consent page. |
| `POST /api/v1/oauth/token` | The token endpoint, a form: `authorization_code` and `refresh_token`. |
| `POST /api/v1/oauth/revoke` | Token revocation (RFC 7009), a form. |

The metadata, the registration, the token endpoint and revocation allow CORS from any
origin, without credentials, so a client running in a browser can use them; nothing else
does. They answer errors in OAuth's own shape, `{"error": "invalid_grant",
"error_description": "…"}`, with:

| Status | When |
|---|---|
| `400` | Most refusals, such as `invalid_request`, `invalid_grant`, `invalid_scope` or `invalid_redirect_uri`. |
| `401` | `invalid_client`: the `client_id` names no client. |
| `403` | `registration_off`: clients do not register themselves while `oauth.registration` is off. |
| `413` | A body larger than the endpoint reads: 64 KiB for a registration, 16 KiB for a form. |
| `429` | `rate_limited`, with `Retry-After`; or `too_many_clients`, too many clients registered from the same place wait unconnected. |
| `503` | `temporarily_unavailable`: too many clients wait unconnected. |
| `500` | `server_error`: the server failed. |

A form holds each parameter once, and at most 16 of them. The routes the consent page and
the connected apps use answer in the API's own shape. The
[API reference](/docs/reference/api/) has every field.

Every URL the metadata names, and the `iss` of every answer, is below `server.public_url`.
Set it in production. Without it, the server names itself after the host each request was
sent to, over `https` when a trusted proxy says it forwarded https: enough on one machine,
but clients compare these URLs, and a public URL keeps them the same everywhere.

## Registering a client

```sh
curl -s https://neon.example.com/api/v1/oauth/register \
  -H 'Content-Type: application/json' \
  -d '{"client_name": "My agent", "redirect_uris": ["http://127.0.0.1:8976/callback"]}'
```

A client registers one to ten redirect URIs, each one of:

- `https`, to any host;
- `http` to this machine alone, `127.0.0.1`, `[::1]` or `localhost`, for a program that
  listens there (RFC 8252);
- a scheme of a native app's own, such as `com.example.app:/callback`, but never one that
  the browser or the system runs or reads itself, such as `javascript:`, `data:`, `file:`,
  `ms-…:` or `x-…:`.

None may hold a fragment, a user name or password, spaces or control characters. Its grant
types are `authorization_code` and `refresh_token`, its response type `code`, and its
`token_endpoint_auth_method` `none`; another method asked for is replaced by `none`, and the
answer says so. Its name, at most 100 characters, is kept without control characters or the
invisible ones that reorder text, and the consent page shows it as the client's own word.

Anyone may register a client, so registering is bounded by where it comes from: an IPv4
address, or the /64 network of an IPv6 address, as one machine, home or office picks its
IPv6 addresses within a /64 at will. Each registers a few at a time, the
`oauth-register:source:` rate limit, and has at most
`oauth.max_unused_clients_per_source` clients that nobody connected at once, past which it
gets `429 too_many_clients` while others still register. At most `oauth.max_unused_clients`
clients that nobody connected wait at once from everywhere together, past which registering
answers `503 temporarily_unavailable`. Either way, connecting one of them, or the hourly
cleanup removing some, makes room. A client nobody connected, or whose connections all
ended, is removed `oauth.unused_client_ttl` after it registered or was last used; it
registers again the next time it connects. With `oauth.registration` off, no client
registers, the metadata names no registration endpoint, and those registered keep working.

## The authorization request

The authorization endpoint checks, in order: the client, the redirect URI, which must be
one the client registered character for character but for the port of an `http` loopback
address, the response type, PKCE with `S256`, whose `plain` method is refused, the scopes,
`read`, `write` and `admin`, the state, at most 1024 printable ASCII characters, and the
resource. Each parameter may be sent once.

A request that does not hold sends the browser to the consent page with its problem, which
the page shows, such as `redirect_uri_mismatch` or `pkce_required`. It is never sent to the
client's redirect URI on its own: anyone may register a client with any redirect URI, and
the server would otherwise send people's browsers wherever a stranger chose (RFC 9700,
section 4.11.2). Only the person's own answer, allowing or denying, sends the browser back.

The `resource` (RFC 8707) is `<public URL>/mcp`, the default, or `<public URL>/api/v1`, the
REST API; anything else is `invalid_target`. Both are this server, which checks every token
itself, so a token issued for either works at both, within its workspace and scopes. The
token endpoint takes a `resource` too: exchanging a code, it must be the one the code was
given for; refreshing, the one the grant was last allowed for, as allowing again replaces
it with the grant's scopes. Another is `invalid_target`, and leaves the code or refresh
token as it was.

## Consent

The consent page is part of the web app, at `/oauth/consent`, and needs a browser session:
someone signed out signs in first and comes back to the same request. It shows the client's
name, with a warning that nobody checked it, and the scheme, host and port the browser goes
back to, with letters of other alphabets written in ASCII so that a lookalike shows as one.

The person chooses:

- **The workspace**, among theirs.
- **Whom the client acts as.** *Act as me* is offered while the setting
  [`mcp.identities`](/docs/reference/settings/) holds `user`. *Act as one of my agents*
  lists their agents in that workspace that work, while `mcp.identities` holds `agent` and
  the workspace has its agents on; guests own no agents. Both are on by default.
- **The scopes.** Every connection reads. `write` lets it change what it acts as may
  change. `admin`, the workspace's admin work, is offered only acting as the person, and
  only to the workspace's owners and admins; an agent never gets it. The scopes the client
  asked for start ticked, read and write when it asked for none.

**Allow** and **Deny** are sent from the session alone, and like every request that changes
something with a cookie, only from the server's own origin; a token cannot approve
anything. Allowing makes a **grant**: the person's consent for that client, workspace and
identity, with those scopes. Allowing the same client, workspace and identity again
replaces the grant's scopes, so allowing fewer narrows every token of it at once, and the
page and database connections they hold open with them: one that edited and may now only
read goes read-only.

## Codes and tokens

| What | Lasts | Kept as |
|---|---|---|
| Authorization code | 60 seconds, once | SHA-256, bound to its client, redirect URI, PKCE challenge, grant, scopes and resource |
| Access token, `nwo_` and 43 characters | `oauth.access_token_ttl`, 1 hour by default | SHA-256 |
| Refresh token, `nwr_` and 43 characters | `oauth.refresh_token_ttl`, 30 days by default, once | SHA-256 |

Each is 32 random bytes, in base64url, shown once. The exchange of a code needs the client
it was given to, the redirect URI it was sent to and the PKCE verifier of its challenge; a
mistake in any of them is `invalid_grant`, and leaves the code to use within its minute.
Each refresh replaces the refresh token with a new one. A refresh may ask for fewer scopes
with `scope`, never more than its refresh token had, and never more than the grant has now.

A code or a refresh token used a second time means one of the two uses was a copy, and
which cannot be told: the grant is revoked, with every token issued from it, the thief's and
the client's alike, and the person connects the client again. So a refresh token spent is
kept until it would have expired, to catch a second use, and each grant exchanges a code or
refresh token a few at a time, 10 at once and then one every 30 seconds, the
`oauth-token:grant:` rate limit, which bounds how many one grant keeps. Past it the token
endpoint answers `429 rate_limited` with `Retry-After`, and the refresh token stays
unspent. A second use is never held back.

## When tokens stop

A grant is read again with every request, so its tokens stop at once when:

- it is revoked, by the client, by the person or, for an agent, by the workspace's owners
  and admins;
- the person is disabled or is no longer in the workspace;
- the agent it acts as is suspended, turned off or deleted, or handed to someone else, which
  revokes its grants as it does its tokens;
- `mcp.identities` no longer holds the mode it acts in.

The page and database connections opened with them close as an API token's do when it
ends. A refresh refused for one of those reasons leaves the refresh token as it was, so a
client whose agent was suspended for a while carries on once it resumes.

An access token acts as a token bound to the grant's workspace: another workspace answers
`404 not_found`, and the account's and the instance admins' routes answer
`403 token_bound`. See [tokens bound to one workspace](/docs/guides/agents/#tokens-bound-to-one-workspace)
and the [permissions](/docs/reference/permissions/) page.

## Revoking and connected apps

A client signs out with the revocation endpoint, with the token and its `client_id`: the
grant of the token ends, with every token of it. A token that is unknown, or another
client's, changes nothing, and the answer is the same.

People see the apps they connected under **Account**, **Connected apps**: each one's
client, workspace, whom it acts as, scopes, and when it was connected and last used, and
**Disconnect** for each, at a browser. The workspace's owners and admins see the apps that act
as any of its agents under the workspace's [Agents](/docs/workspace-settings/agents/)
settings, and the other members those of their own agents, with **Disconnect** too.

| Route | What it does |
|---|---|
| `GET /api/v1/account/connected-apps` | The person's grants, as themselves or as their agents. |
| `DELETE /api/v1/account/connected-apps/{id}` | Revokes one, at a browser. |
| `GET /api/v1/workspaces/{id}/connected-apps` | The grants acting as the workspace's agents that the caller may see. |
| `DELETE /api/v1/workspaces/{id}/connected-apps/{app_id}` | Revokes one, for the agent's owner and the workspace's owners and admins, at a browser. |

The hourly cleanup removes codes and tokens once they expire, grants none of whose codes or
tokens works any more, and clients nobody connected.

## Settings

| Setting | Default | What it does |
|---|---|---|
| `oauth.registration` | `true` | Clients may register themselves. |
| `oauth.access_token_ttl` | `1h` | How long an access token works, 5 minutes to a day. |
| `oauth.refresh_token_ttl` | `30d` | How long a refresh token works, an hour to 90 days, and longer than an access token. |
| `oauth.max_unused_clients` | `1000` | The most clients nobody connected at once. |
| `oauth.max_unused_clients_per_source` | `20` | The most of those from one IPv4 address or IPv6 /64. |
| `oauth.unused_client_ttl` | `1d` | How long such a client is kept after it registered or was last used. |

The [settings reference](/docs/reference/settings/) describes each.

## Observability

The OAuth use cases run in spans named `oauth <operation>`, such as `oauth token` or
`oauth approve`, and are measured as RED metrics of the subsystem `oauth`; a refusal, such as
a reused code, counts as `ok`. No code, token, verifier or secret reaches a log line, a span
or an error message. A grant revoked because a code or refresh token was used twice is
logged as a warning with the grant's id.
