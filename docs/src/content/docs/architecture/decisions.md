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
the process and what tests must replace: mail, attachments' storage, the real-time
broadcaster, the rate limiter and webhooks. They are held as `Arc<dyn Trait>`: dynamic
dispatch for a few calls per request costs nothing that can be measured, and keeps generic
parameters out of every type that holds one.

**The API is code first.** Each route and its OpenAPI entry are declared in one call, with
`utoipa` and `utoipa-axum`, so a route cannot exist without its description or the other way
round. The description is committed as `api/openapi.json`, and the app's client types and
the [API reference](/docs/reference/api/) are generated from it. The server's tests fail
when the committed description no longer matches the routes, and the app's checks fail when
its types no longer match the description.

**One origin.** The app, these docs, the API, real-time sync, agents' endpoint and webhooks
all share one origin behind Caddy. There is no CORS to configure, and the session cookie and
the origin check have one origin to know. [More](/docs/architecture/overview/#one-origin).

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
to run and back up. [More](/docs/architecture/data/#the-job-queue).

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

**Database properties are plain data.** A row's status, assignee or date is a column, not
part of a CRDT. Views filter, sort and group them in SQL, and when two people set one at the
same time, the last write wins and everyone viewing sees it at once. Merging two statuses
has no meaning a CRDT could add. **Planned.**

**Markdown in and out, blocks stored.** A page's blocks, in its Yjs document, are what is
stored; markdown is how content comes in and goes out. Shortcuts work while typing, pasted
markdown becomes blocks, `.md` files and zips import, and pages and teamspaces export as
markdown. Agents read and write markdown over MCP, as language models read and write it well.
Callouts, toggles, mentions and embedded views, which markdown has no syntax for, use a small
extension of it. A converter in Rust, `neon-blocks`, maps markdown to the editor's document
schema and back; [Page documents](/docs/architecture/documents/) has the mapping. An agent's
edit is applied to the live document, where it merges with the people typing there. Import
and export exist, as [Import and export](/docs/guides/import-export/) describes, and agents
read and edit pages as markdown with block ids, as
[Agents over MCP](/docs/guides/agents-over-mcp/#block-ids) describes.

**BlockNote for the editor.** A block editor on Yjs, with a shadcn variant that matches the
app's components. Its core is MPL-2.0; its XL packages are GPL and are not used.
**Planned.**

## Agents

**Agents are members.** An agent is a member of its workspace, of kind agent, with a person
who owns it. It joins teamspaces as a person does and may do what both it and its owner may,
never more than its owner. Its work is shown as its name, `via` its owner, and it is
suspended when its owner leaves. Agents are not a side door. The permission rules, limits and
attribution that hold for people hold for them; [Agents](/docs/guides/agents/) has how.

**MCP inside the server.** The MCP endpoint, `/mcp`, is served by the server itself, with
`rmcp` over Streamable HTTP, not by a service of its own. An agent then reaches the same use
cases through the same checks as a person, and there is nothing more to deploy.

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
assigned or mentioned, without polling. Incoming webhooks are members of kind integration,
so what they write carries the same attribution, permissions and limits as an agent's;
[Webhooks](/docs/guides/webhooks/) has both.

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
Compose profile. **Planned.**

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
its own TLS certificate given a domain, and a `server` image, the `neon` binary on a minimal
base, both for amd64 and arm64, run together by one Compose file.
[More](/docs/self-hosting/install/#with-docker).

## The product

**Many workspaces in one instance.** Accounts belong to the instance and its admins decide
who gets one; a person can be in several workspaces. One deployment then serves a team,
several teams, or the people its owner lets in.

**Invite only, to start with.** Registration is by invite unless the admins open it to
anyone or close it. **Planned.**

**Limits, not billing.** Instance admins can cap a workspace's storage, members and agents.
A workspace at a limit takes no more uploads or invitations, and its members go on editing.
The member limit holds now; storage and agents follow with the features they count.

**Instance admins do not enter workspaces.** Being an instance admin counts only for the
instance's own work: accounts, settings, and the list of workspaces with their limits. Inside
a workspace, an admin is whatever their membership makes them, so running the server never
means reading the teams on it. The permission rules live in one pure function that every use
case calls, with tests over every role, action and resource.
[More](/docs/reference/permissions/).

**English, on the web.** The interface is in English, with its strings in one place for
translations to follow. It is a responsive web app. The desktop and mobile apps, **planned**,
are thin Tauri shells: a person enters their server's address, as in Bitwarden's apps, and
the shell loads the web UI that server serves. An app then never falls behind its server,
and there is no second client to build. No app-store release is planned yet.

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
