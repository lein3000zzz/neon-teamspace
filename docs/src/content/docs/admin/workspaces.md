---
title: Workspaces
description: The instance's workspaces as admins see them, with their owners, members, limits and usage, setting limits, and deleting a workspace.
---

**Workspaces**, at `/admin/workspaces`, lists every workspace on the instance, in the order
they were made, 50 at a time with **Load more** for the rest. It shows what an instance admin
may know of a workspace and nothing more: nothing in it links anywhere, and being an
instance admin opens no workspace. An admin who opens a workspace's address without being in
it gets **Workspace not found**, as anyone outside would.

## Each workspace

- its name and address, `/w/` and its slug;
- its owners, by name and address; a workspace whose owners' accounts are gone has none;
- how many people are in it, guests included, and when it was made;
- each limit with its usage: **Members**, **Storage** and **Agents**, as used of the limit,
  and whether the limit is the workspace's own or the instance default. Members are people,
  guests included; agents are its agents and integrations together, deleted ones left out;
  storage is the bytes its [attachments](/docs/guides/attachments/) take together.

## Limits

**Edit limits** opens a form in the card with one field per limit:

- a number sets the workspace's own limit, and 0 is no limit;
- blank follows the instance default, the `workspaces.limits.*`
  [settings](/docs/reference/settings/), whose value each field names.

Storage takes bytes, MB or GB, a thousand apart. A limit below what a workspace holds removes
nobody: a workspace at its member limit takes in nobody new, by invitation or by link, until
there is room. [Workspace rules and API](/docs/guides/workspace-rules/#limits) has the rules.

## Deleting a workspace

**Delete** removes a workspace with everything in it, for every member: its teamspaces,
members, invitations and invite links. It cannot be undone, so the button that deletes stays
off until the workspace's name is typed exactly. It needs a browser session: an API token,
even one with the admin scope, cannot do it.
