import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const repoRoot = resolve(import.meta.dir, "..");

interface Step {
  name?: string;
  uses?: string;
  run?: string;
  env?: Record<string, string>;
  with?: Record<string, unknown>;
}

interface Job {
  needs?: string | string[];
  permissions?: string | Record<string, string>;
  env?: Record<string, string>;
  steps?: Step[];
}

interface Workflow {
  env?: Record<string, string>;
  permissions?: string | Record<string, string>;
  jobs: Record<string, Job>;
}

const workflow = Bun.YAML.parse(
  readFileSync(join(repoRoot, ".github/workflows/release.yml"), "utf8"),
) as Workflow;

/** The desktop apps' updater key, and its password, as the workflow reads them. */
const UPDATER_KEY = /secrets\.TAURI_SIGNING_PRIVATE_KEY/;

/** Any secret of the repository's, beside the workflow's own token. */
const SECRET = /secrets\.(?!GITHUB_TOKEN\b)/;

function job(name: string): Job {
  const found = workflow.jobs[name];
  if (found === undefined) {
    throw new Error(`release.yml has no job ${name}`);
  }
  return found;
}

/** The jobs `name` waits for, directly or through the jobs they wait for. */
function waitsFor(name: string): Set<string> {
  const found = new Set<string>();
  const next = [name];
  for (let current = next.pop(); current !== undefined; current = next.pop()) {
    const needs = job(current).needs ?? [];
    for (const need of typeof needs === "string" ? [needs] : needs) {
      if (!found.has(need)) {
        found.add(need);
        next.push(need);
      }
    }
  }
  return found;
}

/** Each step of the workflow, under its job's name and its own. */
function steps(): { step: Step; label: string }[] {
  return Object.entries(workflow.jobs).flatMap(([name, definition]) =>
    (definition.steps ?? []).map((step) => ({
      step,
      label: `${name}: ${step.name ?? step.uses ?? ""}`,
    })),
  );
}

/** Whether job `name`'s token may write anything, an OIDC token included. */
function writes(name: string): boolean {
  const permissions = job(name).permissions ?? workflow.permissions ?? {};
  if (typeof permissions === "string") {
    return permissions !== "read-all";
  }
  return Object.values(permissions).some((access) => access === "write");
}

describe("release.yml", () => {
  it("gives the updater's key only to the step that bundles the desktop apps", () => {
    expect(JSON.stringify(workflow.env ?? {})).not.toMatch(UPDATER_KEY);
    for (const [name, { env = {} }] of Object.entries(workflow.jobs)) {
      expect(`${name}: ${JSON.stringify(env)}`).not.toMatch(UPDATER_KEY);
    }
    const holders = steps().filter(({ step }) => UPDATER_KEY.test(JSON.stringify(step)));
    expect(holders.map(({ label }) => label)).toEqual(["desktop: Bundle"]);
    const step: Step = holders[0]?.step ?? {};
    // Through its environment alone, to a step that runs Tauri's bundler and nothing that
    // installs or builds the packages and the crates.
    expect(step.uses).toBeUndefined();
    expect(step.run ?? "").not.toMatch(UPDATER_KEY);
    expect(step.run).toContain("tauri bundle");
    for (const builds of ["tauri build", "bun install", "cargo ", "make "]) {
      expect(step.run).not.toContain(builds);
    }
  });

  it("builds the desktop apps without bundling them, before the step that has the key", () => {
    const desktop = job("desktop").steps ?? [];
    const builds = desktop.filter((step) => step.run?.includes("tauri build"));
    expect(builds.length).toBeGreaterThan(0);
    for (const step of builds) {
      expect(step.run).toContain("--no-bundle");
    }
    const bundle = desktop.findIndex((step) => step.name === "Bundle");
    expect(desktop.findLastIndex((step) => step.run?.includes("tauri build"))).toBeLessThan(bundle);
  });

  it("signs nothing with a key of the repository's before the end-to-end tests pass", () => {
    const signers = Object.entries(workflow.jobs)
      .filter(([, definition]) => SECRET.test(JSON.stringify(definition)))
      .map(([name]) => name);
    expect(signers).toContain("desktop");
    expect(signers).toContain("android-release");
    for (const name of signers) {
      expect([name, waitsFor(name).has("e2e")]).toEqual([name, true]);
    }
  });

  it("runs Bun only in jobs whose token only reads", () => {
    const bun = Object.entries(workflow.jobs)
      .filter(([, definition]) =>
        (definition.steps ?? []).some(
          (step) => step.uses?.startsWith("oven-sh/setup-bun@") === true,
        ),
      )
      .map(([name]) => name);
    expect(bun.filter(writes)).toEqual([]);
    expect(bun).toContain("feed");
    // The feed's job reads, and the release job, which writes, takes the feed from it.
    expect(job("feed").permissions).toEqual({ contents: "read" });
    expect(writes("release")).toBe(true);
    expect(waitsFor("release").has("feed")).toBe(true);
  });
});
