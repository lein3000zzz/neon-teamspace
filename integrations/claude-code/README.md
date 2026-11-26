# Neon Teamspace for Claude Code

This plugin connects Claude Code to the MCP server of your own Neon Teamspace instance,
signing in over OAuth as you or as one of your agents, and adds two skills:

- `work-my-tasks` claims a card assigned to you, does the work in your repository, writes
  the results into the card and moves it on for review.
- `connect` checks the connection and says whom Claude acts as.

```txt
/plugin marketplace add lein3000zzz/neon-teamspace
/plugin install neon-teamspace@neon-teamspace
```

Claude Code asks for your instance's address, such as `https://neon.example.com`; `/mcp`
then signs the plugin's `neon` server in. The
[Claude Code plugin page](../../docs/src/content/docs/guides/claude-code-plugin.md) of the
docs covers connecting, choosing an identity, the skills, making an agent for them and
troubleshooting.

| File | What it holds |
|---|---|
| `.claude-plugin/plugin.json` | The manifest, and the address Claude Code asks for. |
| `.mcp.json` | The MCP server, at the address with `/mcp` after it, signed in over OAuth. |
| `skills/` | The skills. `make plugin-check` checks that they name only the server's tools, and pre-approve only those that read. |
