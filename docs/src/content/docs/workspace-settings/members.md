---
title: Members
description: Inviting accounts by address, invite links, the invitations waiting, and the members with their roles, removing them and transferring ownership.
---

**Members**, at `/w/{slug}/settings/members`, lists everyone in the workspace, owners first,
its [agents](/docs/guides/agents/) among the members with a badge, the person each acts for,
and **Suspended** while it is. An agent's role is always member: its card offers no role,
removal or transfer, as [Agents](/docs/workspace-settings/agents/) manages it.
Owners and admins also get the three ways people come in: invitations by address, invite
links, and the invitations still waiting. Members see the list alone, and guests do not get
the tab.

## Invite by email

An address and a role: admin, member or guest, each said in a line under the choice.
Owners are made by a transfer or a role change, never invited. The invitation waits 14 days
for its answer, in the invited account's list.

The form explains each refusal beside the address:

- **No account here can sign in with this address.** Workspace invitations go to accounts on
  this instance only, and never create one: an instance admin makes the account first, or
  sends an [invite](/docs/admin/invites/) to sign up, and then it can be invited.
- The account is in the workspace already, or has an invitation waiting.
- The workspace is at its member limit.
- Too many addresses were tried: each inviter gets 10, then one every 30 seconds, as an
  answer tells whether an address has an account here. The form says how long to wait.

## Invite links

A link admits anyone with an account here who opens it while signed in. The form takes:

| Field | Rule |
|---|---|
| **Joins as** | Member or guest. |
| **People** | How many it admits, 1 to 1000; 10 unless changed. |
| **Ends** | The last day it works, at most a year ahead. Blank: a week from now. |

The new link shows at once, with a button to copy it. With
[secret keys](/docs/self-hosting/configuration/#secret-keys) set, the server keeps a sealed
copy of its token, and **Show link** brings it back later; without them, this is the only
time it shows, and the page says so.

Every link stays listed, newest first, with its state, **Open**, **Used up**, **Expired** or
**Revoked**, its role, how many joined of how many, and when it was made and ends.
**Revoke** stops an open one at once, after asking; those who joined with it stay. Its
address leaves the page then, as it admits nobody any more. What
opening a link looks like is in [Workspaces and teamspaces](/docs/guides/workspaces/#invite-links).

## Pending invitations

The invitations sent and not answered yet, each with the account invited, the role and the
end. **Revoke**, after asking, takes one back; the account can be invited again.

## The members

Each member shows their name, address, role and when they joined, and **You** marks the
account signed in. What the role in view may do:

- **Role**: a list of the roles to change to, then a question that says what the new role
  may do. Owners change anyone's role, make other owners and may step down. Admins change
  every role but an owner's, their own included, and make no owners. A workspace keeps at
  least one owner, so the last one cannot step down: the card says to make another owner
  first.
- **Transfer ownership**: owners only, to someone who is not an owner yet. They become an
  owner, and the owner transferring an admin, together. It needs a browser session.
- **Remove**: owners remove anyone, admins anyone but owners. Nobody removes themselves:
  they [leave](/docs/workspace-settings/general/#leave-workspace). A removed member leaves
  every teamspace too.

Members who are not owners or admins get no controls.
