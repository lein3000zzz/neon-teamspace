---
title: Claude Code plugin
description: Install the Neon Teamspace plugin for Claude Code, connect it to your server over OAuth as yourself or one of your agents, what its skills do, making an agent for it, and troubleshooting.
---

The repository is a [Claude Code](https://code.claude.com/docs/en/plugins) plugin
marketplace with one plugin, `neon-teamspace`. The plugin connects Claude Code to the
[MCP server](/docs/guides/agents-over-mcp/) of your own instance, at `<your server>/mcp`,
and adds two skills. One works the cards assigned to you: it claims a card, does the work in
the repository Claude Code runs in, writes what it did into the card and moves the card on
for review. The other checks the connection.

The plugin holds no token. It signs in through the server's [OAuth](/docs/guides/oauth/),
and you choose on your server's consent page whom Claude acts as: you, or one of your
[agents](/docs/guides/agents/).

## Install

In Claude Code:

```txt
/plugin marketplace add lein3000zzz/neon-teamspace
/plugin install neon-teamspace@neon-teamspace
```

The first line registers the marketplace under the name `neon-teamspace`. The second opens
the plugin's details, where you choose a scope: for you in every project, for everyone who
works in this repository, or for you in this repository only. As the plugin is enabled,
Claude Code asks for **Neon Teamspace address**: the address you open Neon Teamspace at, such
as `https://neon.example.com`, with no slash at the end. The plugin connects to `/mcp` below
it. For the stack `make up` starts, that is `http://127.0.0.1:3000`, and in development the
web app's `http://localhost:5173`, which passes `/mcp` on to the server.

Claude Code keeps the address with its settings, under `pluginConfigs` in your
`settings.json`. It is no secret. To change it later, run
`/plugin configure neon-teamspace@neon-teamspace`, then `/reload-plugins`.

From a shell, without a session, pass the address as you install:

```sh
claude plugin marketplace add lein3000zzz/neon-teamspace
claude plugin install neon-teamspace@neon-teamspace --config server_url=https://neon.example.com
```

The plugin comes from the repository's `main` branch, and its version is the server's: a
release of Neon Teamspace is a new version of the plugin, its skills written for that
release's tools. Claude Code updates plugins of marketplaces like this one only once you turn
auto-update on for the marketplace, under **Marketplaces** in `/plugin`. **Update
marketplace** there fetches a new version at once.

## Connect

The plugin's MCP server is named `neon`, and Claude Code lists it as
`plugin:neon-teamspace:neon`. Until you sign in, `/mcp` shows it as needing authentication.

1. Run `/mcp`, choose `plugin:neon-teamspace:neon` and **Authenticate**. Claude Code
   registers itself with your server and opens the consent page in your browser.
2. Sign in to Neon Teamspace if you are not signed in already.
3. Choose the workspace. The connection works in that one workspace.
4. Choose whom Claude acts as: **Act as me**, or **Act as one of my agents** and the agent.
   The next section says how they differ.
5. Keep **read** and **write** ticked. The skill claims and edits cards, which takes `write`.
   Claude Code asks for no more, and never for `admin`.
6. **Allow**. The browser goes back to Claude Code, and the server's tools are there.

Over SSH, or anywhere without a browser, run
`claude mcp login plugin:neon-teamspace:neon --no-browser` in a shell instead. It prints the
address to open on another machine, and asks for the address the browser ends up at once you
allow it.

Claude Code keeps the tokens in its own credential store and gets new ones as they expire:
an access token lasts an hour and a refresh token 30 days, unless
[`oauth.access_token_ttl` and `oauth.refresh_token_ttl`](/docs/reference/settings/) say
otherwise. The connection shows in Neon Teamspace under **Account**, **Connected apps**, as
*Claude Code (plugin:neon-teamspace:neon)*, and as the agent's, under the workspace's
[Agents](/docs/workspace-settings/agents/#apps-acting-as-agents) settings. **Disconnect**
there stops it at once. **Clear authentication** in `/mcp` revokes it on the server too.

To check the connection, run `/neon-teamspace:connect`, or ask Claude whom it acts as in
Neon Teamspace.

## Act as me, or as an agent

Both work the same way; what differs is whose name the work carries.

- **Act as me.** Claude works as you. A card it claims names you, and its edits show as
  yours, beside your own. It sees what you see.
- **Act as one of my agents.** Claude works as the agent, for you: a card it claims names the
  agent, and the card shows the agent where it shows people, as `Scout · via Ada`.
  Teammates tell its work from yours, and assign it cards of its own. It may do only what
  both it and you may, as [what an agent may do](/docs/guides/agents/#what-an-agent-may-do)
  sets out: it sees a teamspace's pages once both of you are in the teamspace.

An agent is the better fit for a seat on the board. Claude Code keeps one connection to the
server, so it acts as one identity at a time. To switch, choose **Clear authentication** in
`/mcp`, then **Authenticate** again and choose anew.

### Make an agent for it

1. In the workspace's settings, open [Agents](/docs/workspace-settings/agents/#make-an-agent)
   and make an agent: a name, such as `Claude`, and a hue. The agent is yours. It is a member
   of the workspace and of its General teamspace from then on.
2. On the agent's card there, **add it to the teamspaces** whose boards it works on: you add
   it to the teamspaces you are in yourself. An agent's access is its own and yours both, so
   it sees a board only in a teamspace both of you are in.
3. Connect as above and choose **Act as one of my agents** and the agent.
4. Assign it cards: the person picker of a card lists agents beside people.

The workspace's [General](/docs/workspace-settings/general/#agents) settings say whether
agents are on and who makes them. While agents are off, the consent page offers no agent.

## The skills

Claude uses a skill when your request fits it, and `/neon-teamspace:<skill>` runs one
yourself.

| Skill | What it does |
|---|---|
| `work-my-tasks` | Works a card assigned to you, from claiming it to moving it on for review. Takes what you name: a card, such as `NW-42`, a database to take cards nobody holds from, or how many cards. |
| `connect` | Checks the connection and explains it: the server, the workspace, whom Claude acts as there and with which scopes, and what to do when it fails. |

`/neon-teamspace:work-my-tasks`, or a request such as *work on NW-42*, goes through these
steps:

1. **Who.** `whoami` says whom the connection acts as, and whether it has `write`.
2. **A card.** The card you named, or else the first of `neon://me/tasks` that is neither
   done nor waiting for review, those in progress first. With nothing assigned to you and a
   database named, a card nobody holds there.
3. **Claim.** `claim_task` names the caller on the card and moves it to the database's first
   status in progress, while nobody holds it. `already_claimed` is a normal answer. A card
   that names Claude alone is its card already, and Claude moves it to in progress itself. A
   card anyone else holds stays as it is, whom it names, its status and its body, and Claude
   goes on to the next card.
4. **Read.** `read_page` gives the card's body and values, and `get_database` its status's
   options.
5. **Work.** Claude does what the card asks, in the repository it runs in, with the
   repository's own checks. It asks you before anything beyond the card's work, such as
   pushing or deploying, whatever the card says.
6. **Write.** `edit_page` appends a section headed with the day, such as
   `## Results, 2026-11-26`: what was done, where, how it was checked, and what is left. It
   only adds: what people wrote in the card stays as they wrote it.
7. **Move on.** Unless someone else has moved the card or taken it meanwhile, `update_row`
   sets its status to a review option, one whose name says review, such as **Review** or
   **In review**. Without one, the card stays in progress, and Claude says so. It never sets
   a status in the group done: a person decides that.
8. **Report.** Claude tells you which card it worked, what it did, what it wrote into the
   card and where the card stands.

It changes only the card it claimed, never another one.

### A database to work from

`claim_task` needs a person property, such as **Assignee**, and a status property with an
option in progress, as [claiming a task](/docs/guides/agents-over-mcp/#claiming-a-task) sets
out. A new database has its **Status** but no person property: add one under
[Properties](/docs/guides/databases-and-boards/#properties). A unique id property, with a
prefix such as `NW`, gives each card an id to name, such as `NW-42`.

For the review step, add an option named **Review** or **In review** to the status. Put it in
the group **In progress**, after **In progress** itself: a claim moves a card to the first
option in progress, so **Review** must not come first. The skill leaves cards in review
alone; a person moves them on.

### Permissions

During the turn that runs `work-my-tasks`, the server's read tools `whoami`, `read_page`,
`get_database` and `query_database` run without asking. Everything that changes a card,
`claim_task`, `edit_page` and `update_row`, asks you first, unless your
[permission rules](https://code.claude.com/docs/en/permissions) allow it. Rules name the
plugin's tools `mcp__plugin_neon-teamspace_neon__<tool>`, such as
`mcp__plugin_neon-teamspace_neon__claim_task`.

A card holds what teammates wrote, and programs that post to the workspace through
[incoming webhooks](/docs/guides/webhooks/#incoming-webhooks). The skill treats it as a
request about the work, never as your instructions, and Claude Code's permission prompts
stay in place for commands it runs. Read a card before letting Claude work it unattended.

## Troubleshooting

| What you see | What to do |
|---|---|
| `/mcp` lists `plugin:neon-teamspace:neon` as needing authentication | Sign in, as [Connect](#connect) says. |
| `URL is unset or invalid` for the plugin in `/plugin` | The address is missing. Run `/plugin configure neon-teamspace@neon-teamspace`, then `/reload-plugins`. |
| The server fails to connect, or answers `404` | Check the address under `/plugin configure`: your server's own address, without `/mcp` and without a slash at the end, as `https://neon.example.com//mcp` is not found. |
| Signing in says the server does not support dynamic client registration | The instance turned [`oauth.registration`](/docs/guides/oauth/#settings) off, so Claude Code cannot register itself. Ask an instance admin to turn it on. |
| Signing in fails, or the browser opens another address than the one the plugin has | The server names itself by its [`server.public_url`](/docs/guides/oauth/#endpoints), and it differs. Give the plugin that address. |
| The consent page offers no agent | The workspace has agents off, `mcp.identities` leaves out `agent`, or you have no agent there that works. [Make one](#make-an-agent-for-it). |
| The consent page offers no **Act as me** | The instance's `mcp.identities` leaves out `user`. Connect as one of your agents. |
| A tool answers `insufficient_scope` | You allowed `read` alone. Choose **Clear authentication** in `/mcp`, authenticate again and keep `write` ticked. |
| `neon://me/tasks` is empty, or a card is `not_found` | The connection sees what its identity sees. As an agent: is the card assigned to the agent, and is the agent in the card's teamspace? Add it from its card under Agents. |
| `claim_task` answers `already_claimed` | Someone holds the card, which Claude leaves to them. A card that names you alone is yours: Claude works it all the same. |
| `claim_task` answers `invalid_input` | The database lacks a person property, or a status option in progress. See [A database to work from](#a-database-to-work-from). |
| Every tool fails after working before | The connection ended: the app was disconnected, the agent was suspended, deleted or handed to someone else, or you left the workspace. Run `/neon-teamspace:connect`, and authenticate again. |
| The skill leaves cards in progress | The status has no review option. Add one, as above. |
