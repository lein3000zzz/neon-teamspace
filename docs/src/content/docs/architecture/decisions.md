---
title: Key decisions
description: The choices that shape Neon Teamspace, and the reasons for them.
---

These are the choices that shape Neon Teamspace most. Where a choice was made for a reason,
the reason is given; where it was a starting point, what it gives. Some describe code that
exists, and the rest the design the code is being built toward, marked **planned**.

## The platform

**A Rust server and a TypeScript single-page app.** The server holds every open page's
document in memory, merges the edits of everyone on it, and answers the API, agents and
webhooks from one process. Rust gives that process predictable memory and latency with no
garbage collector, and catches data races at compile time, which matters in a server that
shares live documents between connections. Yjs, the CRDT the editor speaks, has a Rust port,
yrs, so the server reads and writes the same documents the browsers do. The browser side is
TypeScript, where the editor and component libraries are; the app is static files, with no
rendering server to run beside it.

**Six crates, dependencies pointing inward.** `neon-domain` holds the types and the rules,
`neon-blocks` the pages' block documents, `neon-store` the SQL, `neon-app` the use cases,
`neon-server` the HTTP, the command line and the wiring, and `neon-observability` stands to
the side, open to all but the domain and the blocks and installed by the server as it starts. A crate can use only what its `Cargo.toml` lists, so
the direction is kept by the compiler rather than by discipline. The domain cannot reach the
database because it does not depend on sqlx. [More](/docs/architecture/overview/#the-crates).

**Traits only for side effects.** Use cases call the repositories directly, with the
transaction passed in. A trait over every repository would double each signature, and a fake
of a repository tests nothing about its SQL; tests run the real repositories against a real
Postgres instead, a fresh database for each test. Traits, the ports, are kept for what leaves
the process and what tests must replace: mail, the real-time broadcaster, the listeners of
access changes, the rate limiter, webhooks' transport and attachments' storage. They are
held as `Arc<dyn Trait>`: dynamic
dispatch for a few calls per request costs nothing that can be measured, and keeps generic
parameters out of every type that holds one.

**The API is code first.** Each route under `/api/v1` and its OpenAPI entry are declared in
one call, with `utoipa` and `utoipa-axum`, so such a route cannot exist without its
description or the other way round. The paths beside it speak protocols of their own and are
described in their guides instead: `/ws` [real-time sync](/docs/architecture/realtime/),
`/mcp` [MCP](/docs/guides/agents-over-mcp/), `/hooks/in`
[incoming webhooks](/docs/guides/webhooks/#incoming-webhooks) and `/.well-known` the
[OAuth metadata](/docs/guides/oauth/). The description is committed as `api/openapi.json`,
and the app's client types and the [API reference](/docs/reference/api/) are generated from
it. The server's tests fail
when the committed description no longer matches the routes, and the app's checks fail when
its types no longer match the description.

**One error shape.** Every error of the API answers `{"error": {"code", "message"}}`:
`code` is stable, for programs to act on, and `message` is for people. The OAuth endpoints
clients call answer in OAuth's own shape instead, as clients expect.

**One origin.** The app, these docs, the API, real-time sync, agents' endpoint and webhooks
all share one origin behind Caddy. There is no CORS to configure, and the session cookie and
the origin check have one origin to know. [More](/docs/architecture/overview/#one-origin).

**The app on React and TanStack, its types from the API.** React, TypeScript and Vite, with
Bun to install and run, TanStack Router for typed routes and TanStack Query for the server's
data, and shadcn/ui on Radix, styled with Tailwind v4 whose colours are CSS variables, so a
theme is a set of variables. The client's types are generated from `api/openapi.json` with
`openapi-typescript` and called through `openapi-fetch`, so a route the server changes breaks
the app's type check rather than a page. [More](/docs/architecture/web/#the-stack).

## Data

**Postgres only, with sqlx.** One database holds everything lasting, including the job
queue. Postgres has what the features need: transactions, `jsonb`, full-text search and
trigram matching, `SKIP LOCKED` for a queue, and `LISTEN` and `NOTIFY` to wake it. SQL is
written by hand and checked against the schema as the code compiles, through sqlx's macros
and a committed cache of their answers, so a build needs no database. There is no ORM: a
query reads as what runs. [More](/docs/architecture/data/#queries).

**The job queue is a table.** A job is inserted in the same transaction as the change that
caused it, so a rollback leaves no job and a commit always leaves one. A broker outside the
database cannot promise that without a second protocol on top, and would be one more service
to run and back up. Workers claim jobs with `FOR UPDATE SKIP LOCKED`, and a failed job is
tried again with backoff. The queue carries what leaves the process, mail and webhook
deliveries, and notification mail; editing never waits on it, as edits apply to the live
document and search reads text written with the snapshot.
[More](/docs/architecture/data/#the-job-queue).

**An hourly cleanup.** One periodic task deletes what has run out: expired sessions, links
and OAuth grants, accounts left unverified for a week, ended invitations, pages in the
trash past `pages.trash_retention`, versions the history no longer keeps, and attachments
nothing shows any more.

**IDs are UUIDv7.** They are made in the server, sort by creation, and keep indexes compact,
as new IDs land at the end. [More](/docs/architecture/data/#ids).

**Migrations only go forward.** A released migration is never edited; a change is a new one.
The server applies them as it starts unless told not to.

## Content

**One Yjs document per page, with yrs on the server.** Each page is a CRDT document. Edits
made at the same time by several people merge, in any order, to the same result, with no
lock and no central sequencing of operations. A page is the unit that is loaded, shared and
permitted, so it is the unit of the document too. Opening a page loads only that page, and a
workspace's documents do not grow into one. The server runs yrs, so it checks each update
against the editor's permissions, keeps the update log and snapshots in Postgres, and
compacts them; [Real-time sync](/docs/architecture/realtime/) has how. Agents' edits go
through the live document too, as [edits from the server](/docs/architecture/realtime/#edits-from-the-server).

**Rows are pages.** Each row of a database is a page with properties and a body of blocks,
under the database. Sharing, the trash, favorites, comments, history, search and export then
work on rows unchanged, and a card on a board opens as a page.

**Database properties are plain data.** A row's status, assignee or date is a column, not
part of a CRDT. Views filter, sort and group them in SQL, and when two people set one at the
same time, the last write wins and everyone viewing sees it at once. Merging two statuses
has no meaning a CRDT could add.

**Markdown in and out, blocks stored.** A page's blocks, in its Yjs document, are what is
stored; markdown is how content comes in and goes out. Shortcuts work while typing, pasted
markdown becomes blocks, `.md` files and zips import, and pages and teamspaces export as
markdown. Agents read and write markdown over MCP, as language models read and write it well.
Toggles, embedded databases and mentions, which markdown has no syntax for, use a small
extension of it, as callouts will, **planned**. A converter in Rust, `neon-blocks`, maps markdown to the editor's document
schema and back; [Page documents](/docs/architecture/documents/) has the mapping. An agent's
edit is applied to the live document, where it merges with the people typing there. Import
and export exist, as [Import and export](/docs/guides/import-export/) describes, and agents
read and edit pages as markdown with block ids, as
[Agents over MCP](/docs/guides/agents-over-mcp/#block-ids) describes.

**BlockNote for the editor.** A block editor on Yjs, with a shadcn variant that matches the
app's components. Its core is MPL-2.0; its XL packages are GPL and are not used. The block
fixtures `neon-blocks` is tested against are written by BlockNote itself, at the versions the
app uses, and BlockNote loads what `neon-blocks` writes, so the server and the editor agree on
every block.

**Comments beside the document.** Comments are rows in Postgres, not part of a page's Yjs
document, so restoring a version of the page leaves its threads, and who may comment is
decided apart from who may edit. A comment on a passage is anchored with Yjs relative
positions, so it follows the passage as the text around it changes, and its thread stays,
detached, when the passage goes.

**Search in Postgres, written with the page.** Search is Postgres's full-text search with
the `simple` configuration, which does not stem and so reads every language alike, and
`pg_trgm` for titles typed loosely. There is no search engine to run beside it. What search
reads is written in the same transaction as what it comes from, a page's text with its
snapshot and a row's values with them, so it never waits on a queue and a rolled-back write
leaves no trace in it. The query keeps to what the caller may see in the database, a branch
per way of seeing a page, so a guest of a large workspace gets their few pages quickly;
[Data and storage](/docs/architecture/data/#search) has how. There are no embeddings and
no semantic index: the product is about live editing, and an indexing pipeline beside it
does not fit. Agents find pages with the same search people use, and read them whole.

**History from snapshots, restored live.** A version is the page's blocks as each snapshot
leaves them, kept only when they changed, so history costs nothing between saves and keeps
roughly a version per sitting. Restoring one is an edit of the live document, matched by
block ids, on the revision the restorer compared against, so people typing meanwhile see it
at once and a change they made since is never replaced unseen; [Page
history](/docs/guides/history/#restoring) has how.

## Agents

**Agents are members.** An agent is a member of its workspace, of kind agent, with a person
who owns it. It joins teamspaces as a person does and may do what both it and its owner may,
never more than its owner. Its work is shown as its name, `via` its owner, and it is
suspended when its owner leaves. Agents are not a side door. The permission rules, limits and
attribution that hold for people hold for them; [Agents](/docs/guides/agents/) has how.

**MCP inside the server.** The MCP endpoint, `/mcp`, is served by the server itself, with
`rmcp` over Streamable HTTP, not by a service of its own. An agent then reaches the same use
cases through the same checks as a person, and there is nothing more to deploy. It offers
tools and resources, with subscriptions, and no prompts.

**OAuth 2.1 for clients with a person, tokens for the rest.** MCP clients that a person runs
expect an OAuth 2.1 server: dynamic client registration, PKCE, a consent page. Neon Teamspace
is one, and its consent page asks whether the client acts as the person or as one of their
agents. Agents that run on their own, with no browser to sign in from, use a bearer token
instead. A token belongs to one workspace, has the scope read, write or admin, and never does
more than its owner's role allows. Tokens are random and kept as hashes, rather than signed
tokens checked without a lookup, so revoking one stops it at once;
[Connecting apps over OAuth](/docs/guides/oauth/) has the flow.

**Everything MCP does, the API does.** Each MCP tool has an HTTP endpoint with the same
tokens, for scripts and bots that do not speak MCP, and a test holds the table of them.

**Webhooks both ways.** Outgoing webhooks tell an agent that runs on its own of new work,
a row assigned to it or a status changed on one, without polling, and of mentions and
replies. They are signed with HMAC-SHA256, carry the change's trace context, and go through
the job queue with retries. They never reach loopback, private or link-local addresses,
so a webhook cannot make the server call what only it can reach, unless
`webhooks.allow_private` lets them reach a private network, for a receiver beside the
server. Cloud metadata and reserved addresses stay refused either way. Incoming webhooks
are members of kind integration, so what they write carries the same attribution,
permissions and limits as an agent's, and they take a small JSON format of their own rather
than any one sender's; [Webhooks](/docs/guides/webhooks/) has both.

**Agents in their tools.** A Claude Code plugin, with a skill that works through an agent's
tasks over MCP, claiming a card, writing results into it and moving it on, is installed from
this repository's marketplace; [Claude Code plugin](/docs/guides/claude-code-plugin/) has it.

## Accounts and secrets

**Sessions are rows, not signed tokens.** A session is 32 random bytes in an `HttpOnly`
cookie, `__Host-neon_session` over HTTPS, and the database keeps only its SHA-256, as it
does for API tokens and OAuth's tokens. Signing out, a password change or a disabled account
ends sessions at once, with nothing to wait out. By default a session ends when unused for
a week, and in any case 30 days after it began.

**argon2id for passwords.** With 19 MiB of memory, two passes and one lane, in PHC strings,
so the cost can rise later and each account's hash is upgraded at its next sign-in. An
address without an account is checked against a stand-in hash, so it takes as long as a
wrong password.

**An origin check rather than CSRF tokens.** A request that changes something with the
session cookie must come from the public URL's origin, by its `Origin` or `Referer`, and the
cookie is `SameSite=Lax`. Requests with a bearer token are exempt, as browsers never add
one themselves. [More](/docs/architecture/security/#cross-site-requests).

**Forms that tell nobody anything.** Asking for a password reset, a new verification link
or to join answers the same whether or not the address has an account, and does its work
after the answer. A disabled or unverified account says so only after the right password.
The few answers that do tell, such as a workspace invitation to an address without an
account, are for admins alone, and rate limited. [More](/docs/architecture/security/#forms-that-tell-nobody-anything).

**Envelope encryption for the secrets the server reads back.** Invite codes' copies, secret
settings, mail waiting in the queue and webhooks' signing secrets are sealed with AES-256-GCM,
each with a key of its own wrapped by a master key from `secrets.keys`, and bound to their
row. The first key seals and every key listed opens, so keys rotate, and the keys stay in the
deployment's environment, never in the database they protect.
[More](/docs/architecture/security/#envelope-encryption).

**Mail through a relay.** The server hands mail to an SMTP relay the operator chooses, over
STARTTLS or TLS 1.2 and later, and never delivers to recipients' servers itself, which needs
a reputation, PTR records and port 25 that most servers lack. Mail is queued with the change
it reports, sealed, and written when it goes, with the settings of that moment.
[More](/docs/self-hosting/mail/).

## One process, no Redis

The server is one process beside Postgres. The real-time broadcaster, which passes each
page's updates to everyone on it, and the rate limiter both work in memory, behind ports.
That serves a team, or several, with one thing to deploy beside the database and nothing
else to back up.

A second server process would need both shared. Postgres `NOTIFY` cannot carry the updates:
its payloads are capped at 8000 bytes, which a long pasted passage, or the whole state of a
document sent to someone opening it, easily exceeds. Running more than one process therefore
needs Redis adapters for the two ports, which are left for later; because they are ports,
adding them changes the wiring, not the use cases.

## Running it

**Attachments on local disk or any S3 API.** Attachments go through the `object_store`
crate: a directory on local disk by default, which suits one server, or any S3-compatible
service. SeaweedFS is the documented one, Apache-2.0 licensed and small enough to run in a
Compose profile. Its requests go through an HTTP client of the server's own, hyper on rustls
with ring, as webhooks' do, rather than the reqwest and aws-lc stack `object_store` brings by
default.

**Bootstrap configuration apart from settings.** What the server needs before it reaches its
database comes from the environment or `neon.yaml`. Everything else is a runtime setting,
which admins change while the server runs; the environment and the file still win, so an
operator can pin any value with the deployment. Every change is audited.
[More](/docs/self-hosting/configuration/).

**Observability on the ecosystem's facades.** Logs and spans through `tracing`, metrics
through `metrics`, traces exported with OpenTelemetry, the same RED metrics for every
subsystem, and an internal listener for metrics, probes and profiles that is never public.
[More](/docs/architecture/observability/).

**Two images behind Caddy.** A `web` image, Caddy with the app and these docs, which gets
its own TLS certificate given a domain, sets the security headers and believes
`X-Forwarded-For` only from the proxies it is told of, and a `server` image, the `neon`
binary on a minimal base, both for amd64 and arm64, run together by one Compose file with
Postgres, and profiles for Mailpit, SeaweedFS and the observability stack.
[More](/docs/self-hosting/install/#with-docker).

**Releases from tags.** A `v*` tag runs the end-to-end tests against the images built in
the release profile, then publishes both images to GitHub's container registry with an SBOM
and build provenance, and a GitHub release. Images are signed with cosign's keyless signing
once the repository is public, as the signature records the repository in Sigstore's
public log. [More](/docs/self-hosting/upgrading/).

**One Makefile.** Every task, local or in CI, is a target of the root `Makefile`, and `make`
alone lists them, so a check that passes on a laptop is the check CI runs. `make check` runs
everything that needs no Docker and no network.

**Tests against the real thing.** The Rust tests run against Postgres, a fresh database for
each, with fakes only for the ports; the permission rules have tests over every role, action
and resource; the web app's tests run against a stand-in API; and the end-to-end tests drive
the two images, run with Compose, in a browser.

**Docs in the image, generated where they can be.** These docs are an Astro Starlight site
served at `/docs` by the web image. Diagrams are drawn into inline SVG as the docs build, the
API reference is built from `api/openapi.json`, the settings reference from the server's
registry, and a check fails when either falls behind. The inline scripts are allowed by
hash in a Content-Security-Policy of their own.

## The product

**Many workspaces in one instance.** Accounts belong to the instance and its admins decide
who gets one; a person can be in several workspaces. One deployment then serves a team,
several teams, or the people its owner lets in.

**Invite only, to start with.** Registration is by invite unless the admins open it to
anyone or close it. Any account may create a workspace, unless `workspaces.creation` leaves
that to instance admins.

**Four workspace roles, guests among them.** Owners, admins and members work in the
workspace; a guest sees only the pages shared with them, and the pages under those, which
is how a team lets someone outside it into a few pages. A workspace always keeps an owner.

**Teamspaces inside a workspace.** Every workspace has a default teamspace that all its
members but guests are in. The others are open, which any member joins; closed, visible
with members their owners add; or private, which only their members see.

**Access follows the teamspace, shares add to it.** A page takes its access from its
teamspace, and a share gives one person view, comment or edit access to a page and the
pages under it. A person's access is the most any of these gives, and a page they may not
see answers `404`, as if it did not exist.

**Workspace invitations only to existing accounts.** An invitation never creates an
account: accounts are the instance's to give. The invitee accepts or declines in their
inbox. An invite link admits whoever opens it signed in, within its uses and its expiry.

**Limits, not billing.** Instance admins can cap a workspace's storage, members and agents.
A workspace at a limit takes no more uploads, invitations or agents, and its members go on
editing. Agents and integrations count against the agent limit together, and attachments'
bytes against the storage limit.

**Instance admins do not enter workspaces.** Being an instance admin counts only for the
instance's own work: accounts, settings, and the list of workspaces with their limits. Inside
a workspace, an admin is whatever their membership makes them, so running the server never
means reading the teams on it. The permission rules live in one pure function that every use
case calls, with tests over every role, action and resource.
[More](/docs/reference/permissions/).

**A theme of three choices.** Each person picks a palette, Nothing, Soft or Warm, a mode,
and an accent hue. The accent is an OKLCH colour whose lightness and chroma are fixed for
each palette and mode, so text keeps its contrast at any hue, and in dark mode only what
carries a hue glows. People, agents, integrations, workspaces and teamspaces each have a hue
worked out from their id, the same in every browser and on the server, with nothing to store.
[More](/docs/architecture/web/#the-theme-model).

**English, on the web.** The interface is in English, with its strings in one place for
translations to follow. It is a responsive web app. The desktop and mobile apps, **planned**,
are thin Tauri shells: a person enters their server's address, as in Bitwarden's apps, and
the shell loads the web UI that server serves. An app then never falls behind its server,
and there is no second client to build. Tauri, rather than a native toolkit or React Native,
as the editor's stack, BlockNote, ProseMirror and Yjs, exists only in JavaScript; and rather
than Capacitor, which has no desktop target. Push notifications and deep links wait, as the
first needs a relay run by whoever publishes the apps and the second every domain known when
the app is built. No app-store release is planned yet.

## The license

**Source available, for noncommercial use.** Neon Teamspace is licensed under the
[PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0):
anyone may read, run, change and self-host it, for any purpose that is not commercial.

**Dependencies stay permissive.** Every dependency the server links, and every package the
app and these docs ship, is under a permissive license, such as MIT, Apache-2.0, BSD or ISC,
or under MPL-2.0. None is under the GPL, AGPL, LGPL, SSPL or BUSL, or a commercial license.
A copyleft dependency would decide the project's own license for it, and the license would
stop being a choice; MPL-2.0's copyleft stops at the files it covers, so it leaves the rest
free. `cargo-deny` checks the Rust crates against the allowed licenses in `deny.toml`, and
`scripts/licenses.ts` checks the app's and these docs' JavaScript packages against the same
list. The only named exceptions are data files, bundled fonts and packages used only at build
time that never reach a shipped file.
