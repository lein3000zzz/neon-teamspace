---
title: Teamspaces
description: Creating teamspaces with their access, changing and deleting them, managing their members and roles, and the private teamspaces owners and admins see by name.
---

**Teamspaces**, at `/w/{slug}/settings/teamspaces`, creates teamspaces and manages them.
Every member but guests gets the tab. Browsing and joining teamspaces is on their own page,
described in [Workspaces and teamspaces](/docs/guides/workspaces/#teamspaces).

## Creating one

Owners and admins create teamspaces, and members too unless **Members can create
teamspaces** is off in [General](/docs/workspace-settings/general/#workspace); then the page
says so. The form asks for a name, an optional icon, and the access, each choice said in a
sentence:

| Access | Who sees it | Who joins it |
|---|---|---|
| Open | Every member of the workspace. | Anyone of them, at will. |
| Closed | Every member of the workspace. | Those its owners, or the workspace's owners and admins, add. |
| Private | Its own members. The workspace's owners and admins see its name. | Those its owners, or the workspace's owners and admins, add. |

Its creator is its first owner.

## The list

General comes first, then the others. Each card shows the mark, the access, the account's
role in it, if any, and how many are in it, with **Open** for its own page. Those who manage
a teamspace, its owners and the workspace's owners and admins, also get:

- **Edit**: the name, icon, access and [colour](/docs/workspace-settings/general/#colour),
  sending only what changed. General keeps its access, which the form says.
- **Members**: who is in it, opened in place. A select changes each one's teamspace role,
  owner or member, and **Remove** takes them out after asking; they stay in the workspace.
  **Add a member** picks someone from the workspace who is not a guest and not in it yet.
  General takes everyone but guests, so nobody is added to it or removed from it here.
  A teamspace keeps an owner while its own owners manage it, and the card says so when a
  change would leave none.
- **Delete**, after asking, for everyone in it. General is never deleted.

Others see whom to ask, or **Join**, as on the teamspaces page.

## Private teamspaces owners and admins are not in

The workspace's owners and admins see these too, by name alone, marked as private teamspaces
they are not in. Their full view stays closed until they are in one, so the card offers:

- **Add yourself**, which joins it;
- **Members**, to see and manage who is in it;
- **Edit**, for its name and access;
- **Delete**.
