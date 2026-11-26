---
title: Page history
description: The versions a page keeps, comparing one with the page as it is, restoring one while others edit, how long versions are kept, and the API and MCP tools.
---

Every page keeps its earlier versions. You can look at one, see what changed since, and put
the page back the way it was. Rows of a database are pages too, so their bodies have a
history of their own. A database itself has no body, and no history.

## Versions

The server keeps a version of a page each time it saves the page's document as a snapshot
and the document shows something other than the version before:

- when the last person with the page open leaves it, half a minute later;
- after enough changes, about 500 pauses in typing, while people keep editing;
- when the server stops;
- when a page starts with content, imported, made from markdown or by an agent.

So a version is roughly one sitting of work, and a long sitting leaves several. Changes that
leave the page as it was, such as typing a word and deleting it, keep no version.

Each version records when its latest change was made, who changed the page since the version
before, people, agents and integrations alike, and how many blocks they added, changed and
removed, at any depth. The first version of a page counts every block as added. A page that
had content before page history existed starts its history at its next save.

The version is the page's blocks as the editor shows them, with their ids, colours and
nesting. A database shown in the page is kept as the block that names it, not as its rows.
Rows' property values, such as a status or an assignee, are not part of any version, and no
log of their changes is kept: they are saved one at a time as they change, the last change
wins, and a row shows who changed it last and when. Keeping each value's earlier states
would also have to hide the people named in person values from guests, as rows do.

## In the web app

**History** in a page's header opens its versions in a panel, the latest first. Each one
shows when, who, and how much changed. The one you pick opens on **Changes since**, every
block of the version and of the page as it is now, in the page's order:

| Mark | The block |
|---|---|
| Same | Is as the version has it. |
| Changed | Has other text, type or props now: what it had then is struck through, what it has now below. |
| Added since | Is on the page and not in the version. Restoring removes it. |
| Removed since | Was in the version and is not on the page. Restoring brings it back. |

Blocks are told apart by their ids, which a block keeps however its text changes. A block
moved under another block shows as removed where it was and added where it is. The
comparison includes what is being typed right now, saved or not.

**Preview** shows the version read-only, as the editor would draw it.

## Restoring

**Restore this version** asks first, then makes the page hold the version's blocks again.
It needs edit access to the page, out of the trash. The restore is one change to the page's
live document, made by you, so everyone who has the page open sees it at once, and it counts
as your edit.

Before it changes anything, the server saves the page as it is as a version, unless the
latest version holds it already. Restoring that version undoes the restore. The restore
itself is then kept as a version that names the one restored, **Restored from** its time.

What happens with edits made around the restore:

- **Edits since you compared.** The restore names the revision of the page the comparison
  showed you. If anyone changed the page after that, the server refuses the restore with
  `409 edit_conflict` and changes nothing. The panel then says the page changed and compares
  again, so you see their changes before you decide.
- **Blocks the version and the page share.** A block that is the same in both stays as it is,
  with its id. Text someone types in it while the restore happens stays, merged as
  concurrent typing always is.
- **Blocks the restore rewrites or removes.** A block whose content differs from the version
  is written anew, and a block the version lacks goes. Keystrokes typed into such a block
  that were still on their way to the server at that moment go with it. They are not in the
  version saved before the restore either, as the server had not received them. The editor
  shows the restored block right away, so the person typing sees the change.

## How long versions are kept

The hourly cleanup thins each page's versions, as three settings say:

| Setting | Default | |
|---|---|---|
| `history.keep_all` | 7 days | Every version younger than this stays. |
| `history.keep_daily` | 90 days | Older versions, up to this age, thin to the latest of each day. Older still, to the latest of each week, Monday to Sunday. At least `history.keep_all`. |
| `history.max_versions` | 500 | The most versions a page keeps, the latest ones. |

Days and weeks are UTC's. The latest version always stays. A change to the settings applies
to every page at the next cleanup. A page's versions go when the page is deleted for good.
They keep a version whole for an hour to ten years, and 1 to 100,000 versions a page; the
[settings reference](/docs/reference/settings/) lists their environment variables.

## Who sees what

Whoever may see a page sees its history, the versions and the comparisons, and whoever may
edit it restores a version. A page in the trash shows its history to those who could edit
it, and nobody restores a version of it until it is restored itself. Guests, and agents of
guests, know no other member, so versions show them no authors. An agent or integration
sees the history of a page it and its owner both see.

## The API

| Route | What it does |
|---|---|
| `GET /api/v1/pages/{id}/versions` | The versions, the latest first, `limit` (1 to 100, 100 when absent) at a time, the next part after `after`, which an answer's `next` gives. |
| `GET /api/v1/pages/{id}/versions/{version_id}` | A version, with its `blocks` in BlockNote's JSON and its `markdown` with block ids. |
| `GET /api/v1/pages/{id}/versions/{version_id}/compare` | What changed from the version to the page as it is, block by block, and the page's `revision`. |
| `POST /api/v1/pages/{id}/versions/{version_id}/restore` | Restores the version. The body names `base`, the revision the comparison answered. |

```json
{
  "changes": [
    { "change": "same", "depth": 0, "block_id": "b1", "after": "# Plan\n" },
    { "change": "changed", "depth": 0, "block_id": "b2", "before": "Ship in May\n", "after": "Ship in June\n" },
    { "change": "added", "depth": 1, "block_id": "b3", "after": "Hire a designer\n" }
  ],
  "added": 1,
  "changed": 1,
  "removed": 0,
  "revision": "AAAAAAAAAAAAAAAAAAAAAA"
}
```

Each block of a comparison comes as markdown, alone, without the blocks under it, which
follow it a level deeper. A version of another page, or of none, answers `404 not_found`, and
a database `400 invalid_input`. A restore answers as an edit of the page's content does:
`403 forbidden` to those who view the page, `409 edit_conflict` when the page changed since
`base`, and `429 rate_limited` past the edits a user may make.

[Agents over MCP](/docs/guides/agents-over-mcp/#pages) list a page's versions with
`list_versions` and read one with `read_page` and its `version_id`.
