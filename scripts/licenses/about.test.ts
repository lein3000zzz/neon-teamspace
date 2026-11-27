import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const repoRoot = resolve(import.meta.dir, "..", "..");

function toml(file: string): Record<string, unknown> {
  return Bun.TOML.parse(readFileSync(join(repoRoot, file), "utf8")) as Record<string, unknown>;
}

describe("about.toml", () => {
  // cargo-about writes the notices of the server image and of the apps; cargo-deny decides
  // what may be linked, with the root's deny.toml for both workspaces.
  for (const file of ["about.toml", "apps/shell/about.toml"]) {
    it(`${file} accepts exactly the licenses deny.toml allows`, () => {
      const allowed = (toml("deny.toml")["licenses"] as { allow: string[] }).allow;
      const accepted = toml(file)["accepted"] as string[];

      expect(accepted.toSorted()).toEqual(allowed.toSorted());
      expect(new Set(accepted).size).toBe(accepted.length);
    });
  }
});
