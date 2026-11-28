import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const repoRoot = resolve(import.meta.dir, "..");

interface Update {
  "package-ecosystem": string;
  directory?: string;
  directories?: string[];
}

/** The directories Dependabot updates for `ecosystem`, as paths from the repository's root. */
function covered(ecosystem: string): string[] {
  const config = Bun.YAML.parse(readFileSync(join(repoRoot, ".github/dependabot.yml"), "utf8")) as {
    updates: Update[];
  };
  return config.updates
    .filter((update) => update["package-ecosystem"] === ecosystem)
    .flatMap(
      (update) => update.directories ?? (update.directory === undefined ? [] : [update.directory]),
    );
}

/** The directories of the tracked files called `name`, as Dependabot names them. */
function projects(name: string): string[] {
  const listed = Bun.spawnSync(["git", "ls-files", `:(glob)**/${name}`], { cwd: repoRoot });
  return listed.stdout
    .toString()
    .split("\n")
    .filter((file) => file !== "")
    .map((file) => {
      const dir = dirname(file);
      return dir === "." ? "/" : `/${dir}`;
    })
    .toSorted();
}

describe("dependabot.yml", () => {
  for (const [ecosystem, lockfile] of [
    ["bun", "bun.lock"],
    ["cargo", "Cargo.lock"],
    ["gradle", "gradlew"],
  ] as const) {
    it(`updates every project with a ${lockfile}`, () => {
      const found = projects(lockfile);
      expect(found.length).toBeGreaterThan(0);
      expect(covered(ecosystem).toSorted()).toEqual(found);
    });
  }
});
