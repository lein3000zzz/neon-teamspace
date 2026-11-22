---
title: Permissions
description: Who may do what in a workspace, its teamspaces, its pages and databases, and what instance admins may do with workspaces.
---

These tables are the rules the server enforces. They live in one function in `neon-domain`,
which every workspace and page use case calls before it acts, and its tests check every
role against every action and kind of resource. [Workspace rules and
API](/docs/guides/workspace-rules/) and [Pages](/docs/guides/pages/) explain each action,
and the web app shows or offers only what a role allows.

**Yes** is allowed and **no** refused with `403 forbidden`. **Hidden** answers
`404 not_found`, as for something that does not exist, so the caller learns nothing of it.
Someone outside a workspace gets **hidden** for everything in it, instance admins included:
being one gives no access to any workspace. What needs a browser answers `403 forbidden` to
an API token, whoever it belongs to.

## Workspaces

| Action | Owner | Admin | Member | Guest |
|---|---|---|---|---|
| See the workspace | yes | yes | yes | yes |
| Change its name, slug, icon, hue and settings | yes | yes | no | no |
| Delete it, typing its name, at a browser | yes | no | no | no |
| List its members, and see how many there are | yes | yes | yes | only themselves, and no count |
| Leave it | yes, unless the last owner | yes | yes, unless the last owner of a teamspace | yes |
| Invite as an admin, member or guest | yes | yes | no | no |
| Invite as an owner | no | no | no | no |
| List and revoke invitations | yes | yes | no | no |
| Make an invite link for members or guests | yes | yes | no | no |
| Make an invite link for admins or owners | no | no | no | no |
| List, copy and revoke invite links | yes | yes | no | no |
| Create a teamspace | yes | yes | yes, unless `members_can_create_teamspaces` is off | no |

## Members

The column is the role of whoever acts; the row, the member it is done to.

| Action | Owner | Admin | Member | Guest |
|---|---|---|---|---|
| See another member and their role | yes | yes | yes | hidden |
| Change an owner's role | yes | no | no | hidden |
| Change an admin's, member's or guest's role to admin, member or guest | yes | yes | no | hidden |
| Make another member an owner | yes | no | no | hidden |
| Change their own role | yes | to admin, member or guest | no | no |
| Remove an owner | yes | no | no | hidden |
| Remove an admin, member or guest | yes | yes | no | hidden |
| Remove themselves | no: they leave | no: they leave | no: they leave | no: they leave |
| Transfer ownership to someone not an owner yet, at a browser | yes | no | no | hidden |

Whatever the table allows, a workspace keeps one owner: a change that would leave it none
answers `409 last_owner`. Guests may not know who else is in the workspace, so to them every
other member is **hidden**.

## Teamspaces

Guests see no teamspace: every action on one is **hidden** to them, and adding one to a
teamspace answers `403 forbidden`. The columns here are the workspace roles that may join
teamspaces; *teamspace owner* means an owner of that teamspace. A workspace owner or admin
who is in a teamspace may do all that its owners may.

| Action | Workspace owner or admin outside it | Teamspace owner | Teamspace member | Member outside it |
|---|---|---|---|---|
| See an open, closed or default teamspace, and list its members | yes | yes | yes | yes |
| See a private teamspace in full, and list its members | hidden | yes | yes | hidden |
| See that a private teamspace exists, by name | yes | yes | yes | hidden |
| Join an open teamspace | yes | in it already | in it already | yes |
| Join a closed teamspace | yes | in it already | in it already | no: an owner adds them |
| Join a private teamspace | yes | in it already | in it already | hidden |
| Leave a teamspace other than the default | no: not in it, or hidden if private | yes, unless its last owner | yes | no: not in it, or hidden if private |
| Change its name, icon and hue | yes, or hidden if private | yes | no | no, or hidden if private |
| Change its access between open, closed and private | yes, or hidden if private | yes | no | no, or hidden if private |
| Delete it | yes | yes | no | no, or hidden if private |
| Add and remove its members | yes, or hidden if private | yes | no | no, or hidden if private |
| Change its members' roles | yes, or hidden if private | yes | no | no, or hidden if private |

Outside a private teamspace, the workspace's owners and admins know its name alone. They may
delete it, or join it to manage it, which its members then see in its member list.

The default teamspace holds everyone but guests, so nobody joins, leaves, is added to or is
removed from it, and it is never deleted. Its access never changes, and no other teamspace
becomes the default one. Its name, icon and hue change, and so do its members' roles, as for
any teamspace.

A teamspace keeps one owner while its own owners manage it: a change by anyone else that
would leave it none answers `409 last_teamspace_owner`, and so does leaving the workspace as
its last owner. The workspace's owners and admins may leave it without one, as they manage
it themselves.

## Pages

What someone may do with a page depends on its space and on shares. First, the space gives
them edit access, view access or nothing:

| The page, for the caller | What it gives |
|---|---|
| In a teamspace they are in | Edit |
| In a default, open or closed teamspace they are not in | View |
| In a private teamspace they are not in, the workspace's owners and admins included | Nothing: hidden |
| One of their own private pages | Edit |
| Another member's private page | Nothing: hidden |
| Anywhere, for a guest | Nothing: hidden |

Then shares add to it. A share on the page, or on any page above it, gives view or edit
access to the person it names, and the caller ends up with the most of what the space and
every one of those shares give. A share never takes access away. So a guest sees exactly
the pages shared with them and the pages under those, and a member outside a private
teamspace sees only the pages of it shared with them.

With that access:

| Action | Edit | View | Nothing |
|---|---|---|---|
| See the page, its place and the pages under it, and add it to favorites | yes | yes | hidden |
| Export it as markdown, with the pages under it | yes | yes | hidden |
| Open its document live, and follow everyone's changes and cursors | yes | yes | hidden |
| Change its document | yes, with a token only by the `write` scope | no | hidden |
| Change its title, icon, cover and style, and add or import pages under it | yes | no | hidden |
| Move it, from where it is, within its space | yes | no | hidden |
| Move it, with the pages under it, to another teamspace or to private pages | yes, from its space: in its teamspace, or its owner; not from a share | no | hidden |
| Move a page under it | yes | no | hidden |
| Trash it, with the pages under it | yes | no | hidden |
| See it in the trash, restore it, delete it for good | yes | hidden | hidden |
| List who it is shared with | yes, but guests: no | yes, but guests: no | hidden |
| Share it, change or remove a share | yes, but guests: no | no | hidden |

A page in the trash shows only to those with edit access, who may restore it or delete it for
good and do nothing else with it until it is back: changing it, adding pages under it,
moving it, trashing it again and sharing it answer `403 forbidden`, and its document opens
read-only. Restoring or deleting a
page out of the trash answers `403 forbidden` too.

Moving a page needs edit access to the page and to where it goes: the page it goes under,
or the right to add pages at the top of a space, below. Out of its space, to another
teamspace or to someone's private pages, it needs edit access from the space itself: being
in the page's teamspace, or owning the private page. A share to edit lets its holder move
the page only within its space, under pages they edit there, so a share never hands a
member's private page, or a private teamspace's page, to anyone else. Restoring a page whose
parent is in the trash needs the right to add pages at the top of its space.

What shows of a page leaves out what the caller may not know. The page it is under is left
out when they may not see that page, as for the top of the pages shared with them. A guest,
who knows no other member, is not told who made, changed, trashed or owns a page. A list of
who a page is shared with marks each share set on a page above it as inherited, and leaves
out which page that is when the caller may not see it.

### The top of a space

| Action | Teamspace member | Outside a default, open or closed teamspace | Outside a private teamspace | Guest |
|---|---|---|---|---|
| List a teamspace's pages, and export them | yes | yes | hidden | hidden |
| Add a page at its top, import pages there, or move one there | yes | no: join it first | hidden | hidden |

| Action | Owner, admin or member | Guest |
|---|---|---|
| List their own private pages | yes | no: guests have none |
| Add a private page, import private pages, or move one into their private pages | yes | no |

Nobody lists or adds to another member's private pages: they are hidden.

## Databases and rows

A database is a page, and its rows are pages under it, so the access each gives is the one
[above](#pages): from its space, and from shares on it or on a page above it. A row takes its
access from its database, as any subpage does, with what a share on the row itself adds.
[Databases](/docs/guides/databases/) explains each action.

| Action | Edit | View | Nothing |
|---|---|---|---|
| See the database's schema and views, query its rows, follow its changes live, and find a row by its unique id | yes | yes | hidden |
| Add, change, place and delete its properties, options and views | yes | no | hidden |
| Add rows | yes | no | hidden |
| Place a row among the others by hand | yes, with edit access to the row too | no | hidden |

With the access a row gives:

| Action | Edit | View | Nothing |
|---|---|---|---|
| See the row and its values | yes | yes | hidden |
| Change its values, or move it to another group of a view | yes | no | hidden |
| Trash it, restore it and delete it for good | as any page | as any page | hidden |

A database in the trash shows, with its schema, views and the rows that went there with it,
only to those who could edit it, who change nothing in it until it is back: as for any page,
that answers `403 forbidden`. A row in the trash comes back only to its database, and while
the database is in the trash too, restoring the row alone answers `403 forbidden`.

Guests see neither the people that person properties name nor who made and last changed a
row, in answers, in the live feed and in exports, and setting a person value answers them
`403 forbidden`. An API token changes a database only with the `write` scope; a query, which
is a `POST`, needs only `read`.

## The instance

| Action | Instance admin | Anyone else |
|---|---|---|
| Create a workspace | yes | yes, while `workspaces.creation` is `anyone` |
| List every workspace: names, owners, member counts, limits, usage | yes | no |
| Set a workspace's limits | yes | no |
| Delete a workspace, at a browser | yes | no |
| See or change anything in a workspace they are not in, its pages included | hidden | hidden |

An instance admin is an account with the `admin` role, signed in at a browser or with a
token that has the `admin` scope; see [API tokens](/docs/guides/accounts/#api-tokens).
