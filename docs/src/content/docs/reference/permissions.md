---
title: Permissions
description: Who may do what in a workspace and its teamspaces, and what instance admins may do with workspaces.
---

These tables are the rules the server enforces. They live in one function in `neon-domain`,
which every workspace use case calls before it acts, and its tests check every role against
every action and kind of resource. [Workspaces and teamspaces](/docs/guides/workspaces/)
explains each action.

**Yes** is allowed and **no** refused with `403 forbidden`. **Hidden** answers
`404 not_found`, as for something that does not exist, so the caller learns nothing of it.
Someone outside a workspace gets **hidden** for everything in it, instance admins included:
being one gives no access to any workspace.

## Workspaces

| Action | Owner | Admin | Member | Guest |
|---|---|---|---|---|
| See the workspace | yes | yes | yes | yes |
| Change its name, slug, icon, hue and settings | yes | yes | no | no |
| Delete it, typing its name, at a browser | yes | no | no | no |
| List its members | yes | yes | yes | only themselves |
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
| Change an owner's role | yes | no | no | no |
| Change an admin's, member's or guest's role to admin, member or guest | yes | yes | no | no |
| Make anyone an owner | yes | no | no | no |
| Change their own role, to anything but owner | yes | yes | no | no |
| Remove an owner | yes | no | no | no |
| Remove an admin, member or guest | yes | yes | no | no |
| Remove themselves | no: they leave | no: they leave | no: they leave | no: they leave |
| Transfer ownership to someone not an owner yet, at a browser | yes | no | no | no |

Whatever the table allows, a workspace keeps one owner: a change that would leave it none
answers `409 last_owner`. Guests may not know who else is in the workspace, so to them every
other member is **hidden**.

## Teamspaces

Guests see no teamspace: every action on one is **hidden** to them. The columns here are
the workspace roles that may join teamspaces; *teamspace owner* means an owner of that
teamspace.

| Action | Workspace owner or admin | Teamspace owner | Teamspace member | Member outside it |
|---|---|---|---|---|
| See an open, closed or default teamspace, and list its members | yes | yes | yes | yes |
| See a private teamspace in full | only if in it | yes | yes | hidden |
| See that a private teamspace exists, by name | yes | yes | yes | hidden |
| List a private teamspace's members | yes | yes | yes | hidden |
| Join an open teamspace | yes | in it already | in it already | yes |
| Join a closed teamspace | yes | in it already | in it already | no: an owner adds them |
| Join a private teamspace | yes | in it already | in it already | hidden |
| Leave a teamspace other than the default | if in it | yes, unless its last owner | yes | no: not in it, or hidden if private |
| Change its name, icon and hue | yes | yes | no | no, or hidden if private |
| Change its access between open, closed and private | yes | yes | no | no, or hidden if private |
| Delete it | yes | yes | no | no, or hidden if private |
| Add and remove its members | yes | yes | no | no, or hidden if private |
| Change its members' roles | yes | yes | no | no, or hidden if private |

The default teamspace holds everyone but guests, so nobody joins, leaves, is added to or is
removed from it, and it is never deleted. Its access never changes, and no other teamspace
becomes the default one. Its name, icon and hue change, and so do its members' roles, as for
any teamspace.

A teamspace keeps one owner while its own owners manage it: a change by anyone else that
would leave it none answers `409 last_teamspace_owner`, and so does leaving the workspace as
its last owner. The workspace's owners and admins may leave it without one, as they manage
it themselves.

## The instance

| Action | Instance admin | Anyone else |
|---|---|---|
| Create a workspace | yes | yes, while `workspaces.creation` is `anyone` |
| List every workspace: names, owners, member counts, limits, usage | yes | no |
| Set a workspace's limits | yes | no |
| Delete a workspace, at a browser | yes | no |
| See or change anything in a workspace they are not in | hidden | hidden |

An instance admin is an account with the `admin` role, signed in at a browser or with a
token that has the `admin` scope; see [API tokens](/docs/guides/accounts/#api-tokens).
