---
title: Real-time sync
description: How browsers and agents edit a page together over WebSocket, who may connect, how the server keeps the documents, the limits and close codes, and what happens when someone's access changes.
---

Everyone who has a page open edits one Yjs document. The server keeps it in memory while the
page is open, merges what each connection sends, passes every change on to the others, and
saves it to Postgres. The connection speaks y-websocket's protocol, so the browser uses the
stock `WebsocketProvider` from `y-websocket`, and an agent any client of the same protocol.
What the document holds is in [Page documents](/docs/architecture/documents/).

## Connecting

A page's document is at `GET /ws/pages/{page_id}`, a WebSocket upgrade. y-websocket puts the
room name after the URL it is given, so the page's id is the room:

```ts
import * as Y from "yjs";
import { WebsocketProvider } from "y-websocket";

const doc = new Y.Doc();
const url = `${location.origin.replace(/^http/, "ws")}/ws/pages`;
const provider = new WebsocketProvider(url, pageId, doc);
// BlockNote's collaboration options take this fragment.
const fragment = doc.getXmlFragment("document-store");
```

The server checks the request before it upgrades, in this order:

| Check | Refused with |
|---|---|
| The request names an `Origin`, as browsers always do, and it is not the server's own: `server.public_url`, or the `Host` it was sent to when that is unset. | `403 forbidden` |
| It authenticates as any API request does: the session cookie, or `Authorization: Bearer nw_…` with an API token that has the `read` scope, which every scope brings. | `401 unauthenticated`, or `403 insufficient_scope` |
| The page exists, and the rules let the caller see it. | `404 not_found`, the same for a page that does not exist |
| It is a WebSocket upgrade. | `426 upgrade_required` |

The origin check stops cross-site WebSocket hijacking. The check that guards requests that
change something covers `POST`, `PATCH` and `DELETE`, and an upgrade is a `GET`; without its
own check, a page on another site could open a socket that carries the visitor's cookie. A
client that sends no `Origin` is not a browser, and has no visitor's cookie to misuse.

## The protocol

Each WebSocket message is a binary y-protocols message: sync step 1, sync step 2, an
update, an awareness update, an awareness query or an auth message. The server sends one per
WebSocket message, since y-websocket reads one from each. It reads any number from a message
a client sends, one after another. A text message closes the connection.

```mermaid
sequenceDiagram
  participant A as Client A
  participant S as Server
  participant B as Client B
  A->>S: upgrade, cookie
  S->>A: sync step 1: the server's state vector
  S->>A: awareness: everyone's cursors, if anyone is there
  A->>S: sync step 1: A's state vector
  S->>A: sync step 2: what A lacks
  A->>S: sync step 2: what the server lacks
  A->>S: update, as A types
  S->>B: update
  A->>S: awareness: A's cursor
  S->>A: awareness
  S->>B: awareness
```

Once the server applies an update, it sends what changed to every other client. It sends
each awareness change to every client, the sender too, as y-websocket's own server does: the
browser renews its awareness state every 15 seconds and counts the echo as a sign of life.
A client holds the awareness ids it introduced, at most 8, and the server drops entries it
sends for anyone else's, so nobody can move another person's cursor. When a connection
closes, its client's cursors go, and the others are told.

## Viewing and editing

Before the upgrade the server asks the page service what the caller may do with the
document. They edit it when they may edit the page, the page is out of the trash, and their
API token, if they use one, has the `write` scope. Otherwise they view it. A page in the
trash shows only to those who could edit it, and they open its document read-only.

A connection that only views gets the document and every change as it comes. It may send
updates that change nothing, as the stock provider does when it answers the server's sync
step 1 with what it holds, and it may send its awareness, so its cursor and presence show.
An update that adds anything the document lacks, or deletes anything it has not deleted,
closes the connection with `4403`.

## When access changes

The page and workspace services tell the sync service whose access may have changed, once
each change commits: a page moved, trashed, restored or deleted, a share added, changed or
removed, someone joining or leaving a teamspace or getting another role there, a
teamspace's new access or its deletion, a member's new role, leaving or removal, and a
workspace's deletion. The sync service then asks the rules again for every open connection
the change may concern, at once:

| The person may now | The connection |
|---|---|
| Not see the page | Closes with `4404`. |
| Only view it, having edited it | Stays open, read-only. The server sends an auth message, permission denied with the reason `read-only`. |
| Edit it, having only viewed it | Stays open and may edit. The server sends an auth message, permission granted. |

y-websocket logs a denied auth message and leaves granted ones alone. The web app's editor,
**planned**, is to read them to switch between editing and viewing. A connection the rules let in just before a change
committed, and that joined just after, looks again at its access as it joins, so no change
slips between the two.

## Live documents

The first connection to a page loads its document: the latest snapshot, with the updates
logged after it applied. One task owns each live document and its awareness. Every
connection hands it what its client sends, through one queue, so the document needs no lock
and applies the messages in the order they came. Answers to one client, such as sync step 2,
go to that client's queue, and what everyone must hear goes through the broadcaster, which
puts it in each client's queue.

Thirty seconds after the last connection leaves, the document is saved and unloads. A
connection that arrives meanwhile keeps it loaded, and so does a save that fails, to try
again 30 seconds later.

## Saving

An update the server applies waits a quarter of a second, or until 256 KiB of them wait, and
is then logged in `page_updates` with the others in one statement, in order, each with its
author. Consecutive updates of one author merge into one row, so typing logs a row per pause
rather than per key.

Once 500 updates, or 4 MiB of them, are logged after the latest snapshot, and whenever a
document unloads or the server stops, the whole document is saved as a snapshot in one
transaction that:

1. replaces the page's row in `page_snapshots` with the encoded document and its state
   vector, and the number of the last update it holds;
2. deletes the updates it holds from `page_updates`;
3. sets `pages.search_text` to the document's plain text, from its blocks;
4. moves `pages.content_updated_at` forward to the latest change, and names that change's
   author as `last_edited_by`, unless the page's title, icon, cover or style changed later.

Yjs updates commute, so loading applies the snapshot and the updates after it in any order
to the same document. A write that fails is logged and tried again, after half a second at
first and twice as long each time, up to 30 seconds; the document keeps the updates in
memory, in order, meanwhile. A write that finds the page deleted closes the document's
connections with `4404` and saves nothing more.

Nothing logs a document's content: log lines and spans carry the page's id, sizes and
counts.

## Limits

| Limit | Default | Past it |
|---|---|---|
| A message from a client | 4 MiB | The connection closes with `1009`. |
| A document | 16 MiB, its snapshot and the updates logged after it | An update that adds anything closes the connection with `4413`. The server first saves a snapshot, which may show the document smaller than the estimate; updates that only delete are taken. |
| Messages from one connection | 100 a second, with bursts of up to 1000 | The connection closes with `4429`. |
| Messages waiting to be sent to a client | 512 | The client fell behind, and the connection closes with `4408`, so it holds up nobody else. |
| Sending one message | 30 seconds | The connection closes with `4408`. |
| Silence | Nothing from the client, not even a pong, between two pings; the server pings every 30 seconds | The server drops the connection without a close. |
| Awareness ids per connection | 8 | Entries for more are dropped. |

The server's own pings are WebSocket pings, which browsers answer without telling the page.
y-websocket closes and reconnects a connection on which it got no message for 30 seconds; the
awareness echo keeps that from happening while the page is open.

## Close codes

| Code | Why |
|---|---|
| `1003` | A text message. The protocol is binary. |
| `1007` | A message that is not a y-sync or awareness message, or an update that does not decode. |
| `1009` | A message over the size limit. |
| `1011` | The server could not load or save the document. |
| `1012` | The server is restarting. Reconnect. |
| `4403` | A change from a connection that may only view the page. |
| `4404` | The person may no longer see the page. |
| `4408` | The client fell behind on what the server sent. |
| `4413` | The change would make the document larger than the server keeps. |
| `4429` | Too many messages. |

y-websocket reconnects after any close. A client should stop after `4403`, `4404` and
`4413`, which reconnecting does not cure, and reconnect after the rest.

## Stopping

On SIGTERM the HTTP server stops taking connections first, so clients cannot reconnect to a
server on its way out. Then every live document closes its connections with `1012` and saves
its snapshot, within 30 seconds, before the database pool closes; a snapshot that cannot be
saved leaves the updates in the log. [Observability](/docs/architecture/observability/#shutdown)
has the whole order.

## Metrics

The sync subsystem's RED metrics, `neon_sync_operations_total` and the others, time three
operations: `load`, `persist`, a batch of updates logged, and `compact`, a snapshot saved.
Beside them:

| Metric | Type | Labels | What it shows |
|---|---|---|---|
| `neon_sync_live_documents` | gauge | | Documents in memory, open or about to unload. |
| `neon_sync_open_connections` | gauge | | Open WebSocket connections. |
| `neon_sync_message_size_bytes` | histogram | `direction`: `in` or `out` | The sizes of the messages connections carry, in buckets from 64 bytes to 16 MiB. |
| `neon_sync_messages_total` | counter | `kind`: `sync_step1`, `sync_step2`, `update`, `awareness`, `awareness_query`, `auth`, `other` | Messages clients sent. |
| `neon_sync_disconnects_total` | counter | `cause`: `client`, `unresponsive`, or a close reason, such as `read_only` or `too_slow` | Connections ended. |

## One process

The broadcaster, which puts a document's messages in its clients' queues, is a port with one
adapter, for clients connected to this process. Running a second server process would need
an adapter that carries the messages between processes, and the documents themselves held
in one place; [Key decisions](/docs/architecture/decisions/#one-process-no-redis) explains why
that waits.
