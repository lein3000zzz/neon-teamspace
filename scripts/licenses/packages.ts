/**
 * The packages installed in a JS project, and the verdict on their licenses.
 * It walks node_modules rather than the lockfile, so it judges what was really
 * installed, nested copies included. That needs the hoisted layout, which
 * nests each conflicting copy under its dependent.
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import type { Clarification, Exception, Policy } from "./policy.ts";
import { declaredLicense, parseSpdx, satisfies } from "./spdx.ts";

export interface Installed {
  name: string;
  version: string;
  /** What its package.json declares, as one SPDX expression. */
  license: string | undefined;
  /** Its directory, relative to the project. */
  path: string;
}

/** Every package under the project's node_modules, nested copies included. */
export function installedPackages(project: string): Installed[] {
  return [...packagesIn(join(project, "node_modules"), project)];
}

function* packagesIn(nodeModules: string, project: string): Generator<Installed> {
  if (!existsSync(nodeModules)) {
    return;
  }
  for (const entry of readdirSync(nodeModules)) {
    // .bin, .cache, .vite and other tool state, not packages.
    if (entry.startsWith(".")) {
      continue;
    }
    const path = join(nodeModules, entry);
    if (entry.startsWith("@")) {
      for (const scoped of readdirSync(path)) {
        yield* packageAt(join(path, scoped), project);
      }
    } else {
      yield* packageAt(path, project);
    }
  }
}

function* packageAt(path: string, project: string): Generator<Installed> {
  const manifestPath = join(path, "package.json");
  if (!existsSync(manifestPath)) {
    return;
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Record<string, unknown>;
  yield {
    name: String(manifest["name"]),
    version: String(manifest["version"]),
    license: declaredLicense(manifest),
    path: relative(project, path),
  };
  yield* packagesIn(join(path, "node_modules"), project);
}

export interface Verdict {
  /** Packages that may not be installed, each with its problem. */
  failures: string[];
  /** Policy entries that no installed package needs, to be removed. */
  unused: string[];
  /** Packages allowed by a build-only exception, with their licenses. */
  excepted: string[];
  /** Packages judged by a clarified license instead of their metadata. */
  clarified: string[];
  /** How many packages are judged by each license, most used first. */
  counts: [license: string, count: number][];
}

/** Judges the installed packages by the allow list and the project's policy. */
export function judge(
  packages: readonly Installed[],
  allowed: ReadonlySet<string>,
  policy: Policy,
): Verdict {
  const failures: string[] = [];
  const excepted: string[] = [];
  const clarified: string[] = [];
  const usedExceptions = new Set<Exception>();
  const usedClarifications = new Set<Clarification>();
  const counts = new Map<string, number>();

  for (const pkg of packages) {
    const id = `${pkg.name}@${pkg.version}`;
    const clarification = policy.clarifications.find(
      (entry) => entry.name === pkg.name && entry.version === pkg.version,
    );
    if (clarification !== undefined) {
      usedClarifications.add(clarification);
      clarified.push(`${id}: ${clarification.license}`);
    }
    const license = clarification?.license ?? pkg.license;
    const key = license ?? "none";
    counts.set(key, (counts.get(key) ?? 0) + 1);

    const exception = policy.exceptions.find(
      (entry) => namesMatch(entry.name, pkg.name) && entry.license === license,
    );
    if (exception !== undefined) {
      usedExceptions.add(exception);
      excepted.push(`${id}: ${license}`);
      continue;
    }
    const problem = licenseProblem(license, allowed);
    if (problem !== null) {
      failures.push(`${id} (${pkg.path}): ${problem}`);
    }
  }

  const unused = [
    ...policy.exceptions
      .filter((entry) => !usedExceptions.has(entry))
      .map((entry) => `exception ${entry.name}: ${entry.license}`),
    ...policy.clarifications
      .filter((entry) => !usedClarifications.has(entry))
      .map((entry) => `clarification ${entry.name}@${entry.version}`),
  ];
  return {
    failures,
    unused,
    excepted,
    clarified,
    counts: [...counts].toSorted(([, a], [, b]) => b - a),
  };
}

/** An exact name, or a prefix ending in `*`. */
function namesMatch(pattern: string, name: string): boolean {
  return pattern.endsWith("*") ? name.startsWith(pattern.slice(0, -1)) : pattern === name;
}

function licenseProblem(license: string | undefined, allowed: ReadonlySet<string>): string | null {
  if (license === undefined) {
    return "declares no license";
  }
  try {
    return satisfies(parseSpdx(license), allowed) ? null : `license ${license}`;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}
