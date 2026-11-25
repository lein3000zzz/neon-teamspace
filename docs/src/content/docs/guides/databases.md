---
title: Databases
description: Databases and their rows, properties and views, querying them, moving cards across a board, unique ids, following changes live, and exports, with the API for all of it.
---

A database is a page whose subpages are its **rows**. Each row is a page of its own, with a
block body edited live like any page's, and a value for each of the database's
**properties**: a title, a status, an owner, a due date. The database's **views** lay its
rows out as a table, a board of cards or a list, each with a filter, sorts and a grouping of
its own. This page covers the rules and the API; the
[databases reference](/docs/reference/databases/) lists every property type, its values, the
operators of filters and the conversions between types.

:::note[In the web app]
[Databases and boards](/docs/guides/databases-and-boards/) describes the web app's views,
editors and settings for all of this.
:::

Every route below needs a browser session or an API token, with the `read` scope for `GET`
and for queries, and `write` for the rest. A database someone may not see answers
`404 not_found`, the same as one that does not exist.

## A database and its rows

A database is a page of kind `database`, and its rows are pages of kind `row` under it.
Everything about pages holds for both: a database is in a teamspace or someone's private
pages, under a page or at the top; it and its rows are shared, favorited, trashed, restored
and exported as pages are, and a row's body is edited live at `/ws/pages/{id}` like any
page's. `GET /api/v1/pages/{id}` tells them apart by `kind`.

A few things differ:

- Rows are not in page trees: the database is, and stands for them, so a sidebar shows the
  database and its views show the rows. A database's `child_count` leaves its rows out.
- Nothing goes under a database but its rows, which are added as rows, and nothing goes
  under a row. Making, importing or moving a page there answers `400 invalid_input`.
- A row stays in its database. It moves among the rows, or between the groups of a board,
  with `POST /api/v1/rows/{id}/move`; the pages' move answers `400 invalid_input`.
- A row in the trash comes back to its database only. While the database is in the trash
  too, restoring the row alone answers `403 forbidden`: restore the database, which brings
  back the rows that went to the trash with it.

A database is a **full page**, a page of its own wherever it is, or **inline**: embedded in
the page it is under, where the page's body shows it. An inline database is made under the
page that embeds it, and moved away from that page, or restored from the trash to the top of
its space while that page is still in the trash, it becomes a full page.

### Who may do what

What someone may do with a database is what they may do with its page, and its rows take
their access from it as any subpage does: whoever sees the database sees its schema, its
views and every row; whoever edits it changes its properties and views and adds rows, and
edits every row. A share on a row adds to what that row gives, as a share does on any page,
so a row shared to edit with someone who only views the database is theirs to change.
Someone who sees a row only through its own share reads it without its `database_id`, as
its page leaves out the page above it. Placing a row among the others by hand is a change to
the database too, and needs edit access to both.
[Permissions](/docs/reference/permissions/#databases-and-rows) has the table.

Guests know no other member, so what they see of a database leaves people out: the values of
person properties, who made and last changed each row, and in views, the people a filter
names and the person groups a board places or hides by hand. They neither set nor clear a
row's person values, and do not change a person property's type, whose values would become
names: each answers `403 forbidden`. A member who changes a person property to text or to
options does write the names of those it named, as text or as options, for everyone who sees
the database to read, guests too.

## Making a database

`POST /api/v1/workspaces/{id}/databases` makes one. The body names one place for it, as for
a page: `parent_id`, `teamspace_id`, or `private: true`; `inline: true` with `parent_id`
embeds it in that page. `title`, `icon` and `placement` are as a page's.

```sh
curl -X POST "$NEON/api/v1/workspaces/$WORKSPACE/databases" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"teamspace_id": "'"$TEAMSPACE"'", "title": "Tasks"}'
```

A new database has two properties, `Name`, its title, and `Status`, with the options Not
started, In progress and Done, and two views, `Table` and `Board`, a board grouped by
status. The answer, and `GET /api/v1/databases/{id}`, hold its page, its properties and its
views, each with its id.

## Properties

A database has exactly one **title** property, made with it, whose value is each row's page
title, and up to 100 properties in all. Besides the title, a property holds text, numbers,
options of a select, a multi-select or a status, dates, people, a checkbox or a URL, or is
**computed** from the row: its unique id, and when and by whom it was made and last changed.
The [reference](/docs/reference/databases/#property-types) has each type, its settings and
its values.

| Route | What it does |
|---|---|
| `POST /databases/{id}/properties` | Adds a property: `name`, `type`, the type's settings, its `options`, and `placement` among the others. |
| `PATCH /databases/{id}/properties/{property_id}` | Renames it, changes its type or settings, or its options. |
| `POST /databases/{id}/properties/{property_id}/move` | Places it elsewhere among the properties. |
| `DELETE /databases/{id}/properties/{property_id}` | Deletes it, any but the title, with its values on every row. |

Names are unique in a database, ignoring case: another property's name answers
`409 name_taken`. There is at most one unique id property, and the title keeps its type and
stays.

**Changing a type** converts each row's value, as the
[conversions](/docs/reference/databases/#changing-a-type) say: numbers become text, texts
become numbers when they read as one, options carry over between selects and statuses. A
change that would lose values, such as text becoming numbers where some do not read as one,
answers `409 lossy_change`, saying how many rows would lose theirs, until it is sent again
with `"confirm": true`. One that would make a row's values larger than they may be answers
`409 limit_reached`, and changes nothing. Settings come with their type: to change only a number's format,
name its type too, `{"type": "number", "number_format": "percent"}`.

**Options** of a select, multi-select or status change all at once: `options` lists every
option in its new order. An option listed with its `id` keeps it, and its values with it,
whatever its new name or hue; one without an id is new; one left out is deleted, and cleared
from every row that had it. A status option names its group: `to_do`, `in_progress` or
`done`.

Views that name a property deleted, or one whose type no longer takes what they asked, such
as a filter of a text property that became a number, leave that out and go on with the rest.

## Rows and their values

`POST /api/v1/databases/{id}/rows` adds a row, for those who edit the database. `values`
names properties by id or by name, and options by id or name too; the title property's value
is the row's title. `group` puts the row in a group of a view, as a card added to a board's
column: `{"view_id": ..., "key": ...}`, the key being an option's id, a person's id, or `null`
for the group without a value.

```sh
curl -X POST "$NEON/api/v1/databases/$DATABASE/rows" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"values": {"Name": "Fix login", "Status": "In progress", "Points": 3}}'
```

A row answers with its `values` by property id, its `title`, its `number` and `unique_id`,
its `position` among the rows, who made and last changed it and when, and `access`, what the
caller may do with it. `GET /api/v1/rows/{id}` reads one.

`PATCH /api/v1/rows/{id}` changes values: those it names, `null` clearing one, and no others.
Values are plain data rather than a live document: of two changes to one property, the last
wins, and two people changing different properties of a row at once both keep theirs.
Changes to a row's values are checked against the schema as it is when they are saved:
while one is saved, changes to the schema wait for it, and it for them. Computed properties
take no value, and a value its property does not take answers `400 invalid_input`. A person
value names members of the workspace: someone a change adds who is not one answers
`400 invalid_input`, while people a row names already stay as they are, members or not, until
a change takes them off. A row's values take at most **64 KiB** together, as JSON: a change
that would make them larger answers `409 limit_reached`.

### Numbers and unique ids

Each row gets the database's next number as it is made: 1, 2, 3 and on. A number is never
given again, even once its row is deleted. A unique id property shows it after its prefix,
`NW-42`, or alone without one, and `GET /api/v1/databases/{id}/rows/NW-42` finds the row:
that is how people and agents name a task. The prefix may be written in any case, or left
out: `nw-42` and `42` find the same row.

`PATCH /api/v1/databases/{id}/rows/NW-42` changes a row by its unique id, as
`PATCH /api/v1/rows/{id}` does.

### Claiming rows

A row is a task, and its assignees are whom its person properties name.
`POST /api/v1/rows/{id}/claim`, or `POST /api/v1/databases/{id}/rows/NW-42/claim`, takes a
row nobody holds, in one transaction that holds the row. While the database's first person
property, or the one `?person_property=` names, names nobody, the claim names the caller
there. It also moves the row to the first option in progress of the database's first status
property. Otherwise it changes nothing and answers `409 already_claimed`, naming who holds
the row with their ids. Of claims of one row at once, exactly one wins. A database without a
person property, or without a status with an option in progress, answers
`400 invalid_input`.

`GET /api/v1/workspaces/{id}/tasks` lists the rows assigned to the caller: those of the
workspace's databases that a person property names them in, out of the trash, that they see,
the most recently changed first, at most 100, each with its database's title and its status.

### Moving rows

`POST /api/v1/rows/{id}/move` moves a row in one request: `placement` places it among the
rows, `"first"`, `"last"`, or `{"before": id}` or `{"after": id}` beside another row, and
`group` moves it to another group of a view, `{"view_id": ..., "from": ..., "to": ...}`,
which sets its value of the view's grouping property. On a board grouped by person, the
person of `from` makes room for the one of `to`, and the row's other people stay; from a
person's column to the same one, the value stays as it is. On a board of options, `from`
does not count.

The order rows are placed in by hand is their page's position among the database's rows. It
is the database's, shared by every view: a view without sorts shows the rows in it, and a
view that sorts falls back on it between rows that sort the same.

### Trash

A row goes to the trash, comes back and is deleted for good with the pages' routes:
`POST /api/v1/pages/{id}/trash`, `/restore` and `DELETE /api/v1/pages/{id}`. Queries leave out
the rows in the trash. A database in the trash shows, to those who could edit it, the rows
that went to the trash with it.

## Views

A view lays the rows out as a `table`, a `board` or a `list`, with:

| Setting | What it holds |
|---|---|
| `filter` | Conditions on the rows' values, in groups of `and` and `or`. |
| `sorts` | Up to 10 properties to sort by, each `ascending` or `descending`. |
| `group` | The select, status or person property it groups by, the groups it places by hand first, `order`, and those it hides, `hidden`. A board always groups; a table or a list may. |
| `properties` | Every property, in the view's order, each `visible` or not and with a column `width`. Those a request leaves out follow in the schema's order, shown in a table and hidden elsewhere but the title. |

| Route | What it does |
|---|---|
| `POST /databases/{id}/views` | Adds a view, up to 50. |
| `PATCH /databases/{id}/views/{view_id}` | Changes what its body names; `null` removes the filter, or a table's or list's grouping. |
| `POST /databases/{id}/views/{view_id}/move` | Places it elsewhere among the views. |
| `POST /databases/{id}/views/{view_id}/duplicate` | Copies it right after itself, named after it with ` (copy)` unless `name` says otherwise. |
| `DELETE /databases/{id}/views/{view_id}` | Deletes it; a database keeps at least one. |

Answers name properties by id; requests may name them by name. A filter, sort or grouping
that names a property the database lacks, or asks what its type does not take, answers
`400 invalid_input`.

## Querying

`POST /api/v1/databases/{id}/query` answers with the rows the caller sees, a part at a time.
With `view_id`, they come as the view has them; `filter` and `sorts` replace the view's, or,
without a view, apply to every row. It is a `POST` because filters are too large for a query
string, and it needs only the `read` scope.

```sh
curl -X POST "$NEON/api/v1/databases/$DATABASE/query" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "view_id": "'"$BOARD"'",
    "filter": {"and": [
      {"property": "Points", "operator": "greater_than", "value": 2},
      {"property": "Owner", "operator": "contains", "value": "me"}
    ]},
    "sorts": [{"property": "Points", "direction": "descending"}]
  }'
```

A view that groups answers group by group, in its order, each row with its `group`, and
lists the `groups` with how many rows passed the filter in each, the hidden ones too, whose
rows are left out. A row naming two people is in both their groups, once in each.

`limit` is 1 to 1000 rows, 100 unless set, and `next` is where the next part starts: pass it
back as `after`. A cursor is a place in the answer, which is worked out afresh for each part,
so rows that change between two parts may move between them. `utc_offset_minutes` is the
caller's offset from UTC, which days are told at: what `today` is, and which day a moment
falls on.

Filters, sorts and groups are worked out by the server's domain rules over every row of the
database at once, reading of each row only the values the filter, sorts and grouping name;
the rows of the part answered with are then read whole. That is why a database holds at most
**10,000 rows** out of the trash, and a row's values at most 64 KiB: what one query reads is
bounded. One row more answers `409 too_many_rows`, and so does restoring one into a database
that is full.

## Following changes live

`GET /ws/databases/{id}` is a WebSocket on which the server sends, as JSON text messages,
every change to the database once it is saved, to everyone who shows it: a row made,
changed, moved, trashed or restored, the schema changed, a view changed. There is no Yjs
here: values are plain data, and a client applies each message to what it shows.

```ts
const socket = new WebSocket(`${location.origin.replace(/^http/, "ws")}/ws/databases/${id}`);
socket.onmessage = (message) => {
  const event = JSON.parse(message.data);
  if (event.type === "row_updated") applyValues(event.row_id, event.values);
};
```

The upgrade is checked as a page's is: from the server's own origin when it names one,
authenticated by the session cookie or a token with the `read` scope, and the rules must let
the caller see the database, else `404`. A connection gets `ready` first, once it hears every
change; nothing it missed before is sent again, so a client reads the database, and queries
it, when it connects and again whenever it reconnects. When the caller's access changes, the
server tells them with an `access` message, or closes the socket with `4404` once they lose
the database, and with `4401` once their session or token ends.
[Real-time sync](/docs/architecture/realtime/#database-feeds) has every message, the limits
and the close codes.

## Exporting

A database exports as markdown like any page, as [Import and
export](/docs/guides/import-export/#exporting) describes. Its file ends with a table of its
rows: their titles and the properties its first view shows, in that view's order, one line
for each row in the view's sort order, every row whatever its filter. Exported with the pages
under it, or with its teamspace, each row is a file of its own in the database's folder: its
title, its properties that have a value as a list of `name: value`, then its body. Rows count
apart from pages toward what one export holds: at most 5000 pages and 10,000 rows, enough
for a database at its cap.
