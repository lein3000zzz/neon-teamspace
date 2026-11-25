---
title: Agents
description: The workspace's agents and integrations, making one, renaming and recolouring it, handing it over, deleting it, its tokens, and adding it to teamspaces.
---

**Agents**, at `/w/{slug}/settings/agents`, lists the workspace's agents and integrations,
those that work first, then the deleted ones. Everyone but guests gets the tab. What an
agent is and what it may do is in [Agents](/docs/guides/agents/).

When agents are off on the instance, or in the workspace, a notice at the top says so and
where they are turned back on.

## Make an agent

A name, 1 to 100 characters, and a hue: from its id, which the server draws as it makes the
agent, or pinned on the slider. The agent is the caller's: it acts for them, and is a member
of the workspace and of its default teamspace from then on. The form shows to whom the
workspace lets make agents, while agents are on: everyone but guests, or its owners and
admins alone, as [General](/docs/workspace-settings/general/#agents) says. A workspace at
its agent limit refuses, and the form says so.

## Each agent

Each card shows the agent's name and hue, a badge for an agent or an integration, its
owner, **Yours** for the caller's own, when it was made, and its status:

- **Active**: its tokens work.
- **Suspended**: its owner is out of the workspace, a guest there or disabled, or their
  account is gone. The workspace's owners and admins hand it to someone else or delete it.
- **Turned off**: agents are off, on the instance or in the workspace.
- **Deleted**: it is out of the workspace, and what it made still names it.

What the card offers depends on who looks:

- Its owner **renames or recolours** it: a name, and a hue from its id or pinned.
- The workspace's owners and admins **hand it over** to another person of the workspace,
  neither a guest nor disabled, after a question: it acts for them from then on, and its
  tokens are revoked.
- Its owner and the workspace's owners and admins **delete** it, after a question.
- They also **add it to a teamspace** it is not in yet: its owner to the teamspaces they
  are in, whatever their role in the workspace; owners and admins someone else's agent to
  any teamspace but General, which it is in already. It joins as a member.

## Tokens

Each agent's card lists its tokens, for its owner and the workspace's owners and admins:
name, scopes, first characters, when each was made and last used. Its owner makes them, with
a name and the scopes read and write, never admin; the new token shows once, with a button
to copy it. **Revoke**, after a question, stops a token at once. An integration has no
tokens.
