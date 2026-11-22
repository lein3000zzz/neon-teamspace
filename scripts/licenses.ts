/**
 * Fails when a package installed in one of the repository's JS projects,
 * production or dev, carries a license outside the allow list that cargo-deny
 * enforces for the Rust crates in deny.toml, or when a font the project serves
 * from public/ lacks its license. Packages outside the list need a build-only
 * exception in scripts/licenses/policy.ts.
 *
 * Usage: bun scripts/licenses.ts <project>, where the project is the directory
 * of web, docs or e2e, from wherever the command runs.
 */

import { existsSync, readFileSync, realpathSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { bundledFonts, FONT_LICENSE, fontProblem } from "./licenses/fonts.ts";
import { installedPackages, judge } from "./licenses/packages.ts";
import { POLICIES } from "./licenses/policy.ts";

const repoRoot = realpathSync(resolve(import.meta.dir, ".."));
const denyToml = join(repoRoot, "deny.toml");

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

function printList(heading: string, entries: readonly string[], log = console.log): void {
  if (entries.length > 0) {
    log(heading);
    for (const entry of entries) {
      log(`  ${entry}`);
    }
  }
}

function main(args: readonly string[]): number {
  const known = Object.keys(POLICIES).join(" or ");
  const [arg] = args;
  if (arg === undefined || args.length !== 1) {
    console.error(`Usage: bun scripts/licenses.ts <project>, where the project is ${known}`);
    return 2;
  }
  const project = existsSync(arg) ? realpathSync(arg) : resolve(arg);
  const name = relative(repoRoot, project);
  const policy = POLICIES[name];
  if (policy === undefined) {
    console.error(`${project} is not a project with a license policy: ${known}`);
    return 2;
  }
  if (existsSync(join(project, "node_modules", ".bun"))) {
    console.error(
      `${name}/node_modules has Bun's isolated layout, which hides nested copies from this ` +
        `check. Set linker = "hoisted" in ${name}/bunfig.toml and reinstall.`,
    );
    return 1;
  }
  const packages = installedPackages(project);
  if (packages.length === 0) {
    console.error(`No packages under ${name}/node_modules; run bun install there first.`);
    return 1;
  }

  const verdict = judge(packages, readAllowlist(), policy);
  const publicDir = join(project, "public");
  const fonts = existsSync(publicDir) ? bundledFonts(publicDir) : [];
  const fontFailures = fonts.flatMap((font) => {
    const problem = fontProblem(publicDir, font);
    return problem === null ? [] : [`${name}/public/${font.dir}: ${problem}`];
  });

  console.log(`Checked ${packages.length} packages installed in ${name} against deny.toml:`);
  for (const [license, count] of verdict.counts) {
    console.log(`  ${String(count).padStart(4)}  ${license}`);
  }
  printList("Allowed by a build-only exception:", verdict.excepted);
  printList("Judged by a clarified license:", verdict.clarified);
  if (fonts.length === 0) {
    console.log(`No font files in ${name}/public.`);
  }
  printList(
    `Checked ${fonts.length} font director${fonts.length === 1 ? "y" : "ies"} in ${name}/public:`,
    fonts.map((font) => `${font.dir}: ${font.fonts.length} files, ${FONT_LICENSE}`),
  );

  printList(
    `\n${verdict.failures.length} package(s) with a disallowed license:`,
    verdict.failures,
    console.error,
  );
  printList(
    "\nPolicy entries no installed package needs, remove them from scripts/licenses/policy.ts:",
    verdict.unused,
    console.error,
  );
  printList(
    `\nFonts that may not ship (only ${FONT_LICENSE}, with its text beside them):`,
    fontFailures,
    console.error,
  );
  const failed = verdict.failures.length + verdict.unused.length + fontFailures.length > 0;
  return failed ? 1 : 0;
}

process.exit(main(process.argv.slice(2)));
