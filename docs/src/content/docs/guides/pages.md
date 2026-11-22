---
title: Pages
description: Where pages live and who reaches them, their order, sharing, moving, the trash, favorites, and the API for all of it.
---

A page is a document of a workspace. Pages nest under other pages, as deep as 100 levels,
and each one is either in a teamspace or private to one member. This page covers the rules
and the API. Who may do what is also listed, in one table, on the
[permissions](/docs/reference/permissions/#pages) page.

:::note[What is built]
Everything here works through the API. A page's content, its blocks, is edited live over a
WebSocket, as [Real-time sync](/docs/architecture/realtime/) describes. The web app's editor
and its pages for all of this are **planned**.
:::

Every route below needs a browser session or an API token, with the `read` scope for `GET`
and `write` for the rest. A page someone may not see answers `404 not_found`, the same as a
page that does not exist, so nobody learns of a page they were not given.

## Where a page lives

A page is in one of two places, its **space**:

| Space | Who reaches its pages |
|---|---|
| A teamspace | Its members edit them. Everyone else in the workspace but guests views the pages of a default, open or closed teamspace. A private teamspace's pages are for its members alone, the workspace's owners and admins included. |
| A member's private pages | That member alone. |

A page's subpages are always in its space: under a teamspace's page go pages of that
teamspace, and under a private page the same member's private pages. Moving a page moves
its subpages with it.

Guests are in no teamspace and have no private pages. They reach a page only when it is
shared with them, which brings every page under it too. A member who becomes a guest keeps
only what is shared with them; their private pages come back if they become a member again.
A member who leaves the workspace loses their shares and favorites in it. Their private
pages stay, unseen, for when they come back, and go when their account is deleted.

## Making and changing a page

`POST /api/v1/workspaces/{id}/pages` makes a page. The body names one place for it:

| Field | Where the page goes |
|---|---|
| `parent_id` | Under that page of the workspace, which the caller edits. |
| `teamspace_id` | At the top of that teamspace, which the caller is in. |
| `private: true` | At the top of the caller's private pages. |

The rest is optional:

| Field | Rule |
|---|---|
| `title` | Up to 500 characters, trimmed, no line breaks. Empty, the page shows as Untitled. |
| `icon` | An emoji or a short text, at most 16 characters. |
| `cover` | One of the covers below. |
| `style` | `font`, `default`, `serif` or `mono`; `small_text` and `full_width`, both off unless set. |
| `placement` | Where among its siblings; see [Order](#order). Last unless set. |

A cover is drawn from the theme's tokens, at the lightness and chroma of the reader's
palette and mode, the way the accent is. `accent` is the reader's accent and
`accent-gradient` runs from it to the hue opposite. `red`, `orange`, `yellow`, `green`,
`teal`, `blue`, `violet` and `pink` are single hues. `sunrise`, `lagoon`, `aurora` and `dusk`
are gradients between two of them. Image covers come with attachments.

`GET /api/v1/pages/{id}` reads a page: its place (`teamspace_id` or `owner_id`, and
`parent_id`), its title, icon, cover and style, who made it and who changed it last, and
`access`, which is `edit` or `view`: what the caller may do with it. `PATCH` changes what its
body names, for those who edit the page; `null` removes the icon or the cover.

## Order

Siblings keep the order they are placed in. Each page has a `position`, a short key of the
characters `0-9A-Za-z` that sorts byte by byte, and the server works it out from where the
page is put among its neighbours. `placement` says where:

| `placement` | Where the page goes |
|---|---|
| `"first"` | Before every sibling. |
| `"last"` | After every sibling. |
| `{"after": id}` | Right after that sibling. |
| `{"before": id}` | Right before that sibling. |

A key is worked out from the neighbours' keys alone, with nothing random in it. Pages added
at either end keep keys of two or three characters for thousands of pages. Placing pages
again and again at one spot between two others lengthens keys; once one would pass 64
characters, the server gives that set of siblings fresh short keys first, in the same order.

## Moving

`POST /api/v1/pages/{id}/move` takes the same place fields as making a page, and a
`placement`. To reorder a page among its siblings, name the place it is in already. The
caller edits the page and the place it goes to. A page never moves under itself or one of
its subpages, which answers `409 cycle`, nor out of its workspace, nor so deep that a page
under it would pass 100 levels, which answers `409 too_deep`.

The page and the pages under it take their access from where they land. A page moved from
a teamspace into the caller's private pages leaves the teamspace's members, and one moved
into a teamspace reaches its members. Shares stay with the pages they are on.

`GET /api/v1/pages/{id}/breadcrumbs` lists the way down to a page: the pages above it the
caller sees, from the highest, then the page. For a guest it starts at the page shared with
them.

## Sharing

A page can be shared with a member or guest of its workspace, to view it (`can_view`) or to
edit it (`can_edit`). A share holds for the page and every page under it. It adds to what
the person may do there already and never takes anything away: a teamspace's member who is
given `can_view` still edits. What a person may do with a page is the most of what its
space gives them and every share on it or on a page above it.

| Route | What it does |
|---|---|
| `GET /api/v1/pages/{id}/shares` | Lists the shares on the page, then those on each page above it, marked `inherited`, each with the page it is set on. |
| `POST /api/v1/pages/{id}/shares` | Shares the page with `user_id` for `access`. A second share with the same person answers `409 already_shared`. |
| `PATCH /api/v1/pages/{id}/shares/{user_id}` | Changes what a share on this page gives. |
| `DELETE /api/v1/pages/{id}/shares/{user_id}` | Removes a share on this page. |

Those who edit a page share it. Guests neither share nor list shares, since they may not
know who else is in the workspace. An inherited share changes or goes on its own page; on a
page below, it answers `404 not_found`.

## The trash

`POST /api/v1/pages/{id}/trash` moves a page to the trash with every page under it. While it
is there, it shows only to those who could edit it, and nobody changes it, adds pages under
it or moves it. Pages in the trash drop out of every tree.

`POST /api/v1/pages/{id}/restore` brings a page back with the pages that went to the trash
with it. A subpage trashed on its own before stays in the trash; restoring it is a separate
step. The page goes back under its parent, unless the parent is in the trash too: then it
goes to the top of its space, which needs the right to add pages there. Someone without it,
such as a guest, restores the page above first. A restored page keeps its place among its
siblings while that place is free, and goes last otherwise.

`GET /api/v1/workspaces/{id}/trash` lists the pages in the trash that the caller could edit,
latest first. A page trashed with the pages under it stands for them. `DELETE
/api/v1/pages/{id}` deletes a page in the trash for good, with everything under it; a page
out of the trash answers `403 forbidden`. Once a page has been in the trash for
`pages.trash_retention`, 30 days unless set, the hourly cleanup deletes it.

## Trees

Three routes list pages as a tree, each page then the pages under it, siblings in order.
Each entry has the page's id, `parent_id`, place, title and icon, `child_count`, the number
of pages under it out of the trash, and `access`.

| Route | What it lists |
|---|---|
| `GET /api/v1/teamspaces/{id}/pages` | A teamspace's pages, for everyone who may see it in full. |
| `GET /api/v1/workspaces/{id}/private-pages` | The caller's private pages. Guests get `403 forbidden`. |
| `GET /api/v1/workspaces/{id}/shared-pages` | The pages shared with the caller, each with the pages under it, in the order they were made. A shared page under another one shared with them shows under that one only. Shared pages at the top have no `parent_id`, as the page above them stays hidden. |

A tree comes whole up to 1000 pages. A larger one comes in parts: an answer with `next` has
more, and passing that value back as `after` gets the next part. `limit` asks for smaller
parts, from 1 to 1000. The trash list takes `after` and `limit` the same way.

## Favorites

Each account keeps favorite pages in an order of its own, across its workspaces.

| Route | What it does |
|---|---|
| `GET /api/v1/account/favorites` | Lists them in order, or those of one workspace with `?workspace_id=`. |
| `POST /api/v1/account/favorites` | Adds `page_id`, a page the caller sees, out of the trash, with a `placement` among the others. |
| `POST /api/v1/account/favorites/{page_id}/move` | Moves it among the others, `placement` naming them. |
| `DELETE /api/v1/account/favorites/{page_id}` | Removes it. |

A favorite the caller can no longer see, or one in the trash, stays saved and is left out of
the list until it can be seen again.

## When access changes

Moving, trashing, restoring and deleting a page, and adding, changing or removing a share,
may change who reaches a page. So do joining and leaving a teamspace, a new role in one, a
teamspace's new access or its deletion, and a member's new role, leaving or removal. Once
such a change is saved, the server checks again, at once, what each person with those pages
open may do. It closes the page for someone who may no longer see it, and stops the editing
of someone who may now only view it, as [Real-time
sync](/docs/architecture/realtime/#when-access-changes) describes.
