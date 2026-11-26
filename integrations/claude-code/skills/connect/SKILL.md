---
name: connect
description: Check the connection to Neon Teamspace and explain it, naming the server, the workspace, and whom Claude acts as there, the person or one of their agents, with which scopes. Use when the Neon Teamspace tools are missing or fail, or when asked whom Claude acts as in Neon Teamspace, or how to sign in or switch workspace or identity.
allowed-tools: mcp__plugin_neon-teamspace_neon__whoami
---

# Connect to Neon Teamspace

The plugin's MCP server, `neon`, is at `${user_config.server_url}/mcp`: the address the
person gave as they enabled the plugin, with `/mcp` after it. It signs in with OAuth on the
consent page of the person's own server, so no token is kept in any file.

## Check the connection

Call `whoami`, then tell the person, in a few lines:

- **Server**: `${user_config.server_url}`.
- **Workspace**: its `workspace.name`, and their role there, `workspace.role`. The connection
  works in this one workspace.
- **Identity**: with `kind` `human`, Claude acts as the person, and what it changes names
  them. With `kind` `agent`, Claude acts as one of their agents, `identity.display_name`, for
  its `owner`: its changes show as the agent's, via its owner, and it does only what both the
  agent and its owner may.
- **Scopes**: `read` reads; claiming and editing cards takes `write` too.

## When it does not work

Claude cannot sign in or change the plugin's options itself. Tell the person what to do:

- **No `whoami` tool, or the server asks to sign in**: run `/mcp`, choose the plugin's `neon`
  server and **Authenticate**. The browser opens the consent page of their server: they sign
  in, choose the workspace, whom Claude acts as, themselves or one of their agents there, and
  the scopes, and allow it.
- **The address is missing or wrong**, such as `URL is unset or invalid` in `/plugin`, a
  server that answers `404`, or one that is not theirs: run
  `/plugin configure neon-teamspace@neon-teamspace`, give the address they open Neon
  Teamspace at, such as `https://neon.example.com`, without `/mcp` and with no slash at the
  end, then run `/reload-plugins`.
- **Signing in says the server does not support dynamic client registration**: an instance
  admin turned client registration off, `oauth.registration`, and has to turn it back on.
- **Another workspace, another identity, or `write`**: in `/mcp`, choose the server, **Clear
  authentication**, which disconnects it on the server too, then **Authenticate** again and
  choose anew.
- **Signing in from a shell**, over SSH say: `claude mcp login plugin:neon-teamspace:neon`,
  with `--no-browser` to get the address to open elsewhere.
