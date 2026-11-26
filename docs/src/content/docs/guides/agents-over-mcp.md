---
title: Agents over MCP
description: Connect an MCP client to a workspace with a token bound to it, or with OAuth, and the tools and resources it gets, claiming tasks, and how pages' blocks are named.
---

The server speaks the [Model Context Protocol](https://modelcontextprotocol.io) at `/mcp`,
on the same origin as the web app. An MCP client, a coding tool in an editor or an agent
that runs on its own, reads and changes one workspace's pages and databases there. It acts
as the identity its token acts as, a person or one of their [agents](/docs/guides/agents/),
and sees what that identity sees and nothing more. Its changes reach everyone at once. An
edit to a page shows in the editors that have the page open, and a row it changes moves on
the boards that show it.

## Connecting with a token

The server works in one workspace per connection, so the token must be bound to one:

- **An agent's token.** Make an [agent](/docs/workspace-settings/agents/), then a token for it
  with the `read` scope, or `read` and `write` to change things. The client acts as the
  agent, for its owner, so what it makes and changes names the agent, `Scout · via Ada`.
- **A person's token bound to a workspace.** Under [API tokens](/docs/guides/accounts/#api-tokens),
  make a token and choose the workspace. The client acts as you.

The server refuses a token across the instance, bound to no workspace, with
`403 token_not_bound`. The setting [`mcp.identities`](/docs/reference/settings/) says whom
clients may act as. Leave `user` out of it and the server refuses people's tokens with
`403 identity_not_allowed`, so only agents connect.

A client takes the server in its configuration as Streamable HTTP with a header. Most read
it in this shape:

```json
{
  "mcpServers": {
    "neon": {
      "type": "http",
      "url": "https://neon.example.com/mcp",
      "headers": { "Authorization": "Bearer nw_…" }
    }
  }
}
```

## Connecting with OAuth

A request to `/mcp` without a working token answers `401` with a header naming where the
server describes itself as a protected resource, as RFC 9728 has it:

```http
WWW-Authenticate: Bearer resource_metadata="https://neon.example.com/.well-known/oauth-protected-resource"
```

A client that supports MCP's authorization reads that document, which names the resource,
`<public URL>/mcp`, then the authorization server's metadata, registers itself, and opens
the consent page in a browser. There you sign in, pick the workspace, and pick whom the
client acts as, you or one of your agents in that workspace, and its scopes. The client gets
an access token bound to that workspace, which works at `/mcp` as any bound token does: an
agent's acts as the agent, within what both it and you may, and the scopes you allowed. Give
such a client the URL alone. [Connecting apps over OAuth](/docs/guides/oauth/) has the
whole flow, the tokens' lifetimes, and how to disconnect an app.

An access token lasts an hour unless
[`oauth.access_token_ttl`](/docs/reference/settings/) says otherwise. A client that gets a
new one with its refresh token before the old one expires carries on with its session, as
the new token is the same identity's; a session whose token expired ends, as below, and the
client opens a new one. Disconnecting the app ends its sessions at once, and a consent that
narrows its scopes narrows its next requests.

## What a connection is

The transport is Streamable HTTP with sessions, as the protocol's revisions 2025-03-26,
2025-06-18 and 2025-11-25 have it. An `initialize` that asks for another revision, an older
one such as 2024-11-05 or one the server does not know, is answered with 2025-11-25, which the
client then speaks or leaves. A client's `initialize` opens a session, which the
`Mcp-Session-Id` header of the answer names and every later request carries. Answers come as
JSON or as server-sent events, and a `GET` opens the stream of notifications. A `DELETE` ends
the session.

- **Every request authenticates.** Each carries the token, and the server checks it every
  time. It refuses a token revoked or expired, an agent suspended, and an identity no
  longer in the workspace at once. It also ends their sessions as soon as one of those
  happens, without waiting for a request, and closes their streams. A token that expires
  tells nobody, so its session ends at its next request, which is refused.
- **Refusals say why.** A token unknown, revoked or expired answers `401 unauthenticated`
  with the challenge above, which then holds `error="invalid_token"` and why, as RFC 6750
  has it; a client gets a new token. An agent's token that works but for its agent answers
  `403` without a challenge, as a new token would not help: `agent_suspended`, `agents_off`
  or `agent_deleted`, as [Agents](/docs/guides/agents/#tokens) describes, with how the agent
  works again. A token whose identity is no longer in the token's workspace answers
  `403 token_unusable`.
- **A session is its identity's.** A request on a session with another identity's token,
  or for another workspace, gets `404`, as for a session that ended. A new token of the same
  identity carries on with the session.
- **Outside a session**, the server takes only `initialize`, and anything else gets
  `400 session_required`.
- **Origin.** The server refuses a request whose `Origin` is not its own with `403`, so a
  web page cannot reach a server on the visitor's machine by rebinding its name. Without
  `server.public_url`, the request's `Host` stands in for the server's origin only when it
  is `localhost` or an address, since a page that rebinds a name of its own sends that name
  as the `Host` too. Clients that are not browsers send no `Origin`, and pass either way.
- **Bounds.** The server holds at most `mcp.max_sessions` sessions,
  `mcp.max_sessions_per_identity` of one person or agent, and `mcp.max_sessions_per_person`
  of one person with their agents together, so nobody takes every session through agents of
  theirs. A session ends with its subscriptions after `mcp.session_idle_timeout` without a
  request or an open stream, and the client starts another when it comes back.
- **Crashes.** A client that crashes leaves its session behind, and one that restarts
  without its session id cannot end it with `DELETE`. So an identity at its own bound that
  opens one more ends its session used least recently with no request in flight and no
  stream open, and a client that crashes in a loop keeps one working session rather than
  locking itself out. Only when each of its sessions is in use, past the person's bound or
  past the server's, does `initialize` get `429 too_many_sessions`, with `Retry-After` saying
  when the next idle session that counts ends, if one is idle.
- **Size.** A request holds at most 2 MiB; a larger one gets `413 payload_too_large`. Send
  large content in several edits, such as several `append` operations.
- **Rate.** Each identity has 200 requests at once, then four a second, and each person 400
  at once, then ten a second, with their agents together. Past that, a request gets
  `429 rate_limited` with `Retry-After`. Edits of pages' content count again on their
  own, 100 at once, then two a second, with those made over REST. Past that, `edit_page`
  answers the tool error `rate_limited`, whose `retry_after` says in seconds when to try
  again, as its message does.
- **Refusals of the request itself**, before any tool or resource runs, answer as the API
  does, `{"error": {"code", "message"}}` with an HTTP status: `400 invalid_json` for a body
  that is not a JSON-RPC message, `406 not_acceptable` for an `Accept` without both
  `application/json` and `text/event-stream`, `415 unsupported_media_type` for a body not
  sent as JSON, `413 payload_too_large`, and those above.

The [settings reference](/docs/reference/settings/) lists the `mcp.*` settings, which apply
without a restart.

## Tools

Each tool answers with JSON as structured content, the objects its
[API route](#the-same-over-rest) answers with, and with that JSON as text. Two answer
otherwise: `create_page` answers as `read_page` does, with the page and its markdown, and
`get_page_tree` nests the pages that its routes list flat, a part at a time. When a tool
cannot do what it was asked, it answers a tool error. Its text says why, and its structured
content holds `{"error": {"code", "message"}}` with the API's code. `not_found` stands for
what is not there and for what the caller may not see, alike. Others are `forbidden`, `invalid_input`,
`insufficient_scope` for a change made with a token without `write`, `rate_limited` with
`retry_after`, and `invalid_arguments`. Each tool publishes the JSON schema of its arguments, and refuses
unknown fields, missing ones and values of the wrong type with `invalid_arguments`, naming
the field, as `` `teamspace_id`: UUID parsing failed ``. A filter that does not read says where
and what a condition holds, as `` `filter`: and[0]: a condition names its `operator`, one of
is, is_not, … ``. Its
annotations tell a client whether it only reads, and whether it may overwrite or remove what
is there, as `edit_page`, `move_page`, `trash_page`, `update_row` and `claim_task` may, so a
client that asks before such a call asks before these.

The tools work in the token's workspace. An id of something in another workspace answers
`not_found`.

### Orientation

| Tool | Arguments | What it answers |
|---|---|---|
| `whoami` | none | The identity, `kind` (`human` or `agent`), an agent's `owner`, the scopes, and the workspace with the role there. |
| `list_members` | none | The members, people and agents, with roles. A guest sees only themselves. |
| `list_teamspaces` | none | The teamspaces the caller sees, the default first, with its role in each. |
| `get_page_tree` | `teamspace_id`, or none for the private pages; `depth`, 1 to 10, 3 unless set | The pages nested to `depth`, each with `kind`, `title`, `icon` and `child_count`. Past `depth`, `children` is left out and `child_count` still counts them. Rows are left out. |

An agent has no private pages, since its owner could not see them, so `get_page_tree`
without a teamspace answers an agent `forbidden`.

### Pages

| Tool | Arguments | What it does |
|---|---|---|
| `read_page` | `page_id` | The page, its blocks as markdown with their ids, and the `revision` they are at; for a row, its `properties`, each with `id`, `name`, `type`, `value` as rows give it, and `text` as people read it. |
| `create_page` | one of `parent_id`, `teamspace_id`, `private: true`; `title`, `icon`, `markdown`, `placement` | Makes the page, its content the blocks of `markdown`, and answers as `read_page` does. |
| `edit_page` | `page_id`, `operations`, `base` | Makes the operations, in order, all or none; see [Editing](#editing-a-page). |
| `move_page` | `page_id`; one of `parent_id`, `teamspace_id`, `private: true`; `placement` | Moves it with the pages under it. |
| `trash_page` | `page_id` | Moves it to the trash with the pages under it. |
| `restore_page` | `page_id` | Brings it back, with the pages trashed with it. |

`placement` is `"first"`, `"last"`, `{"after": id}` or `{"before": id}` beside a sibling, and
`"last"` unless set. A database has no body of its own, so `read_page` and `edit_page` answer
it `invalid_input`. Read it with `get_database` and `query_database` instead.

A page shows an inline database where a line `:::database <id>`, or
`:::database <id> view=<view id>` for one of its views, stands in its markdown. `create_database`
with `inline` and `embed` adds that line at the end of the parent page; without `embed`, an
`insert_after` or `append` of the line puts it where it should go.

### Databases

| Tool | Arguments | What it does |
|---|---|---|
| `get_database` | `database_id` | Its page, properties, each with `id`, `name`, `type` and options, and views, each with its filter, sorts and grouping. |
| `query_database` | `database_id`; `view_id`, `filter`, `sorts`, `limit` 1 to 1000, `after`, `utc_offset_minutes` | Its rows, as a view has them or with a filter and sorts of the caller's, a part at a time. `next` passed as `after` gives the next part. |
| `create_database` | one of `parent_id`, `teamspace_id`, `private: true`; `inline`, `embed`, `title`, `icon`, `placement` | Makes one with a `Name`, a `Status` and two views; inline under `parent_id`, with `embed` its block at the end of that page. |
| `add_property` | `database_id`, `name`, `type`; `options`, `number_format`, `currency`, `prefix`, `placement` | Adds a property, as the [databases reference](/docs/reference/databases/) lists the types, such as a person property for a board an agent claims tasks on. |
| `create_row` | `database_id`; `values`, `placement` | Adds a row with its values. |
| `update_row` | `row_id`, or `database_id` and `unique_id`; `values` | Sets the values named; `null` clears one. |
| `claim_task` | `row_id`, or `database_id` and `unique_id`; `person_property` | Claims the row; see [Claiming a task](#claiming-a-task). |

Values name properties by id or name, and options by name or id. A title, text or URL is a
string, a number a number, a select or status an option, and a multi-select a list of them.
A date is `{"start": "2026-11-22", "end": …}`, a person a list of member ids, a checkbox
`true`. Rows answer with their values by property id, which `get_database` names. A filter is
a condition, `{"property", "operator", "value"}`, or `{"and": […]}` and `{"or": […]}` of
them. In a person condition, `value` may be `me`, whoever asks. The
[databases reference](/docs/reference/databases/#filters) lists the operators.

A unique id is the row's number after its database's prefix, `NW-42`, written in any case,
or the number alone.

```json
{"name": "query_database", "arguments": {
  "database_id": "0193a5b2-0000-7000-8000-0000000000d1",
  "filter": {"and": [
    {"property": "Assignee", "operator": "contains", "value": "me"},
    {"property": "Status", "operator": "in_group", "value": "in_progress"}
  ]},
  "sorts": [{"property": "Due", "direction": "ascending"}]
}}
```

## Block ids

`read_page` gives a page's blocks as markdown, each block's id on a line of its own before
the block, at the block's level. Under a list item the line is indented, and in a quote
quoted.

```markdown
<!-- block 0193a5b2-0000-7000-8000-0000000000b1 -->
# Plan

<!-- block 0193a5b2-0000-7000-8000-0000000000b2 -->
- First
  <!-- block 0193a5b2-0000-7000-8000-0000000000b3 -->
  - Under it
<!-- block 0193a5b2-0000-7000-8000-0000000000b4 -->
- Second
```

The line is `<!-- block `, the id, and ` -->`. An id with anything but ASCII letters,
digits, `-`, `_`, `.` and `~` in it is percent-encoded, and the operations of `edit_page`
take it encoded as shown or decoded. Every block has such a line, an
empty paragraph too, which has no other. A block keeps its id for as long as it is there,
whoever edits around it. The lines are HTML comments, which markdown renderers leave unshown.
The server leaves them out of markdown it reads, wherever they stand but in code, so
markdown sent back with them makes the same blocks as without them. A line with no line of
its block after it, followed by a blank line, another such line or the end, names an empty
paragraph, and reads as one. The rest is the
[markdown the documents use](/docs/architecture/documents/#markdown).

Markdown nests blocks only under list items, toggles and quotes. The blocks someone nests
under a paragraph or a heading show after it, each with its own line, at the same level,
though they are under it. A range of `replace_range` runs between blocks under the same
block, so it cannot start at such a paragraph and end at a block under it.

## Editing a page

`edit_page` takes a list of at most 50 operations and makes them in order, all or none.
When one of them cannot be made, the server makes none and names it, as in
`edit 1: the page has no block …`, with the code `edit_refused`. It makes each through the
page's live document, so whoever has the page open sees it at once. What they type meanwhile
merges with it and stays where they typed it, unless the operation removes that block.

| `op` | Fields | What it does |
|---|---|---|
| `append` | `markdown` | Adds the blocks at the end of the page, before the empty paragraph the editor keeps last. |
| `insert_after` | `block_id`, `markdown` | Adds the blocks right after the block, beside it under the same block. |
| `replace_range` | `from`, `to`, `markdown` | Replaces the blocks from `from` to `to`, siblings in order, both included; `to` is `from` unless set. Empty `markdown` removes them. |
| `find_replace` | `find`, `replace`, `all` | Replaces the text, in every block's text, table cells and code included. |
| `replace_body` | `markdown` | Replaces every block, but those that come out the same, which stay with their ids. The markdown `read_page` gave, sent back unchanged, changes nothing. |

`find_replace` matches the text exactly, character for character, case included, and within
one line, so neither `find` nor `replace` may hold a line break. The replacement takes the
styles and link of the first character it replaces. Unless `all` is set, the text must be in
the page once, and a text found twice answers `edit_refused` saying how many times. With
`all`, it replaces the text at most 10000 times, and a text found more often answers
`edit_refused` too. Each operation answers what it did, with the ids of the blocks it
`added`, how many it `removed` and their ids, `removed_ids`, and how many times it
`replaced` the text.

### Stale reads

People edit a page while an agent thinks about it, and an agent takes seconds or minutes
between reading a page and writing to it. `replace_range` and `replace_body` remove the
blocks they do not keep, so made from a stale read they would remove what someone added or
changed since, and the page keeps no history yet to bring it back from. So:

- `read_page`, the page resource and `GET /api/v1/pages/{id}/markdown` answer the
  `revision` of what they show, which changes whenever it does.
- `edit_page` takes it as `base`. `replace_range` and `replace_body` need it, answering
  `base_required` without it, and once the page is at another revision they answer
  `edit_conflict` and change nothing: read the page again, and make the edit on what it
  holds now.
- `append`, `insert_after` and `find_replace` remove nothing others wrote, and go in
  whatever the revision.
- An edit made on its `base` answers the `revision` it leaves the page at, which is the
  `base` of the next edit, so a series of edits needs one read. One made off its `base`, or
  without one, answers a null `revision`, as the page may hold what the caller has not read.

The page in an edit's answer, as in `read_page`, names who changed its content last and when,
`last_edited_by` and `content_updated_at`, as of the page's last stored snapshot. Those move
as snapshots are stored, a little while after the edits they hold, as
[real-time editing](/docs/architecture/realtime/) describes, so right after an edit they may
still name the change before it, someone else's. The pages `resources/list` gives as changed
last are ordered by the same time.

`replace_body` compares blocks as `read_page` shows them, so send its markdown back with the
lines naming blocks. Those lines keep the empty paragraphs, such as the one the editor keeps
last, and whatever someone types in a block that stays merges with the edit. A block shown
after a paragraph it is under stays while that paragraph does. When the paragraph changes,
the blocks under it are written anew after it.

A page always keeps a block, so removing every one leaves an empty paragraph. A request
holds at most 2 MiB, its markdown included, so large content goes in several `append`
operations, each in a call of its own. A page holds at
most 16 MiB, and an edit past that answers `document_too_large`. The server weighs what
`find_replace` would add before it adds any of it, so replacements that would grow the page
past that are refused at once. Markdown, an edit's or `create_page`'s, nests blocks at most
32 levels deep, as [Page documents](/docs/architecture/documents/) describes; deeper answers
`invalid_input` and changes nothing.

```json
{"name": "edit_page", "arguments": {
  "page_id": "0193a5b2-0000-7000-8000-0000000000a1",
  "operations": [
    {"op": "find_replace", "find": "Status: open", "replace": "Status: done"},
    {"op": "insert_after", "block_id": "0193a5b2-0000-7000-8000-0000000000b4",
     "markdown": "## Results\n\n- Login fixed\n- Tests added"}
  ]
}}
```

## Claiming a task

A task is a row of a database, and its assignees are the people and agents its person
properties name. `claim_task` takes a row nobody holds, in one transaction that holds the
row.

1. It finds the person property, the one `person_property` names by id or name, or else
   the database's first person property in its order.
2. While that property names nobody, it names the caller there, and moves the row to the
   first option in progress of the database's status property, its first.
3. Otherwise it changes nothing, and answers `already_claimed`, its message naming who holds
   it and its structured content listing them as `holders`, each with `id` and `name`.

Claims of one row at the same time take their turn on it, so exactly one wins and the
others hear who holds it. A database without a person property, or without a status property
with an option in progress, answers `invalid_input`; a database an agent makes has a status
but no person property, which `add_property` with `"type": "person"` adds. Claiming needs edit access to the row,
and guests, who know no other member, do not claim. A claim is the caller's change of the
row, and those following the database see it at once.

A claim, as every `create_row` and `update_row`, tells [outgoing webhooks](/docs/guides/webhooks/#events)
too: `row.assigned` for whoever it names, and `row.status_changed` for everyone the row's
person properties name. The caller's own webhooks hear of their own claims only with
`include_own`, so an agent that runs elsewhere learns of its work without polling.

## Resources

| URI | What it reads |
|---|---|
| `neon://page/{id}` | The page's blocks as markdown with their ids, `text/markdown`, then the page as JSON, with a row's properties. |
| `neon://database/{id}` | The database's page, properties and views, as JSON. |
| `neon://database/{id}/view/{view}` | The rows the view shows, in its order, as JSON, at most 1000. |
| `neon://me/tasks` | The rows whose person properties name the caller, out of the trash, the most recently changed first, at most 100, each with its database's title and its status. |

A URI writes its ids lowercase and hyphenated, as the server lists them and tells of their
changes, and a URI that writes them another way is not found.

`resources/list` gives `neon://me/tasks`, then the caller's favorites, at most 50, the 20
pages changed last, and up to 50 tasks, each resource once. An agent's favorites are its
own.

`resources/subscribe` works on every one of them. The server then sends
`notifications/resources/updated` for it, at most once every `mcp.notification_interval`,
two seconds unless set. It tells of the first change at once, and of those within the
interval after it together, once the interval ends. It hears of changes as they happen and
never asks the database again to find them. A subscription hears of:

- a page when its document changes, and a row's page when its values change too;
- a database when its properties or views change;
- a view's rows when a row of the database, its schema or that view changes;
- the tasks when what the caller sees of them changes: a row of theirs changes, or a row
  starts or stops naming them. A change in a database the caller cannot see is never told.

A subscription ends with its session, with `resources/unsubscribe`, and as soon as the caller
can no longer see the resource. A session subscribes to at most 100 resources.

A resource request that cannot be answered gets a JSON-RPC error whose `data` holds the API's
code, as `{"code": "not_found"}`: `-32002` for what is not there or not the caller's to see,
`-32602` for the rest, such as `too_many_subscriptions`. The server offers no prompts and no
completions: asking for them gets `-32601`, saying so.

## The same over REST

For programs that do not speak MCP, a route of the [API](/docs/reference/api/) does what
each tool does, with the same tokens.

| Tool or resource | Route |
|---|---|
| `whoami` | `GET /api/v1/whoami` |
| `list_members` | `GET /api/v1/workspaces/{id}/members` |
| `list_teamspaces` | `GET /api/v1/workspaces/{id}/teamspaces` |
| `get_page_tree` | `GET /api/v1/teamspaces/{id}/pages`, `GET /api/v1/workspaces/{id}/private-pages` |
| `read_page`, `neon://page/{id}` | `GET /api/v1/pages/{id}/markdown` |
| `create_page` | `POST /api/v1/workspaces/{id}/pages`, with `markdown` |
| `edit_page` | `PATCH /api/v1/pages/{id}/markdown` |
| `move_page`, `trash_page`, `restore_page` | `POST /api/v1/pages/{id}/move`, `/trash`, `/restore` |
| `get_database`, `neon://database/{id}` | `GET /api/v1/databases/{id}` |
| `query_database`, `neon://database/{id}/view/{view}` | `POST /api/v1/databases/{id}/query` |
| `create_database` | `POST /api/v1/workspaces/{id}/databases` |
| `add_property` | `POST /api/v1/databases/{id}/properties` |
| `create_row` | `POST /api/v1/databases/{id}/rows` |
| `update_row` | `PATCH /api/v1/rows/{id}`, `PATCH /api/v1/databases/{id}/rows/{unique_id}` |
| `claim_task` | `POST /api/v1/rows/{id}/claim`, `POST /api/v1/databases/{id}/rows/{unique_id}/claim`, with `?person_property=` |
| `neon://me/tasks` | `GET /api/v1/workspaces/{id}/tasks` |
| `resources/list` | `GET /api/v1/account/favorites`, `GET /api/v1/workspaces/{id}/recent-pages`, and the tasks |

Over REST, the [live connections](/docs/architecture/realtime/) take the place of
subscriptions, `/ws/pages/{id}` for a page's document and `/ws/databases/{id}` for a
database. For `neon://me/tasks`, an [outgoing webhook](/docs/guides/webhooks/#outgoing-webhooks)
registered for the identity takes its place: `row.assigned` and `row.status_changed` tell a
program that runs elsewhere of its work as it comes, without a socket held open. A claim
refused over REST answers `409 already_claimed` with the same `holders` beside its code and
message, each with `id` and `name`.

## Observability

Each tool call runs in a span named `mcp <tool>`, and the RED metrics of the subsystem `mcp`
count it with the tool's name as its operation. Listing, reading and subscribing to
resources count as `list_resources`, `read_resource` and `subscribe`. A refusal counts as
`ok` and a failure of the server as `error`. The gauges `neon_mcp_sessions` and
`neon_mcp_subscriptions` count what is open. No log line holds a page's content, a value or
a token. The [observability page](/docs/architecture/observability/) has the rest.
