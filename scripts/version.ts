/**
 * Checks that every place stating Neon Teamspace's version states the one the cargo workspace
 * gives every crate, in the root Cargo.toml: the crates and Cargo.lock, the Claude Code plugin,
 * the packages of the web app and the docs, the API description, the Compose file's images,
 * the changelog, the docs, `neon --version`, and the release tag when there is one.
 *
 * The server answers GET /api/v1/meta, and the web app shows, the version the neon binary was
 * built with, the one `neon --version` prints; a server test and the end-to-end tests check both
 * against the running server.
 *
 * A release's tag is `v` and the version. A release candidate's tag, `v` and the version then
 * `-rc.` and a number, rehearses the release of that version: everything states the version
 * alone, so the release itself can then tag the same commit.
 *
 * Usage: bun scripts/version.ts --neon "<what neon --version printed>" [--tag vX.Y.Z[-rc.N]]
 */

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

/** A version as Cargo takes it: major.minor.patch, then a pre-release such as -rc.1. */
const SEMVER = String.raw`\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?`;
const SEMVER_ONLY = new RegExp(`^${SEMVER}$`);
/** A version without a pre-release, which a release candidate's tag may name. */
const RELEASE_ONLY = /^\d+\.\d+\.\d+$/;
/** What follows the version in a release candidate's tag: -rc. and its number. */
const CANDIDATE = /^-rc\.(?:0|[1-9]\d*)$/;

/**
 * How the docs state Neon Teamspace's version, each with the version as its first group: the
 * variable that picks the images, the images, /api/v1/meta's answer, the Compose archive, the
 * repository's release pages and files at a tag, and the tag a clone checks out. Other numbers,
 * such as the versions of other software or of a webhook's payload, are none of these.
 */
const DOC_FORMS: { name: string; pattern: RegExp }[] = [
  { name: "NEON_VERSION", pattern: new RegExp(`NEON_VERSION=(${SEMVER})`, "g") },
  { name: "an image", pattern: new RegExp(`neon-teamspace-(?:server|web):(${SEMVER})`, "g") },
  { name: "a version in JSON", pattern: new RegExp(`"version": ?"(${SEMVER})"`, "g") },
  { name: "the Compose archive", pattern: new RegExp(`neon-teamspace-compose-(${SEMVER})`, "g") },
  {
    name: "a release",
    pattern: new RegExp(
      `(?:neon-teamspace/(?:releases/(?:download|tag)|blob|tree)/|gh release (?:download|view) )v(${SEMVER})`,
      "g",
    ),
  },
  { name: "a checkout", pattern: new RegExp(`git checkout v(${SEMVER})`, "g") },
];

/** The files of the docs that DOC_FORMS reads, beside the docs' pages. */
const DOC_FILES = ["README.md", "deployments/.env.example", "deployments/server.env.example"];

/** A changelog's section heading: `## 1.2.3 - 2026-11-27`, or `## 1.2.3 - Unreleased`. */
const CHANGELOG_SECTION = new RegExp(
  `^## (${SEMVER}) - (\\d{4}-\\d{2}-\\d{2}|Unreleased)$|^## (.*)$`,
  "m",
);

function readJson(path: string): Record<string, unknown> {
  return JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
}

/** The version the workspace's Cargo.toml gives every crate. */
export function workspaceVersion(root: string): string {
  const cargo = Bun.TOML.parse(readFileSync(join(root, "Cargo.toml"), "utf8")) as {
    workspace?: { package?: { version?: unknown } };
  };
  const version = cargo.workspace?.package?.version;
  if (typeof version !== "string") {
    throw new Error("Cargo.toml has no [workspace.package] version");
  }
  return version;
}

/** Every markdown file under `dir`, as paths relative to `root`. */
function markdownFiles(root: string, dir: string): string[] {
  const full = join(root, dir);
  if (!existsSync(full)) {
    return [];
  }
  return readdirSync(full, { recursive: true, encoding: "utf8" })
    .filter((path) => /\.mdx?$/.test(path) && statSync(join(full, path)).isFile())
    .map((path) => join(dir, path))
    .toSorted();
}

/** The crates' manifests: each must take its version from the workspace. */
function checkCrates(root: string): string[] {
  const crates = join(root, "crates");
  return readdirSync(crates)
    .filter((name) => existsSync(join(crates, name, "Cargo.toml")))
    .toSorted()
    .flatMap((name) => {
      const manifest = Bun.TOML.parse(readFileSync(join(crates, name, "Cargo.toml"), "utf8")) as {
        package?: { version?: unknown };
      };
      const version = manifest.package?.version as { workspace?: unknown } | undefined;
      return version?.workspace === true
        ? []
        : [`crates/${name}/Cargo.toml: its version is not version.workspace = true`];
    });
}

/** Cargo.lock's entries of the workspace's own crates, which have no source. */
function checkLock(root: string, version: string): string[] {
  const lock = Bun.TOML.parse(readFileSync(join(root, "Cargo.lock"), "utf8")) as {
    package?: { name: string; version: string; source?: string }[];
  };
  return (lock.package ?? [])
    .filter((entry) => entry.source === undefined && entry.version !== version)
    .map((entry) => `Cargo.lock: ${entry.name} is ${entry.version}: build once to update it`);
}

/** A JSON file's field, read by its path. */
function checkJsonField(root: string, file: string, path: string[], version: string): string[] {
  let value: unknown = readJson(join(root, file));
  for (const key of path) {
    value =
      value !== null && typeof value === "object"
        ? (value as Record<string, unknown>)[key]
        : undefined;
  }
  return value === version ? [] : [`${file}: ${path.join(".")} is ${JSON.stringify(value)}`];
}

/** The release the Compose file runs when NEON_VERSION is unset, for both images. */
function checkCompose(root: string, version: string): string[] {
  const file = "deployments/docker-compose.yml";
  const source = readFileSync(join(root, file), "utf8");
  return ["server", "web"].flatMap((image) => {
    const line = new RegExp(
      `^\\s+image: ghcr\\.io/[^/]+/neon-teamspace-${image}:\\$\\{NEON_VERSION:-([^}]*)\\}$`,
      "m",
    ).exec(source);
    if (line === null) {
      return [`${file}: has no image line for neon-teamspace-${image} with \${NEON_VERSION:-...}`];
    }
    return line[1] === version
      ? []
      : [`${file}: neon-teamspace-${image} defaults to ${line[1]}, not the release it belongs to`];
  });
}

/**
 * The changelog's newest section must be the version's. A release's has its date; between
 * releases the section of the next one may say Unreleased.
 */
function checkChangelog(root: string, version: string, releasing: boolean): string[] {
  const file = "CHANGELOG.md";
  if (!existsSync(join(root, file))) {
    return [`${file}: is missing`];
  }
  const first = CHANGELOG_SECTION.exec(readFileSync(join(root, file), "utf8"));
  if (first === null) {
    return [`${file}: has no section`];
  }
  if (first[1] === undefined) {
    return [`${file}: its newest section, "## ${first[3]}", is not "## ${version} - YYYY-MM-DD"`];
  }
  if (first[1] !== version) {
    return [`${file}: its newest section is ${first[1]}`];
  }
  return releasing && first[2] === "Unreleased"
    ? [`${file}: the section of ${version} says Unreleased: give it the release's date`]
    : [];
}

/** The docs' statements of the version, in the forms of DOC_FORMS. */
function checkDocs(root: string, version: string): string[] {
  const files = [
    ...DOC_FILES.filter((file) => existsSync(join(root, file))),
    ...markdownFiles(root, "docs/src/content/docs"),
  ];
  return files.flatMap((file) => {
    const lines = readFileSync(join(root, file), "utf8").split("\n");
    return lines.flatMap((line, index) =>
      DOC_FORMS.flatMap(({ name, pattern }) =>
        [...line.matchAll(pattern)]
          .filter((match) => match[1] !== version)
          .map((match) => `${file}:${index + 1}: ${name} names ${match[1]}`),
      ),
    );
  });
}

/**
 * Whether `tag` releases `version`: `v` and the version, or, for a version without a
 * pre-release of its own, a release candidate of it, such as v1.2.3-rc.1 for 1.2.3.
 */
export function isReleaseTag(tag: string, version: string): boolean {
  const release = `v${version}`;
  if (tag === release) {
    return true;
  }
  return (
    RELEASE_ONLY.test(version) &&
    tag.startsWith(release) &&
    CANDIDATE.test(tag.slice(release.length))
  );
}

/**
 * Every place that states another version than the workspace's. `neon` is what
 * `neon --version` printed; `tag`, when given, is the release's tag, which must be `v` and
 * the version or a release candidate of it, and whose changelog section must then have its
 * date.
 */
export function checkVersions(root: string, neon: string, tag?: string): string[] {
  const version = workspaceVersion(root);
  if (!SEMVER_ONLY.test(version)) {
    return [`Cargo.toml: the workspace's version ${version} is not major.minor.patch`];
  }
  const problems = [
    ...checkCrates(root),
    ...checkLock(root, version),
    ...checkJsonField(
      root,
      "integrations/claude-code/.claude-plugin/plugin.json",
      ["version"],
      version,
    ),
    ...checkJsonField(root, "web/package.json", ["version"], version),
    ...checkJsonField(root, "docs/package.json", ["version"], version),
    ...checkJsonField(root, "api/openapi.json", ["info", "version"], version),
    ...checkJsonField(
      root,
      "api/openapi.json",
      ["components", "schemas", "Meta", "properties", "version", "example"],
      version,
    ),
    ...checkCompose(root, version),
    ...checkChangelog(root, version, tag !== undefined),
    ...checkDocs(root, version),
  ];
  if (neon.trim() !== `neon ${version}`) {
    problems.push(`neon --version: printed ${JSON.stringify(neon.trim())}`);
  }
  if (tag !== undefined && !isReleaseTag(tag, version)) {
    problems.push(
      RELEASE_ONLY.test(version)
        ? `the tag ${tag} is neither v${version} nor v${version}-rc.N`
        : `the tag ${tag} is not v${version}`,
    );
  }
  return problems;
}

function main(args: readonly string[]): number {
  const usage =
    'Usage: bun scripts/version.ts --neon "<what neon --version printed>" [--tag vX.Y.Z[-rc.N]]';
  const options = new Map<string, string>();
  for (let index = 0; index < args.length; index += 2) {
    const [name, value] = [args[index], args[index + 1]];
    if ((name !== "--neon" && name !== "--tag") || value === undefined) {
      console.error(usage);
      return 2;
    }
    options.set(name, value);
  }
  const neon = options.get("--neon");
  if (neon === undefined) {
    console.error(usage);
    return 2;
  }
  const root = resolve(import.meta.dir, "..");
  const version = workspaceVersion(root);
  const problems = checkVersions(root, neon, options.get("--tag"));
  if (problems.length > 0) {
    console.error(`Not the workspace's version, ${version} in Cargo.toml:`);
    for (const problem of problems) {
      console.error(`  ${problem}`);
    }
    return 1;
  }
  const tag = options.get("--tag");
  console.log(
    `Every stated version is ${version}${tag === undefined ? "" : `, which the tag ${tag} names`}.`,
  );
  return 0;
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2)));
}
