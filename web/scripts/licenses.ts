/**
 * Fails when an installed package, production or dev, carries a license
 * outside the allowlist the Rust side enforces with cargo-deny, or when a
 * bundled font lacks its license. Walks node_modules rather than the
 * lockfile, so it checks what was really installed, nested copies included.
 *
 * Usage: bun run licenses
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { bundledFonts, FONT_LICENSE, fontProblem } from "./licenses/fonts.ts";
import { declaredLicense, parseSpdx, satisfies } from "./licenses/spdx.ts";

const webRoot = resolve(import.meta.dir, "..");
const denyToml = resolve(webRoot, "../deny.toml");
const publicDir = resolve(webRoot, "public");

/**
 * Packages allowed outside the allowlist: data, never code, which deny.toml
 * governs alone. Each is keyed by name and exact license, so a relicensed
 * version fails again, and carries its reason.
 */
const EXCEPTIONS: Record<string, { license: string; reason: string }> = {
  "caniuse-lite": {
    license: "CC-BY-4.0",
    reason:
      "Browser support tables (with a small loader) read by browserslist, which " +
      "@babel/core loads for the TanStack Router plugin at build time. Never in the bundle.",
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

  const fonts = bundledFonts(publicDir);
  const fontFailures = fonts.flatMap((font) => {
    const problem = fontProblem(publicDir, font);
    return problem === null ? [] : [`public/${font.dir}: ${problem}`];
  });

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
  console.log(`Checked ${fonts.length} font director${fonts.length === 1 ? "y" : "ies"}:`);
  for (const font of fonts) {
    console.log(`  public/${font.dir}: ${font.fonts.length} files, ${FONT_LICENSE}`);
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
  if (fontFailures.length > 0) {
    console.error(`\nFonts that may not ship (only ${FONT_LICENSE}, with its text beside them):`);
    for (const failure of fontFailures) {
      console.error(`  ${failure}`);
    }
  }
  return failures.length > 0 || unused.length > 0 || fontFailures.length > 0 ? 1 : 0;
}

process.exit(main());
