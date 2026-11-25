---
title: General
description: A workspace's name, icon, address and colour, whether members create teamspaces, its limits, leaving it, and deleting it.
---

**Workspace settings**, in the sidebar and the switcher, opens `/w/{slug}/settings`. It has
four tabs: **General**, this page; [Members](/docs/workspace-settings/members/);
[Agents](/docs/workspace-settings/agents/); and
[Teamspaces](/docs/workspace-settings/teamspaces/). Guests see only **General**, as they see
no member but themselves and no teamspace.

## Workspace

Owners and admins change, in one form:

- **Name**, 1 to 100 characters.
- **Icon**: an emoji or up to 16 characters, shown in the workspace's mark. Blank, the mark
  shows the first letter of the name.
- **Address**, the slug after `/w/`: lowercase letters, digits and single dashes. A new name
  keeps the address, so links keep working; changing the address breaks links to the old
  one, and the page moves to the new one. An address another workspace has is refused.
- **Members can create teamspaces**: on unless turned off. Owners and admins always can;
  guests never.

Members see the same facts without the form: the name, the address, their role, how many
people are in it and when it was made. Guests see them too, but for how many people are in
it, as they see no member but themselves.

## Colour

Owners and admins choose the hue the workspace is drawn in, for everyone: in the switcher,
the sidebar and its invitations. It is also the app's accent for every member who has not
chosen one of their own under **Appearance**. **From its id**, the default, is the hue its id hashes to,
the same on every device; [the web app](/docs/architecture/web/#hues-of-things) says how.
**Pinned** opens the slider and presets of the accent picker, with a preview of the mark.
**Save colour** keeps the choice.

## Agents

Owners and admins set the workspace's [agents](/docs/guides/agents/):

- **Agents are on in this workspace**: on unless turned off. Off, nobody makes agents and
  every agent's tokens are refused, its open connections closed. The agents stay, and work
  again once this is back on.
- **Who makes agents**: every member, the default, or owners and admins.

While the instance has agents off, with the setting `mcp.identities`, the section says so,
and the switches wait for it.

## Limits

How many people are in the workspace, guests included, and its member limit, which instance
admins set; **no limit** unless they set one. Agents and integrations count against the
agent limit instead. Guests see the limit alone. A workspace at its limit takes in nobody new,
and nobody in it is removed by a limit.

## Leave workspace

Everyone may leave, after a question. Leaving takes the account out of every teamspace too,
and coming back needs a new invitation or invite link. The last owner cannot leave: the page
says to make another member an owner first, or delete the workspace. Nor can a member who
is the last owner of one of its teamspaces, until they make another owner of it or delete
it; the workspace's owners and admins may leave such a teamspace without an owner. The app then opens on
another workspace, or on the page for accounts in none.

## Danger zone

Owners only. **Delete workspace** removes it for everyone, with its teamspaces, members,
invitations and invite links, and cannot be undone. The button that deletes stays off until
the workspace's name is typed exactly. Deleting needs a browser session: an API token cannot
do it.
