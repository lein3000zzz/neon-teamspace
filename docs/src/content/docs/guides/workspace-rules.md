---
title: Workspace rules and API
description: The rules for workspaces, their members and roles, invitations and invite links, teamspaces and limits, the API that applies them, and what instance admins do with workspaces.
---

A workspace is a team's space on the instance. Accounts belong to the instance, so one
person can be in several workspaces, with a role in each. This page has the rules and the
API. The web app's pages for them are in [Workspaces and teamspaces](/docs/guides/workspaces/)
and the [workspace settings](/docs/workspace-settings/general/), and the instance admins'
list in [Workspaces](/docs/admin/workspaces/). Who may do what is listed in one table on the
[permissions](/docs/reference/permissions/) page.

Every route below needs a browser session or an API token, with the `read` scope for `GET`
and `write` for the rest. A token bound to one workspace, an agent's included, works on that
workspace's routes alone; see [Agents](/docs/guides/agents/). Deleting a workspace and transferring its ownership need a browser
session, so a token that leaked cannot take a team's work away. A workspace someone is not
in answers `404 not_found` to them, as one that does not exist would, instance admins
included.

## Creating a workspace

`POST /api/v1/workspaces` with a name makes a workspace, its default teamspace, named
General, and its creator the owner of both, in one transaction. Who may create workspaces is
the setting `workspaces.creation`: `anyone`, the default, or `admins`, for instance admins
only.

| Field | Rule |
|---|---|
| `name` | 1 to 100 characters, trimmed, no line breaks. |
| `slug` | For URLs. Made from the name unless given: its ASCII letters and digits in lowercase, dashes between words, at most 40 characters. When another workspace has it, a dash and six random characters follow. A slug typed by hand has 1 to 60 lowercase letters, digits and single dashes, and answers `slug_taken` when another workspace has it. Slugs that the app uses for its own paths, such as `admin` or `settings`, and slugs in the form of an id, are refused. |
| `icon` | An emoji or a short text, at most 16 characters. |
| `hue_override` | A hue from 0 to 359 to draw the workspace in, instead of the one its id hashes to. |

`GET /api/v1/workspaces` lists the caller's workspaces with their role, slug and hue, and
says in `can_create` whether the setting lets them create one.
`GET /api/v1/workspaces/{id}` adds the member count, which guests do not get, the limits
that hold, and the workspace's settings. Owners and admins change the name, slug, icon, hue and
`members_can_create_teamspaces` with `PATCH /api/v1/workspaces/{id}`; `null` removes the
icon or the pinned hue. A new name keeps the slug, so links keep working.

An owner deletes a workspace with `DELETE /api/v1/workspaces/{id}`, typing its name as
`confirm_name` in the body. A name that does not match answers `confirmation_mismatch`.
Everything in the workspace goes with it: members, teamspaces, invitations and links.

## Roles

| Role | What it may do |
|---|---|
| Owner | Everything, the workspace itself included: change anyone's role, make owners, transfer ownership, delete the workspace. |
| Admin | Manage members and guests, invitations, invite links and teamspaces, a private one once they join it. Never change or remove an owner, nor make one. |
| Member | Use the workspace and its teamspaces, and create teamspaces unless the workspace's `members_can_create_teamspaces` is off. |
| Guest | See only the pages shared with them; see [Pages](/docs/guides/pages/). A guest is in no teamspace, and sees no member but themselves. |

A workspace always keeps at least one owner. Demoting, removing or leaving as its last owner
answers `last_owner`. Every change to a workspace's members locks the workspace's row
first, so two owners demoting themselves at the same moment cannot both succeed.

| Route | What it does |
|---|---|
| `GET /api/v1/workspaces/{id}/members` | Lists the members, owners first. A guest gets only themselves. |
| `PATCH /api/v1/workspaces/{id}/members/{user_id}` | Changes a role. A member who becomes a guest leaves every teamspace; a guest who becomes more joins the default one. |
| `DELETE /api/v1/workspaces/{id}/members/{user_id}` | Removes a member from the workspace and its teamspaces. Nobody removes themselves: they leave. |
| `POST /api/v1/workspaces/{id}/leave` | Leaves the workspace and its teamspaces. The last owner of a teamspace who neither owns nor administers the workspace makes another owner of it first, or deletes it: else leaving answers `last_teamspace_owner`. |
| `POST /api/v1/workspaces/{id}/transfer` | Makes another member an owner, and the owner calling an admin, together. |

## Invitations

Invitations only go to accounts that already exist, and never make one. An owner or admin
invites by address with `POST /api/v1/workspaces/{id}/invitations`, with the role to join
as: `admin`, `member` or `guest`. Owners are made by transferring ownership, never invited.

- An address with no account that can sign in answers `404 no_account`. That answer tells
  whether an address has an account here, so each inviter gets 10 invitations, then one
  every 30 seconds, and the rest answer `429 rate_limited`.
- An account in the workspace already answers `already_member`, and one with a pending
  invitation `already_invited`: an account has one pending invitation per workspace.
- An invitation waits 14 days for its answer. Once it expired, a new one can be sent.

The invited account lists what waits for it with `GET /api/v1/account/invitations`, and
answers with `POST /api/v1/account/invitations/{id}/accept` or `.../decline`. Nothing is
joined until it accepts: then it joins with the invitation's role, and the default teamspace
unless as a guest. Owners and admins list the pending invitations with
`GET /api/v1/workspaces/{id}/invitations` and revoke one with
`POST /api/v1/workspaces/{id}/invitations/{invitation_id}/revoke`. An account that joins
another way, by an invite link, loses its pending invitation then: once it leaves or is
removed, an old invitation cannot bring it back.

## Invite links

An invite link admits whoever opens it while signed in. Owners and admins make one with
`POST /api/v1/workspaces/{id}/invite-links`:

| Field | Rule |
|---|---|
| `role` | `member` or `guest`. |
| `max_uses` | 1 to 1000 people; 10 unless set. |
| `expires_at` | After now and within a year; a week from now unless set. |

The answer carries the link's token, `wsl_` and 43 more characters, 256 random bits. The
database keeps its SHA-256 and, with `secrets.keys` set, a sealed copy, so owners and admins
copy it again from `GET /api/v1/workspaces/{id}/invite-links/{link_id}/token`. Without a
key, the token shows once. Lists never carry tokens.

Before joining, `POST /api/v1/workspace-invite-links/preview` with the token, signed in,
answers the workspace the link admits to, by name, icon and hue, the role it admits with, its
end, and the caller's role there when they are in it already; it takes no use. Joining is
`POST /api/v1/workspace-invite-links/join` with the token, signed in: using the
link is accepting it. The use is taken in the transaction that adds the member, so two
people racing for a link's last use cannot both have it, and someone in the workspace
already takes no use and gets `already_member`. A link used up, expired or revoked answers
`invalid_invite_link`, to a preview too. Previews and joins share one allowance per account,
10 and then one every 30 seconds, past which they answer `429 rate_limited`. Revoking one with `.../invite-links/{link_id}/revoke` stops it at
once; it stays listed with its uses.

Thirty days after an invitation was answered, revoked or expired, or a link expired or was
revoked, the hourly cleanup removes it.

## Teamspaces

Every workspace has exactly one default teamspace, which everyone but guests is in. It
cannot be deleted, left, or given another access, and members join it as they join the
workspace. Other teamspaces have one of three kinds of access:

| Access | Who sees it | Who joins it |
|---|---|---|
| Open | Every member but guests. | Any of them, at will, and leaves at will. |
| Closed | Every member but guests. | Those its owners, or the workspace's owners and admins, add. |
| Private | Its own members only. The workspace's owners and admins see its name, to manage or delete it. | Those its owners, or the workspace's owners and admins, add. |

`GET /api/v1/workspaces/{id}/teamspaces` lists the teamspaces the caller sees, with their
role in each, the default one first. For owners and admins, `hidden` lists the private
teamspaces they are not in, by id and name only. A private teamspace answers `not_found` to
everyone outside it, the workspace's owners and admins included, whatever they ask of it,
but for two things they may do: delete it, or join it to manage it, which its members then
see in its member list.

A teamspace's roles are owner and member. Its creator is its owner. Owners, and the
workspace's owners and admins, change it, add and remove members and change their roles.
A teamspace keeps at least one owner while its own owners manage it: its last owner can
neither leave it, nor leave the workspace, nor be demoted, which answers
`last_teamspace_owner`, except by the workspace's owners and admins, who manage it then.

| Route | What it does |
|---|---|
| `POST /api/v1/workspaces/{id}/teamspaces` | Creates an open, closed or private teamspace. |
| `GET`, `PATCH`, `DELETE /api/v1/workspaces/{id}/teamspaces/{teamspace_id}` | Reads, changes or deletes one. |
| `POST .../teamspaces/{teamspace_id}/join`, `.../leave` | Joins or leaves one. |
| `GET`, `POST .../teamspaces/{teamspace_id}/members` | Lists members, or adds one. Only members of the workspace who are not guests can be added. |
| `PATCH`, `DELETE .../teamspaces/{teamspace_id}/members/{user_id}` | Changes a member's role, or removes them. |

## Hues

Users, workspaces and teamspaces have a `hue`, 0 to 359, that the web app draws them in:
the hue their id hashes to, as [the web app](/docs/architecture/web/#hues-of-things)
describes, unless they pin one, `hue_override`. A user pins theirs with
`PATCH /api/v1/account/me`.

## Limits

A workspace has three limits: the people in it, guests included; the bytes its attachments
take; and its agents. Each instance-wide default is a setting, `workspaces.limits.members`,
`workspaces.limits.storage_bytes` and `workspaces.limits.agents`, and 0, their default, is no
limit. Instance admins can set any of them for one workspace, which then wins over the
default.

The member limit holds. A workspace at it refuses new invitations and invite links, and
accepting an invitation or joining by link answers `member_limit`; the invitation stays, to
accept once there is room. The agent limit holds too: agents and integrations count against
it together, deleted ones left out, and a workspace at it makes no more agents or incoming
webhooks, answering `409 agent_limit`. Nobody in the workspace is ever removed by a limit,
however low it is set. The storage limit holds too: a workspace at it refuses uploads and
imports that bring files, answering `409 storage_limit`, and its pages stay editable; see
[Attachments](/docs/guides/attachments/#limits).

## For instance admins

Instance admins manage workspaces without entering them. The admin routes show a
workspace's name, slug, owners with their names and addresses, member count, creation time,
limits and usage, and nothing of what is in it. In a workspace they are not in, an instance
admin is anyone outside: its routes answer them `not_found`.

| Route | What it does |
|---|---|
| `GET /api/v1/admin/workspaces` | Lists every workspace, in pages of up to 200, in the order they were made. |
| `PATCH /api/v1/admin/workspaces/{id}/limits` | Sets a workspace's limits. Each field takes a number, 0 for no limit, or `null` to follow the instance's default again; a field left out stays. |
| `DELETE /api/v1/admin/workspaces/{id}` | Deletes a workspace with everything in it, from a browser session. |

These need an instance admin, signed in at a browser or with a token that has the `admin`
scope.
