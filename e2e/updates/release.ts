/**
 * The newer release make updates-e2e builds: the next patch version, stated everywhere the
 * workspace's version is, in a copy of the checkout. scripts/version.ts checks the copy
 * afterwards, so a place a later change adds and this misses fails the run.
 */
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";

/** The version the workspace's Cargo.toml gives every crate. */
export function workspaceVersion(root: string): string {
  const manifest = readFileSync(join(root, "Cargo.toml"), "utf8");
  const section = manifest.split(/^\[workspace\.package\]$/m)[1] ?? "";
  const version = /^version = "(\d+\.\d+\.\d+)"$/m.exec(section)?.[1];
  if (version === undefined) {
    throw new Error("Cargo.toml has no major.minor.patch version under [workspace.package]");
  }
  return version;
}

/** The next patch version of `version`. */
export function nextPatch(version: string): string {
  const [major, minor, patch] = version.split(".").map(Number);
  return `${major}.${minor}.${(patch ?? 0) + 1}`;
}

/**
 * The files of the checkout at `root` as git sees them, changes not yet committed included,
 * copied into `to`.
 */
export function copyCheckout(root: string, to: string) {
  const listed = spawnSync(
    "git",
    ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
    { cwd: root, encoding: "utf8" },
  );
  if (listed.status !== 0) {
    throw new Error(`git ls-files: ${listed.stderr}`);
  }
  for (const path of new Set(listed.stdout.split("\0").filter((entry) => entry !== ""))) {
    const from = join(root, path);
    if (!existsSync(from) || lstatSync(from).isDirectory()) {
      continue;
    }
    mkdirSync(dirname(join(to, path)), { recursive: true });
    copyFileSync(from, join(to, path));
  }
}

/** `text` with each match of `pattern` replaced, failing when there is none. */
function replaced(file: string, text: string, pattern: RegExp, replacement: string): string {
  const changed = text.replace(pattern, replacement);
  if (changed === text) {
    throw new Error(`${file}: found no ${String(pattern)} to bump`);
  }
  return changed;
}

function edit(root: string, file: string, change: (text: string) => string) {
  const path = join(root, file);
  writeFileSync(path, change(readFileSync(path, "utf8")));
}

/** A Cargo.lock with the entries of its own crates, which have no source, moved to `to`. */
function bumpLock(text: string, from: string, to: string): string {
  return text
    .split("\n[[package]]\n")
    .map((entry) =>
      entry.includes("\nsource = ") || !entry.startsWith("name = ")
        ? entry
        : entry.replace(`\nversion = "${from}"\n`, `\nversion = "${to}"\n`),
    )
    .join("\n[[package]]\n");
}

/**
 * Every place that states the version, in the copy at `root`, moved from `from` to `to`, as
 * a release bumps them; scripts/version.ts then checks that none was missed. Only the copy
 * changes, never the checkout.
 */
export function bumpVersion(root: string, from: string, to: string) {
  const v = from.replaceAll(".", String.raw`\.`);
  edit(root, "Cargo.toml", (text) =>
    replaced(
      "Cargo.toml",
      text,
      new RegExp(String.raw`(\[workspace\.package\]\nversion = ")${v}"`),
      `$1${to}"`,
    ),
  );
  for (const lock of ["Cargo.lock", "apps/shell/src-tauri/Cargo.lock"]) {
    edit(root, lock, (text) => bumpLock(text, from, to));
  }
  edit(root, "apps/shell/src-tauri/Cargo.toml", (text) =>
    replaced(
      "apps/shell/src-tauri/Cargo.toml",
      text,
      new RegExp(String.raw`(\[package\][^[]*?\nversion = ")${v}"`),
      `$1${to}"`,
    ),
  );
  for (const file of [
    "apps/shell/package.json",
    "web/package.json",
    "docs/package.json",
    "integrations/claude-code/.claude-plugin/plugin.json",
  ]) {
    edit(root, file, (text) => replaced(file, text, new RegExp(`("version": ")${v}"`), `$1${to}"`));
  }
  edit(root, "api/openapi.json", (text) => {
    const spec = JSON.parse(text) as {
      info: { version: string };
      components: { schemas: { Meta: { properties: { version: { example: string } } } } };
    };
    spec.info.version = to;
    spec.components.schemas.Meta.properties.version.example = to;
    return `${JSON.stringify(spec, null, 2)}\n`;
  });
  edit(root, "deployments/docker-compose.yml", (text) =>
    replaced(
      "deployments/docker-compose.yml",
      text,
      new RegExp(String.raw`\$\{NEON_VERSION:-${v}\}`, "g"),
      `\${NEON_VERSION:-${to}}`,
    ),
  );
  edit(root, "CHANGELOG.md", (text) =>
    replaced(
      "CHANGELOG.md",
      text,
      /^## /m,
      `## ${to} - Unreleased\n\nA release only make updates-e2e builds, never published.\n\n## `,
    ),
  );
  // The forms scripts/version.ts reads the docs for.
  const forms = new RegExp(
    String.raw`(NEON_VERSION=|neon-teamspace-(?:server|web|updater):|"version": ?"|` +
      String.raw`neon-teamspace-compose-|git checkout v|gh release (?:download|view) v|` +
      String.raw`neon-teamspace/(?:releases/(?:download|tag)|blob|tree)/v)` +
      v +
      String.raw`(?![0-9])`,
    "g",
  );
  const pages = readdirSync(join(root, "docs/src/content/docs"), {
    recursive: true,
    encoding: "utf8",
  })
    .filter((path) => /\.mdx?$/.test(path))
    .map((path) => join("docs/src/content/docs", path));
  for (const file of [
    "README.md",
    "deployments/.env.example",
    "deployments/server.env.example",
    ...pages,
  ]) {
    edit(root, file, (text) => text.replace(forms, `$1${to}`));
  }
}
