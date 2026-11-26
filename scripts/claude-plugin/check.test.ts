import { describe, expect, it, onTestFinished } from "bun:test";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fixture } from "../licenses/fixture.ts";
import {
  apiNames,
  checkMcpServers,
  checkPlugin,
  checkRepository,
  checkSkill,
  codeNames,
  type Plugin,
  readServer,
  resourceTemplates,
  type Server,
  serverVersion,
  readTools,
} from "./check.ts";

const repoRoot = resolve(import.meta.dir, "..", "..");

describe("the Claude Code plugin", () => {
  it("has a marketplace, a manifest, an MCP server and skills that pass every check", () => {
    expect(checkRepository(repoRoot)).toEqual([]);
  });
});

describe("readTools", () => {
  it("reads every tool of the MCP server's table, and whether it writes", () => {
    const tools = readTools(readFileSync(join(repoRoot, "crates/server/src/mcp/tools.rs"), "utf8"));

    expect(tools).toContainEqual({ name: "whoami", writes: false });
    expect(tools).toContainEqual({ name: "claim_task", writes: true });
    expect(new Set(tools.map((tool) => tool.name)).size).toBe(tools.length);
  });

  it("fails when it reads fewer tools than the table holds", () => {
    const source =
      'pub(crate) const TOOLS: [Spec; 2] = [\n    Spec {\n        name: "whoami",\n        writes: false,\n    },\n];';

    expect(() => readTools(source)).toThrow("TOOLS holds 2 tools, but 1 were read");
  });

  it("fails a tool whose writes it cannot read", () => {
    const source =
      'pub(crate) const TOOLS: [Spec; 1] = [\n    Spec {\n        name: "whoami",\n    },\n];';

    expect(() => readTools(source)).toThrow("a Spec of TOOLS lacks its name or its writes");
  });
});

describe("the server's names", () => {
  it("reads resource templates, and the names of the API and the MCP code", () => {
    const server = readServer(repoRoot);

    expect(server.resources).toContain("neon://me/tasks");
    expect(server.names).toContain("already_claimed");
    expect(server.names).toContain("person_property");
  });

  it("reads properties, enum values and names in code, but not operation ids", () => {
    const names = apiNames({
      paths: {
        "/rows": { post: { operationId: "update_page", description: "Answers `not_found`." } },
      },
      components: { schemas: { Row: { properties: { unique_id: {} }, enum: ["to_do"] } } },
    });

    expect([...names].toSorted()).toEqual(["not_found", "to_do", "unique_id"]);
  });

  it("reads literals and fields from code but not its tests", () => {
    const names = codeNames(
      'struct Args {\n    pub(crate) row_id: Uuid,\n}\nconst CODE: &str = "holders";\n#[cfg(test)]\nmod tests { const T: &str = "search"; }',
    );

    expect([...names].toSorted()).toEqual(["holders", "row_id"]);
  });

  it("reads code below a test-only item, stopping only at the test module", () => {
    const names = codeNames(
      '#[cfg(test)]\npub(crate) fn in_use() -> usize { 0 }\nconst CODE: &str = "rate_limited";\n#[cfg(test)]\nmod tests { const T: &str = "search"; }',
    );

    expect([...names].toSorted()).toEqual(["rate_limited"]);
  });
});

const plugin: Plugin = {
  name: "neon-teamspace",
  servers: ["neon"],
  options: new Map([
    ["server_url", { sensitive: false }],
    ["token", { sensitive: true }],
  ]),
};

const server: Server = {
  tools: new Set(["whoami", "claim_task"]),
  writes: new Set(["claim_task"]),
  resources: ["neon://page/{id}", "neon://me/tasks"],
  names: new Set(["row_id", "already_claimed", "identity", "id"]),
};

function skill(body: string, frontmatter = "name: work\ndescription: Works.") {
  return `---\n${frontmatter}\n---\n\n${body}\n`;
}

describe("checkSkill", () => {
  it("passes the tools, resources and names the server has", () => {
    const body =
      "Call `whoami`, keep `identity.id`, read `neon://me/tasks` and `neon://page/0193` of " +
      "`neon`, then `claim_task` with `row_id`; `already_claimed` is normal. Run `git status`.";

    expect(checkSkill("work", skill(body), plugin, server)).toEqual([]);
  });

  it("fails a tool the server does not have", () => {
    expect(checkSkill("work", skill("Then `add_comment` on the card."), plugin, server)).toEqual([
      "`add_comment`: add_comment is neither a tool of the MCP server nor a name its API or MCP code uses",
    ]);
  });

  it("fails a call of a name that is no tool, though the server uses it", () => {
    expect(checkSkill("work", skill("Call `row_id` first."), plugin, server)).toEqual([
      "calls `row_id`, which is no tool of the MCP server: whoami, claim_task",
    ]);
  });

  it("fails a full tool name of a missing tool or of another server", () => {
    const frontmatter =
      "name: work\ndescription: Works.\nallowed-tools: mcp__plugin_neon-teamspace_neon__whoami Read mcp__plugin_neon-teamspace_neon__search";

    expect(
      checkSkill(
        "work",
        skill("Use `mcp__plugin_other_neon__whoami`.", frontmatter),
        plugin,
        server,
      ),
    ).toEqual([
      "allowed-tools: mcp__plugin_neon-teamspace_neon__search is no tool of the plugin's MCP server, mcp__plugin_neon-teamspace_neon__<tool>",
      "mcp__plugin_other_neon__whoami is no tool of the plugin's MCP server, mcp__plugin_neon-teamspace_neon__<tool>",
    ]);
  });

  it("fails a tool that writes among those allowed without asking", () => {
    const frontmatter =
      "name: work\ndescription: Works.\nallowed-tools: mcp__plugin_neon-teamspace_neon__whoami mcp__plugin_neon-teamspace_neon__claim_task";

    expect(checkSkill("work", skill("Call `whoami`.", frontmatter), plugin, server)).toEqual([
      "allowed-tools: pre-approves mcp__plugin_neon-teamspace_neon__claim_task, which changes the workspace: the person approves each change",
    ]);
  });

  it("fails a resource the server does not have", () => {
    expect(checkSkill("work", skill("Read `neon://me/inbox`."), plugin, server)).toEqual([
      "neon://me/inbox is no resource of the MCP server, whose templates are neon://page/{id}, neon://me/tasks",
    ]);
  });

  it("fails options the plugin lacks or keeps secret, and frontmatter Claude Code ignores", () => {
    const text = skill(
      "At `${user_config.address}/mcp`, with `${user_config.token}`.",
      "name: other\ndescription: Works <here>.\nallowed_tools: Read",
    );

    expect(checkSkill("work", text, plugin, server)).toEqual([
      "has the frontmatter field allowed_tools, which Claude Code does not read",
      'is named "other", not its folder\'s kebab-case name work',
      "has angle brackets in its description, which the skill listing escapes",
      "reads ${user_config.address}, which the plugin's userConfig lacks",
      "reads the sensitive option token, which skills get as a placeholder",
    ]);
  });

  it("ignores names inside fenced code, indented in a list too", () => {
    const body = '```\n`add_comment`\n```\n\n- A list:\n\n  ```json\n  {"a": `add_comment`}\n  ```';

    expect(checkSkill("work", skill(body), plugin, server)).toEqual([]);
  });
});

describe("checkMcpServers", () => {
  it("passes a remote server at a configured address", () => {
    const config = { mcpServers: { neon: { type: "http", url: "${user_config.server_url}/mcp" } } };

    expect(checkMcpServers(config, plugin)).toEqual([]);
  });

  it("fails an undeclared option, a stdio server and a header with a token", () => {
    const config = {
      mcpServers: {
        neon: { type: "http", url: "${user_config.address}/mcp" },
        local: { command: "neon" },
        token: {
          type: "http",
          url: "https://neon.example.com/mcp",
          headers: { Authorization: "x" },
        },
      },
    };

    expect(checkMcpServers(config, plugin)).toEqual([
      "neon: its url reads ${user_config.address}, which userConfig lacks",
      "local: is not a remote server of type http with a url",
      "token: sends headers, but the server signs in with OAuth, with no token in a file",
    ]);
  });
});

describe("checkRepository", () => {
  it("fails a skill that calls a missing tool or lacks its SKILL.md, past files beside them", () => {
    const root = fixture(
      {
        "Cargo.toml": '[workspace.package]\nversion = "0.1.0"\n',
        "api/openapi.json": JSON.stringify({ components: { schemas: {} } }),
        "crates/server/src/mcp/tools.rs":
          'pub(crate) const TOOLS: [Spec; 1] = [\n    Spec {\n        name: "whoami",\n        writes: false,\n    },\n];',
        "crates/server/src/mcp/resources.rs": 'ResourceTemplate::new("neon://me/tasks", "tasks")',
        ".claude-plugin/marketplace.json": JSON.stringify({
          name: "neon-teamspace",
          owner: { name: "Neon" },
          description: "Works.",
          plugins: [{ name: "neon-teamspace", source: "./plugin" }],
        }),
        "plugin/.claude-plugin/plugin.json": JSON.stringify({
          name: "neon-teamspace",
          version: "0.1.0",
          description: "Works.",
          author: { name: "Neon" },
        }),
        "plugin/.mcp.json": JSON.stringify({
          mcpServers: { neon: { type: "http", url: "https://neon.example.com/mcp" } },
        }),
        "plugin/skills/work/SKILL.md": skill("Call `whoami`, then call `search`."),
        "plugin/skills/empty/README.md": "No skill here.",
        "plugin/skills/.DS_Store": "",
      },
      onTestFinished,
    );

    expect(checkRepository(root).toSorted()).toEqual([
      "plugin/skills/empty: has no SKILL.md",
      "plugin/skills/work/SKILL.md: calls `search`, which is no tool of the MCP server: whoami",
    ]);
  });
});

describe("checkPlugin", () => {
  it("fails a version other than the server's, an unknown option field and a missing path", () => {
    const dir = fixture(
      {
        ".claude-plugin/plugin.json": JSON.stringify({
          name: "neon-teamspace",
          version: "0.0.9",
          description: "Works.",
          author: { name: "Neon" },
          skills: ["./more-skills/"],
          userConfig: {
            server_url: { type: "string", title: "Address", description: "Yours.", pattern: "x" },
          },
        }),
        ".mcp.json": JSON.stringify({
          mcpServers: { neon: { type: "http", url: "${user_config.server_url}/mcp" } },
        }),
      },
      onTestFinished,
    );

    const checked = checkPlugin(dir, "0.1.0");

    expect(checked.problems).toEqual([
      `plugin.json: its version "0.0.9" is not the server's, 0.1.0 in Cargo.toml`,
      "plugin.json: skills names ./more-skills/, which is no path inside the plugin",
      "plugin.json: userConfig.server_url has unknown fields: pattern",
    ]);
    expect(checked.plugin.servers).toEqual(["neon"]);
  });

  it("reads the server's version from the workspace", () => {
    expect(serverVersion(repoRoot)).toMatch(/^\d+\.\d+\.\d+/);
  });
});

describe("resourceTemplates", () => {
  it("reads each template's URI", () => {
    const source =
      'ResourceTemplate::new("neon://page/{id}", "page")\nResourceTemplate::new("neon://me/tasks", "tasks")';

    expect(resourceTemplates(source)).toEqual(["neon://page/{id}", "neon://me/tasks"]);
  });
});
