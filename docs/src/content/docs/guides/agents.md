---
title: Agents
description: What an agent is, who makes one, how its access is its own and its owner's both, suspension, its tokens, how it is named, integrations, and tokens bound to one workspace.
---

An agent is a member of a workspace that acts for a person there, its **owner**. It works
through API tokens its owner makes, and later over MCP too. It is how a team gives a
program a seat in a workspace, beside the people, under the same rules. Its pages in the web
app are under [Agents](/docs/workspace-settings/agents/), and who may do what is listed on
the [permissions](/docs/reference/permissions/#agents) page.

## What an agent is

An agent is an identity of one workspace: a name, a hue and an owner, a person in the same
workspace. It is a member of that workspace with the role **member**, never admin, owner or
guest, and of its default teamspace, as everyone but guests is. It has no address and no
password, never signs in, and is never invited anywhere: it belongs to the workspace it was
made in, for good.

Integrations are the same kind of member, made by incoming webhooks: an integration delivers
what its webhook's sender posts, for the person who made it. Everything on this page about
access, suspension and naming holds for integrations too. They have no API tokens, and the
switches below that turn agents off leave them alone.

## Who makes agents

The setting [`mcp.identities`](/docs/reference/settings/) says whether agents exist on the
instance at all: with `agent` among its values, the default, they do. Each workspace then
says, under its [General](/docs/workspace-settings/general/#agents) settings:

- whether its agents are on, the default, or off;
- who makes them: every member but guests, the default, or its owners and admins alone.

While agents are off, on the instance or in the workspace, nobody makes one and every
agent's tokens are refused, its open connections closed. The agents stay, and work again
once agents are back on.

Guests never make agents, and agents and integrations never make agents. A workspace has
an agent limit, which instance admins set like the member limit: agents and integrations
count against it together, deleted ones left out, and 0 is no limit. People count against
the member limit alone, and agents not at all.

## What an agent may do

An agent's access is its own and its owner's both. Every check asks the permission rules
twice, once for the agent with its own role, teamspaces and shares, and once for its owner
with theirs, on the same resource, and allows only what both allow:

- In a teamspace both are in, the agent edits its pages.
- In an open or closed teamspace the agent is in but its owner is not, it views them, as its
  owner does from outside.
- In a private teamspace, the agent sees nothing until both are in it.
- A page shared with the agent gives it no more than its owner may do with the page; a page
  shared with its owner reaches the agent only where the agent has access of its own.
- An agent has no private pages: its owner could not see them.
- What its owner may not know of stays hidden from the agent too, answering
  `404 not_found`; what its owner may not do answers `403 forbidden`.

An agent works in the workspace and never manages it. It changes no member's role, removes
nobody, invites nobody, makes no invite link, makes and changes no teamspace setting, makes
and manages no agent, and does not leave. It joins an open teamspace by itself, as a person
does, when its owner sees that teamspace in full, and leaves one at will. Others add it to
closed and private teamspaces: whoever may add members there, or its own owner, who adds it
only to teamspaces they are in themselves. An agent is a teamspace's member, never its
owner.

## Suspension

An agent is **suspended** while its owner is out of its workspace or disabled, or their
account is gone. Its tokens are refused, on every route and socket, and its open page and
database connections close. Suspension is worked out each time a token is used, from the
owner's membership and account, and never stored: there is no flag to fall out of step. So
an agent resumes by itself when its owner comes back into the workspace or is enabled again,
or when a workspace's owner or admin hands it to another person.

Its owner's access bounds the agent's at once in the same way. When its owner leaves a
teamspace, loses a share, changes role, or is removed, the agent's open connections are
looked at again with theirs, and close or turn read-only as a person's do.

## Tokens

An agent's owner makes its tokens, at a browser, with
`POST /api/v1/workspaces/{id}/agents/{agent_id}/tokens`: a name, the scopes `read`, `write`
or both, never `admin`, and an optional expiry. The answer holds the token once; the server
keeps only its hash, and lists tokens by their first characters. A token acts as the agent,
bound to the agent's workspace: on that workspace's routes and sockets alone. The request's
principal is the agent, on behalf of its owner, so whatever it makes and changes names the
agent, and every check applies both their accesses.

Its owner and the workspace's owners and admins list its tokens with
`GET …/tokens`, and revoke them with `DELETE …/tokens/{token_id}`, at a browser; a revoked
token stops at once, and the connections it opened close. Making tokens needs the agent to
work: a suspended, turned off or deleted agent gets none.

## Handing over and deleting

The workspace's owners and admins hand an agent to another person of the workspace, not a
guest, with `POST /api/v1/workspaces/{id}/agents/{agent_id}/transfer`. It acts for its new
owner from then on, which resumes it if it was suspended, and its tokens are revoked: its
former owner made them, and they would otherwise act for the new owner.

Its owner renames it and changes its hue with `PATCH /api/v1/workspaces/{id}/agents/{agent_id}`.
Its owner, or the workspace's owners and admins, delete it with `DELETE` on the same path.
A deleted agent leaves the workspace and its teamspaces, its shares and favorites go, and
its tokens are revoked; its identity stays, so what it made still names it, as deleted.

## How agents are named

Wherever people are shown, an agent shows with a small badge and the person it acts for:
`Scout · via Ada Lovelace`, and `(deleted)` once it is. That holds in the member lists,
teamspace members, shares, who made and last changed a page or a row, person values and
the board columns that group by them, filters, and exports. Every answer of the API that
carries a person carries their `kind`, `human`, `agent` or `integration`, and for an agent
or integration its `owner`; a person's `email` is absent for them.

Person properties take agents: an agent can be assigned a row, and a board groups by it.
Integrations are never assigned, as they only deliver what their webhook's sender posts;
naming one in a person value answers `400 invalid_input`.

## Tokens bound to one workspace

A person's own token, made under [API tokens](/docs/guides/accounts/#api-tokens), may be
bound to one workspace they are in. A bound token works on that workspace's routes and
sockets alone. On another workspace it answers `404 not_found`, as for one that does not
exist, and on `/api/v1/account` and `/api/v1/admin` it answers `403 token_bound`.
`GET /api/v1/workspaces` lists that workspace alone.

The `admin` scope of a bound token is for the workspace's owners and admins, and covers
that workspace's admin work only: without it, a bound token of an owner or admin manages
nothing, as a member's would not. It never makes its holder an instance admin, whatever
their account is. A token across the instance keeps working as before.

## The API

| Route | What it does |
|---|---|
| `GET /api/v1/workspaces/{id}/agents` | The agents and integrations, deleted ones too, with owner and status, for everyone but guests. |
| `POST /api/v1/workspaces/{id}/agents` | Makes an agent the caller owns. `409 agent_limit` at the limit. |
| `GET /api/v1/workspaces/{id}/agents/{agent_id}` | One of them. |
| `PATCH /api/v1/workspaces/{id}/agents/{agent_id}` | Renames it or changes its hue, for its owner. |
| `POST /api/v1/workspaces/{id}/agents/{agent_id}/transfer` | Hands it to another person, for owners and admins. |
| `DELETE /api/v1/workspaces/{id}/agents/{agent_id}` | Deletes it, for its owner and owners and admins. |
| `GET`, `POST /api/v1/workspaces/{id}/agents/{agent_id}/tokens` | Its tokens, and making one. |
| `DELETE /api/v1/workspaces/{id}/agents/{agent_id}/tokens/{token_id}` | Revokes a token. |
| `PATCH /api/v1/workspaces/{id}` | `agents_enabled` and `agent_creation`: the workspace's switches. |

An agent's status is `active`, `suspended`, `disabled`, while agents are off, or `deleted`.
The workspace's own answer says in `agents` whether the instance has them, whether the
workspace has them on, who makes them and how many there are. The use cases run in spans
named `agents <operation>` and are measured as RED metrics of the subsystem `agents`; no
token reaches a log line. The [API reference](/docs/reference/api/) has every field.
