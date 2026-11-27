/**
 * Checks the Claude Code plugin in integrations/claude-code and the marketplace that lists it,
 * .claude-plugin/marketplace.json, against the fields the plugin reference documents, and its
 * skills against the server: every tool a skill names must be one of the MCP server's tools,
 * which crates/server/src/mcp/tools.rs lists, and every resource one of its templates.
 *
 * A skill names things of the server in inline code. A bare name there, such as `claim_task`,
 * `row_id` or `already_claimed`, must be a tool, or a name the server's API description or MCP
 * code uses: a field, an argument, a value or a code. A tool the server lacks, such as one
 * planned but not built, is none of these, and fails. A full tool name, as `allowed-tools`
 * lists them, must name the plugin's own server and one of its tools, and `allowed-tools`
 * pre-approves only tools that read: the person approves each change to the workspace.
 */

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { workspaceVersion } from "../version.ts";

/** What the skills are checked against, read from the server's code. */
export interface Server {
  tools: Set<string>;
  /** The tools that change the workspace, which need the write scope. */
  writes: Set<string>;
  /** The resource templates, such as neon://page/{id}. */
  resources: string[];
  /** The names the API description and the MCP code use, beside the tools. */
  names: Set<string>;
}

/** What the skills may refer to of the plugin itself. */
export interface Plugin {
  name: string;
  /** The keys of its MCP servers. */
  servers: string[];
  /** Its userConfig options, and whether each is sensitive. */
  options: Map<string, { sensitive: boolean }>;
}

const IDENTIFIER = /^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/;
const KEBAB = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
/** A marketplace's or a plugin entry's name: what a plugin id may hold. */
const ID_PART = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const USER_CONFIG = /\$\{user_config\.([^}]*)\}/g;

/** The text a group of a regular expression's match matched, or "" for one it did not. */
function group(match: RegExpMatchArray, index: number): string {
  return match[index] ?? "";
}

/** A tool of the MCP server: its name, and whether it changes the workspace. */
export interface Tool {
  name: string;
  writes: boolean;
}

/** The MCP server's tools, in the order of its TOOLS table. */
export function readTools(toolsSource: string): Tool[] {
  const table = /const TOOLS: \[Spec; (\d+)\] = \[([\s\S]*?)\n\];/.exec(toolsSource);
  if (!table) {
    throw new Error("tools.rs has no `const TOOLS: [Spec; N] = [...];` table");
  }
  const tools = group(table, 2)
    .split(/^\s+Spec \{$/m)
    .slice(1)
    .map((spec): Tool => {
      const name = /^\s+name: "([a-z_]+)",$/m.exec(spec);
      const writes = /^\s+writes: (true|false),$/m.exec(spec);
      if (!name || !writes) {
        throw new Error(`a Spec of TOOLS lacks its name or its writes: ${spec.trim()}`);
      }
      return { name: group(name, 1), writes: group(writes, 1) === "true" };
    });
  if (tools.length !== Number(table[1])) {
    throw new Error(`TOOLS holds ${table[1]} tools, but ${tools.length} were read`);
  }
  return tools;
}

/** The URI templates of the MCP server's resources. */
export function resourceTemplates(resourcesSource: string): string[] {
  return [...resourcesSource.matchAll(/ResourceTemplate::new\("(neon:\/\/[^"]+)"/g)].map((match) =>
    group(match, 1),
  );
}

/**
 * The names an API description uses: its properties, parameters and enum values, and the
 * names its descriptions write in code, such as error codes. Operation ids are left out, as
 * they name the server's handlers, which are no tools.
 */
export function apiNames(openapi: unknown): Set<string> {
  const names = new Set<string>();
  const add = (name: unknown) => {
    if (typeof name === "string" && IDENTIFIER.test(name)) {
      names.add(name);
    }
  };
  const walk = (node: unknown, key?: string): void => {
    if (Array.isArray(node)) {
      for (const item of node) {
        if (key === "enum") {
          add(item);
        }
        walk(item);
      }
      return;
    }
    if (node === null || typeof node !== "object") {
      return;
    }
    for (const [name, value] of Object.entries(node)) {
      if (key === "properties") {
        add(name);
      }
      if (name === "name") {
        add(value);
      }
      if (name === "description" && typeof value === "string") {
        for (const match of value.matchAll(/`([^`]+)`/g)) {
          add(match[1]);
        }
      }
      if (name !== "operationId") {
        walk(value, name);
      }
    }
  };
  walk(openapi);
  return names;
}

/**
 * The names a Rust file of the MCP server uses, its test module left out: its string
 * literals, such as error codes and the keys of what tools answer, and its structs' fields,
 * such as the tools' arguments. Only a `#[cfg(test)]` module ends the code read; a single
 * test-only item, such as a helper method, may sit above code that still counts.
 */
export function codeNames(rustSource: string): Set<string> {
  const code = rustSource.split(/#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s/, 1)[0] ?? "";
  const names = new Set<string>();
  for (const match of code.matchAll(/"([^"\\\n]*)"/g)) {
    if (IDENTIFIER.test(group(match, 1))) {
      names.add(group(match, 1));
    }
  }
  for (const struct of code.matchAll(/struct \w+(?:<[^>]*>)? \{([^}]*)\}/g)) {
    for (const field of group(struct, 1).matchAll(
      /^\s*(?:pub(?:\([^)]*\))? )?([a-z][a-z0-9_]*):/gm,
    )) {
      names.add(group(field, 1));
    }
  }
  return names;
}

/** Reads the tools, resources and names of the server in the repository at `root`. */
export function readServer(root: string): Server {
  const mcp = join(root, "crates/server/src/mcp");
  const names = apiNames(JSON.parse(readFileSync(join(root, "api/openapi.json"), "utf8")));
  for (const file of readdirSync(mcp).filter((entry) => entry.endsWith(".rs"))) {
    for (const name of codeNames(readFileSync(join(mcp, file), "utf8"))) {
      names.add(name);
    }
  }
  const tools = readTools(readFileSync(join(mcp, "tools.rs"), "utf8"));
  return {
    tools: new Set(tools.map((tool) => tool.name)),
    writes: new Set(tools.filter((tool) => tool.writes).map((tool) => tool.name)),
    resources: resourceTemplates(readFileSync(join(mcp, "resources.rs"), "utf8")),
    names,
  };
}

/** The frontmatter fields of a skill, as the skills reference lists them. */
const SKILL_FIELDS = new Set([
  "name",
  "description",
  "when_to_use",
  "argument-hint",
  "arguments",
  "disable-model-invocation",
  "user-invocable",
  "allowed-tools",
  "disallowed-tools",
  "model",
  "effort",
  "context",
  "agent",
  "background",
  "hooks",
  "paths",
  "shell",
  "metadata",
  "license",
  "compatibility",
]);

/** How long a skill's description and when_to_use may be together before the listing cuts. */
const DESCRIPTION_LIMIT = 1536;

/** A part of a tool's callable name, with what it may not hold replaced by `_`. */
function callablePart(part: string): string {
  return part.replace(/[^A-Za-z0-9_-]/g, "_");
}

/** A plugin's callable name for a tool of one of its MCP servers. */
function callableName(plugin: string, server: string, tool: string): string {
  return `mcp__plugin_${callablePart(plugin)}_${callablePart(server)}__${tool}`;
}

function matchesTemplate(uri: string, template: string): boolean {
  const pattern = template
    .split(/\{[^}]+\}/)
    .map((part) => part.replace(/[.*+?^$()|[\]\\]/g, "\\$&"))
    .join("[^/]+");
  return new RegExp(`^${pattern}$`).test(uri);
}

/** What is wrong with a tool's full name, as `allowed-tools` lists it, if anything. */
function callableProblem(span: string, plugin: Plugin, server: Server): string | undefined {
  const known = plugin.servers.flatMap((key) =>
    [...server.tools].map((tool) => callableName(plugin.name, key, tool)),
  );
  if (known.includes(span)) {
    return undefined;
  }
  const shapes = plugin.servers.map((key) => callableName(plugin.name, key, "<tool>"));
  return `${span} is no tool of the plugin's MCP server, ${shapes.join(" or ")}`;
}

/**
 * What is wrong with one span of inline code in a skill, if anything: a tool's full name, a
 * resource, or names of the server, such as `claim_task` or `identity.id`. `called` says the
 * skill calls it, so it must be a tool. Anything else, such as a command, passes.
 */
function spanProblem(
  span: string,
  called: boolean,
  plugin: Plugin,
  server: Server,
): string | undefined {
  if (span.startsWith("mcp__")) {
    return callableProblem(span, plugin, server);
  }
  if (span.startsWith("neon://")) {
    return server.resources.some((template) => matchesTemplate(span, template))
      ? undefined
      : `${span} is no resource of the MCP server, whose templates are ${server.resources.join(", ")}`;
  }
  if (called) {
    return server.tools.has(span)
      ? undefined
      : `calls \`${span}\`, which is no tool of the MCP server: ${[...server.tools].join(", ")}`;
  }
  const parts = span.split(".");
  if (!parts.every((part) => IDENTIFIER.test(part))) {
    return undefined;
  }
  const known = (part: string) =>
    server.tools.has(part) || server.names.has(part) || plugin.servers.includes(part);
  const unknown = parts.filter((part) => !known(part));
  return unknown.length === 0
    ? undefined
    : `\`${span}\`: ${unknown.join(", ")} is neither a tool of the MCP server nor a name its API or MCP code uses`;
}

/** The skill's frontmatter and body, or why they cannot be read. */
function splitSkill(text: string): { fields: Record<string, unknown>; body: string } | string {
  const match = /^---\n([\s\S]*?)\n---\n([\s\S]*)$/.exec(text);
  if (!match) {
    return "has no frontmatter between --- lines at its start";
  }
  let fields: unknown;
  try {
    fields = Bun.YAML.parse(group(match, 1));
  } catch (error) {
    return `has frontmatter that is not YAML: ${String(error)}`;
  }
  if (fields === null || typeof fields !== "object" || Array.isArray(fields)) {
    return "has frontmatter that is no map of fields";
  }
  return { fields: fields as Record<string, unknown>, body: group(match, 2) };
}

/** The entries of a frontmatter list, written as a YAML list or a space- or comma-separated string. */
function listField(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.map(String);
  }
  return typeof value === "string" ? value.split(/[\s,]+/).filter(Boolean) : [];
}

/**
 * What is wrong with a skill: its frontmatter, the options it reads, and every tool,
 * resource and name of the server it writes in code. `directory` is its folder's name.
 */
export function checkSkill(
  directory: string,
  text: string,
  plugin: Plugin,
  server: Server,
): string[] {
  const skill = splitSkill(text);
  if (typeof skill === "string") {
    return [skill];
  }
  const { fields, body } = skill;
  const problems: string[] = [];
  for (const field of Object.keys(fields)) {
    if (!SKILL_FIELDS.has(field)) {
      problems.push(`has the frontmatter field ${field}, which Claude Code does not read`);
    }
  }
  const name = fields["name"] ?? directory;
  if (typeof name !== "string" || !KEBAB.test(name) || name !== directory) {
    problems.push(
      `is named ${JSON.stringify(name)}, not its folder's kebab-case name ${directory}`,
    );
  }
  const description = fields["description"];
  const whenToUse = fields["when_to_use"];
  if (typeof description !== "string" || description.trim() === "") {
    problems.push("has no description, which Claude reads to know when to use it");
  } else {
    if (/[<>]/.test(description)) {
      problems.push("has angle brackets in its description, which the skill listing escapes");
    }
    const length = description.length + (typeof whenToUse === "string" ? whenToUse.length : 0);
    if (length > DESCRIPTION_LIMIT) {
      problems.push(`has a description of ${length} characters, past ${DESCRIPTION_LIMIT}`);
    }
  }
  for (const tool of listField(fields["allowed-tools"]).filter((entry) =>
    entry.startsWith("mcp__"),
  )) {
    const problem = callableProblem(tool, plugin, server);
    if (problem) {
      problems.push(`allowed-tools: ${problem}`);
    } else if (server.writes.has(tool.slice(tool.lastIndexOf("__") + 2))) {
      problems.push(
        `allowed-tools: pre-approves ${tool}, which changes the workspace: the person approves each change`,
      );
    }
  }
  for (const match of body.matchAll(USER_CONFIG)) {
    const key = group(match, 1);
    const option = plugin.options.get(key);
    if (!option) {
      problems.push(`reads \${user_config.${key}}, which the plugin's userConfig lacks`);
    } else if (option.sensitive) {
      problems.push(`reads the sensitive option ${key}, which skills get as a placeholder`);
    }
  }
  const prose = body.replace(/^ *```[\s\S]*?^ *```$/gm, "");
  for (const match of prose.matchAll(/(\b[Cc]all )?`([^`\n]+)`/g)) {
    const problem = spanProblem(group(match, 2), match[1] !== undefined, plugin, server);
    if (problem) {
      problems.push(problem);
    }
  }
  return problems;
}

/** What is wrong with a plugin's MCP servers, as .mcp.json or plugin.json declares them. */
export function checkMcpServers(config: unknown, plugin: Plugin): string[] {
  const servers = (config as { mcpServers?: unknown } | null)?.mcpServers;
  if (servers === null || typeof servers !== "object" || Array.isArray(servers)) {
    return ["has no mcpServers map"];
  }
  const problems: string[] = [];
  for (const [key, entry] of Object.entries(servers as Record<string, unknown>)) {
    const { type, url, headers } = (entry ?? {}) as Record<string, unknown>;
    if (type !== "http" || typeof url !== "string") {
      problems.push(`${key}: is not a remote server of type http with a url`);
      continue;
    }
    for (const match of url.matchAll(USER_CONFIG)) {
      if (!plugin.options.has(group(match, 1))) {
        problems.push(
          `${key}: its url reads \${user_config.${group(match, 1)}}, which userConfig lacks`,
        );
      }
    }
    if (!URL.canParse(url.replace(USER_CONFIG, "https://neon.example.com"))) {
      problems.push(`${key}: its url ${url} is no absolute URL once its options are set`);
    }
    if (headers !== undefined) {
      problems.push(
        `${key}: sends headers, but the server signs in with OAuth, with no token in a file`,
      );
    }
  }
  return problems;
}

/** The userConfig option fields the plugin reference allows, and their types. */
const OPTION_FIELDS = new Set([
  "type",
  "title",
  "description",
  "required",
  "default",
  "options",
  "multiple",
  "sensitive",
  "min",
  "max",
]);
const OPTION_TYPES = new Set(["string", "number", "boolean", "directory", "file"]);

function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf8"));
}

function nonEmpty(value: unknown): boolean {
  return typeof value === "string" && value.trim() !== "";
}

/**
 * What is wrong with the plugin at `dir`, and what its skills may refer to. Its version must
 * be the server's: Claude Code keeps a plugin whose manifest pins a version until the version
 * changes, so each release of the server is a release of the plugin, with the skills written
 * for its tools.
 */
export function checkPlugin(dir: string, version: string): { problems: string[]; plugin: Plugin } {
  const problems: string[] = [];
  const manifest = readJson(join(dir, ".claude-plugin/plugin.json")) as Record<string, unknown>;
  const name = manifest["name"];
  if (typeof name !== "string" || !KEBAB.test(name) || /^(claude|anthropic)/.test(name)) {
    problems.push(
      `plugin.json: its name ${JSON.stringify(name)} is not a kebab-case name of its own`,
    );
  }
  if (manifest["version"] !== version) {
    problems.push(
      `plugin.json: its version ${JSON.stringify(manifest["version"])} is not the server's, ${version} in Cargo.toml`,
    );
  }
  if (!nonEmpty(manifest["description"])) {
    problems.push("plugin.json: has no description");
  }
  if (!nonEmpty((manifest["author"] as { name?: unknown } | undefined)?.name)) {
    problems.push("plugin.json: has no author with a name");
  }
  const homepage = manifest["homepage"];
  if (homepage !== undefined && (typeof homepage !== "string" || !URL.canParse(homepage))) {
    problems.push("plugin.json: its homepage is no URL");
  }
  for (const field of ["skills", "commands", "agents", "hooks", "mcpServers", "outputStyles"]) {
    for (const path of listField(manifest[field])) {
      if (!path.startsWith("./") || path.includes("..") || !existsSync(join(dir, path))) {
        problems.push(`plugin.json: ${field} names ${path}, which is no path inside the plugin`);
      }
    }
  }
  const options = new Map<string, { sensitive: boolean }>();
  for (const [key, option] of Object.entries(
    (manifest["userConfig"] ?? {}) as Record<string, Record<string, unknown>>,
  )) {
    const unknown = Object.keys(option).filter((field) => !OPTION_FIELDS.has(field));
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      problems.push(`plugin.json: the userConfig key ${key} is no identifier`);
    }
    if (unknown.length > 0) {
      problems.push(`plugin.json: userConfig.${key} has unknown fields: ${unknown.join(", ")}`);
    }
    if (!OPTION_TYPES.has(String(option["type"]))) {
      problems.push(
        `plugin.json: userConfig.${key} has no type of ${[...OPTION_TYPES].join(", ")}`,
      );
    }
    if (!nonEmpty(option["title"]) || !nonEmpty(option["description"])) {
      problems.push(`plugin.json: userConfig.${key} needs a title and a description`);
    }
    options.set(key, { sensitive: option["sensitive"] === true });
  }
  const mcpFile = join(dir, ".mcp.json");
  const mcpConfig = existsSync(mcpFile)
    ? readJson(mcpFile)
    : { mcpServers: manifest["mcpServers"] };
  const servers = (mcpConfig as { mcpServers?: unknown }).mcpServers;
  const plugin: Plugin = {
    name: String(name),
    servers: servers && typeof servers === "object" ? Object.keys(servers) : [],
    options,
  };
  problems.push(...checkMcpServers(mcpConfig, plugin).map((problem) => `.mcp.json: ${problem}`));
  return { problems, plugin };
}

/** Every problem of the marketplace at `root`, its plugins and their skills. */
export function checkRepository(root: string): string[] {
  const problems: string[] = [];
  const file = ".claude-plugin/marketplace.json";
  const marketplace = readJson(join(root, file)) as Record<string, unknown>;
  const name = marketplace["name"];
  if (typeof name !== "string" || !ID_PART.test(name) || name.includes("..")) {
    problems.push(`${file}: its name ${JSON.stringify(name)} cannot be part of a plugin id`);
  }
  if (!nonEmpty((marketplace["owner"] as { name?: unknown } | undefined)?.name)) {
    problems.push(`${file}: has no owner with a name`);
  }
  if (!nonEmpty(marketplace["description"])) {
    problems.push(`${file}: has no description`);
  }
  const entries = marketplace["plugins"];
  if (!Array.isArray(entries) || entries.length === 0) {
    return [...problems, `${file}: lists no plugins`];
  }
  const server = readServer(root);
  const version = workspaceVersion(root);
  for (const entry of entries as Record<string, unknown>[]) {
    const source = entry["source"];
    if (typeof source !== "string" || !source.startsWith("./") || source.includes("..")) {
      problems.push(`${file}: ${String(entry["name"])} has no source inside the repository`);
      continue;
    }
    const dir = join(root, source);
    if (!existsSync(join(dir, ".claude-plugin/plugin.json"))) {
      problems.push(`${file}: ${source} holds no .claude-plugin/plugin.json`);
      continue;
    }
    const at = relative(root, dir);
    const checked = checkPlugin(dir, version);
    problems.push(...checked.problems.map((problem) => `${at}/${problem}`));
    if (entry["name"] !== checked.plugin.name) {
      problems.push(
        `${file}: lists ${String(entry["name"])}, but its plugin.json names it ${checked.plugin.name}`,
      );
    }
    const skills = join(dir, "skills");
    // A skill is a folder; Claude Code passes over files beside them, such as .DS_Store.
    const folders = existsSync(skills)
      ? readdirSync(skills).filter((folder) => statSync(join(skills, folder)).isDirectory())
      : [];
    for (const skill of folders) {
      const path = join(skills, skill, "SKILL.md");
      if (!existsSync(path)) {
        problems.push(`${relative(root, dirname(path))}: has no SKILL.md`);
        continue;
      }
      for (const problem of checkSkill(skill, readFileSync(path, "utf8"), checked.plugin, server)) {
        problems.push(`${relative(root, path)}: ${problem}`);
      }
    }
  }
  return problems;
}
