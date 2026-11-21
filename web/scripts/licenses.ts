/**
 * Fails when an installed package, production or dev, carries a license
 * outside the allowlist the Rust side enforces with cargo-deny. Walks
 * node_modules rather than the lockfile, so it checks what was really
 * installed, nested copies included.
 *
 * Usage: bun run licenses
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { declaredLicense, parseSpdx, satisfies } from "./licenses/spdx";

const webRoot = resolve(import.meta.dir, "..");
const denyToml = resolve(webRoot, "../deny.toml");

/**
 * Packages allowed outside the allowlist, each by name and exact license, so
 * a relicensed version fails again. Keep every entry justified.
 */
const EXCEPTIONS: Record<string, { license: string; reason: string }> = {
  "caniuse-lite": {
    license: "CC-BY-4.0",
    reason: "Browser support data used at build time by browserslist; data, not code.",
  },
  "graceful-fs": {
    license: "BlueOak-1.0.0",
    reason: "Permissive (Blue Oak Model License); build-time only, via @tailwindcss/node.",
  },
  "lru-cache": {
    license: "BlueOak-1.0.0",
    reason: "Permissive (Blue Oak Model License); test-only, via jsdom.",
  },
  argparse: {
    license: "Python-2.0",
    reason: "Permissive (PSF license); build-time only, via openapi-typescript.",
  },
  isbot: {
    license: "Unlicense",
    reason: "Public-domain dedication; shipped via @tanstack/react-router.",
  },
};

interface Installed {
  name: string;
  version: string;
  license: string | undefined;
  path: string;
}

function readAllowlist(): Set<string> {
  const parsed = Bun.TOML.parse(readFileSync(denyToml, "utf8")) as {
    licenses?: { allow?: unknown };
  };
  const allow = parsed.licenses?.allow;
  if (!Array.isArray(allow) || !allow.every((entry) => typeof entry === "string")) {
    throw new Error(`${denyToml} has no [licenses] allow list`);
  }
  return new Set(allow);
}

function* packagesIn(nodeModules: string): Generator<Installed> {
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
        yield* packageAt(join(path, scoped));
      }
    } else {
      yield* packageAt(path);
    }
  }
}

function* packageAt(path: string): Generator<Installed> {
  const manifestPath = join(path, "package.json");
  if (!existsSync(manifestPath)) {
    return;
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Record<string, unknown>;
  yield {
    name: String(manifest["name"]),
    version: String(manifest["version"]),
    license: declaredLicense(manifest),
    path: relative(webRoot, path),
  };
  yield* packagesIn(join(path, "node_modules"));
}

function verdict(pkg: Installed, allowed: ReadonlySet<string>): string | null {
  const exception = EXCEPTIONS[pkg.name];
  if (exception !== undefined && exception.license === pkg.license) {
    return null;
  }
  if (pkg.license === undefined) {
    return "declares no license";
  }
  try {
    return satisfies(parseSpdx(pkg.license), allowed) ? null : `license ${pkg.license}`;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

function main(): number {
  const allowed = readAllowlist();
  const packages = [...packagesIn(join(webRoot, "node_modules"))];
  if (packages.length === 0) {
    console.error("No packages under node_modules; run bun install first.");
    return 1;
  }

  const failures = packages.flatMap((pkg) => {
    const problem = verdict(pkg, allowed);
    return problem === null ? [] : [`${pkg.name}@${pkg.version} (${pkg.path}): ${problem}`];
  });

  // An exception nothing needs any more is removed, so the list stays honest.
  const unused = Object.entries(EXCEPTIONS)
    .filter(
      ([name, { license }]) =>
        !packages.some((pkg) => pkg.name === name && pkg.license === license),
    )
    .map(([name, { license }]) => `${name} (${license})`);

  const counts = new Map<string, number>();
  for (const pkg of packages) {
    const key = pkg.license ?? "none";
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  console.log(
    `Checked ${packages.length} installed packages against ${relative(webRoot, denyToml)}:`,
  );
  for (const [license, count] of [...counts].toSorted(([, a], [, b]) => b - a)) {
    console.log(`  ${String(count).padStart(4)}  ${license}`);
  }

  if (failures.length > 0) {
    console.error(`\n${failures.length} package(s) with a disallowed license:`);
    for (const failure of failures) {
      console.error(`  ${failure}`);
    }
  }
  if (unused.length > 0) {
    console.error(`\nExceptions no installed package needs, remove them:`);
    for (const entry of unused) {
      console.error(`  ${entry}`);
    }
  }
  return failures.length > 0 || unused.length > 0 ? 1 : 0;
}

process.exit(main());
