import { describe, expect, it, onTestFinished } from "bun:test";
import { resolve } from "node:path";
import { fixture } from "./licenses/fixture.ts";
import { checkVersions, isReleaseTag, workspaceVersion } from "./version.ts";

const repoRoot = resolve(import.meta.dir, "..");

/** A repository whose every file states `version`, with `files` over them. */
function repository(version: string, files: Record<string, string> = {}): string {
  const image = (name: string) =>
    `    image: ghcr.io/lein3000zzz/neon-teamspace-${name}:\${NEON_VERSION:-${version}}\n`;
  return fixture(
    {
      "Cargo.toml": `[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "${version}"\n`,
      "crates/server/Cargo.toml": '[package]\nname = "neon-server"\nversion.workspace = true\n',
      "Cargo.lock": [
        `[[package]]\nname = "neon-server"\nversion = "${version}"\n`,
        `[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`,
      ].join("\n"),
      "integrations/claude-code/.claude-plugin/plugin.json": JSON.stringify({ version }),
      "web/package.json": JSON.stringify({ version }),
      "docs/package.json": JSON.stringify({ version }),
      "api/openapi.json": JSON.stringify({
        info: { version },
        components: { schemas: { Meta: { properties: { version: { example: version } } } } },
      }),
      "deployments/docker-compose.yml": `services:\n  server:\n${image("server")}  web:\n${image("web")}`,
      "CHANGELOG.md": `# Changelog\n\n## ${version} - 2026-11-27\n\nThe first release.\n`,
      "docs/src/content/docs/install.md": `Run \`NEON_VERSION=${version}\`.\n`,
      ...files,
    },
    onTestFinished,
  );
}

describe("checkVersions", () => {
  it("passes a repository whose every file states the workspace's version", () => {
    const root = repository("1.2.3");

    expect(checkVersions(root, "neon 1.2.3\n", "v1.2.3")).toEqual([]);
  });

  it("names each file that states another version, and a tag and a binary of another", () => {
    const root = repository("1.2.3", {
      "crates/store/Cargo.toml": '[package]\nname = "neon-store"\nversion = "1.2.2"\n',
      "Cargo.lock": '[[package]]\nname = "neon-server"\nversion = "1.2.2"\n',
      "web/package.json": JSON.stringify({ version: "0.0.0" }),
      "README.md": "Pull `ghcr.io/lein3000zzz/neon-teamspace-web:1.2.2`.\n",
      "docs/src/content/docs/self-hosting/upgrading.md": [
        "```json",
        '{"name":"Neon Teamspace","version":"1.2.2"}',
        "```",
        "Unpack `neon-teamspace-compose-1.2.2.tar.gz` from",
        "https://github.com/lein3000zzz/neon-teamspace/releases/download/v1.2.2/.",
        "git checkout v1.2.2",
        '```sh\ncurl -d \'{"version": 1, "action": "append", "markdown": "## v1.4.0"}\'\n```',
        "Postgres 18.7 and Compose 2.24.4 are other software.",
      ].join("\n"),
    });

    expect(checkVersions(root, "neon 1.2.2", "v1.2.4")).toEqual([
      "crates/store/Cargo.toml: its version is not version.workspace = true",
      "Cargo.lock: neon-server is 1.2.2: build once to update it",
      'web/package.json: version is "0.0.0"',
      "README.md:1: an image names 1.2.2",
      "docs/src/content/docs/self-hosting/upgrading.md:2: a version in JSON names 1.2.2",
      "docs/src/content/docs/self-hosting/upgrading.md:4: the Compose archive names 1.2.2",
      "docs/src/content/docs/self-hosting/upgrading.md:5: a release names 1.2.2",
      "docs/src/content/docs/self-hosting/upgrading.md:6: a checkout names 1.2.2",
      'neon --version: printed "neon 1.2.2"',
      "the tag v1.2.4 is neither v1.2.3 nor v1.2.3-rc.N",
    ]);
  });

  it("fails a Compose file that runs another release, or latest, unless NEON_VERSION says", () => {
    const root = repository("1.2.3", {
      "deployments/docker-compose.yml": [
        "services:",
        "  server:",
        "    image: ghcr.io/lein3000zzz/neon-teamspace-server:${NEON_VERSION:-latest}",
        "  web:",
        "    image: ghcr.io/lein3000zzz/neon-teamspace-web:1.2.3",
      ].join("\n"),
    });

    expect(checkVersions(root, "neon 1.2.3")).toEqual([
      "deployments/docker-compose.yml: neon-teamspace-server defaults to latest, not the release it belongs to",
      "deployments/docker-compose.yml: has no image line for neon-teamspace-web with ${NEON_VERSION:-...}",
    ]);
  });

  it("takes an unreleased changelog section between releases, but not for a release", () => {
    const root = repository("1.3.0", {
      "CHANGELOG.md": "# Changelog\n\n## 1.3.0 - Unreleased\n\n## 1.2.3 - 2026-11-27\n",
    });

    expect(checkVersions(root, "neon 1.3.0")).toEqual([]);
    expect(checkVersions(root, "neon 1.3.0", "v1.3.0")).toEqual([
      "CHANGELOG.md: the section of 1.3.0 says Unreleased: give it the release's date",
    ]);
  });

  it("fails a changelog whose newest section is another version's or has no version", () => {
    const older = repository("1.3.0", { "CHANGELOG.md": "## 1.2.3 - 2026-11-27\n" });
    const unnamed = repository("1.3.0", { "CHANGELOG.md": "## Next\n\n## 1.3.0 - 2026-11-27\n" });

    expect(checkVersions(older, "neon 1.3.0")).toEqual([
      "CHANGELOG.md: its newest section is 1.2.3",
    ]);
    expect(checkVersions(unnamed, "neon 1.3.0")).toEqual([
      'CHANGELOG.md: its newest section, "## Next", is not "## 1.3.0 - YYYY-MM-DD"',
    ]);
  });

  it("takes a release candidate's tag of the version, which needs the changelog's date too", () => {
    const dated = repository("1.2.3");
    const unreleased = repository("1.2.3", {
      "CHANGELOG.md": "# Changelog\n\n## 1.2.3 - Unreleased\n",
    });

    expect(checkVersions(dated, "neon 1.2.3", "v1.2.3-rc.1")).toEqual([]);
    expect(checkVersions(dated, "neon 1.2.3", "v1.2.3-rc.12")).toEqual([]);
    expect(checkVersions(unreleased, "neon 1.2.3", "v1.2.3-rc.1")).toEqual([
      "CHANGELOG.md: the section of 1.2.3 says Unreleased: give it the release's date",
    ]);
  });

  it("fails a candidate's tag of another version, or another kind of pre-release", () => {
    const root = repository("1.2.3");

    for (const tag of ["v1.2.4-rc.1", "v1.2.3-beta.1", "v1.2.3-rc", "v1.2.3-rc.01", "1.2.3-rc.1"]) {
      expect(checkVersions(root, "neon 1.2.3", tag)).toEqual([
        `the tag ${tag} is neither v1.2.3 nor v1.2.3-rc.N`,
      ]);
    }
  });

  it("takes no candidate of a version that is a pre-release itself", () => {
    const root = repository("1.2.3-rc.1");

    expect(checkVersions(root, "neon 1.2.3-rc.1", "v1.2.3-rc.1")).toEqual([]);
    expect(checkVersions(root, "neon 1.2.3-rc.1", "v1.2.3-rc.1-rc.1")).toEqual([
      "the tag v1.2.3-rc.1-rc.1 is not v1.2.3-rc.1",
    ]);
  });

  it("fails a file that states the candidate's version rather than the release's", () => {
    const root = repository("1.2.3", {
      "README.md": "Pull `ghcr.io/lein3000zzz/neon-teamspace-server:1.2.3-rc.1`.\n",
    });

    expect(checkVersions(root, "neon 1.2.3", "v1.2.3-rc.1")).toEqual([
      "README.md:1: an image names 1.2.3-rc.1",
    ]);
  });

  it("passes the repository itself, as the workspace's version", () => {
    const version = workspaceVersion(repoRoot);

    expect(version).toMatch(/^\d+\.\d+\.\d+/);
    expect(checkVersions(repoRoot, `neon ${version}`)).toEqual([]);
  });
});

describe("isReleaseTag", () => {
  it.each([
    ["v1.2.3", "1.2.3", true],
    ["v1.2.3-rc.1", "1.2.3", true],
    ["v1.2.3-rc.0", "1.2.3", true],
    ["v1.2.3-rc.1.1", "1.2.3", false],
    ["v1.2.3-rc.1-x", "1.2.3", false],
    ["v1.2.30-rc.1", "1.2.3", false],
    ["v1.2.3-rc.1", "1.2.3-rc.1", true],
    ["v1.2.3-rc.1-rc.1", "1.2.3-rc.1", false],
  ])("%s releases %s: %p", (tag, version, releases) => {
    expect(isReleaseTag(tag, version)).toBe(releases);
  });
});
