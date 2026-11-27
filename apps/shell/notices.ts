/**
 * Writes THIRD_PARTY_NOTICES.txt into the picker's build, which the app embeds with the
 * picker, and the picker links to: the license text of every package whose code is in the
 * picker's bundle, then those of the Rust crates the app links on any of its platforms, as
 * cargo-about finds them (about.toml). MIT, ISC and BSD ask for their notice in every copy and
 * Apache-2.0 for a copy of the license, while the app's binary and the minified bundle carry
 * none.
 */

import { spawnSync } from "node:child_process";
import type { Plugin } from "vite";
import {
  type Notice,
  noticesFile,
  packageNotice,
  packageRoot,
  packageSections,
} from "../../scripts/licenses/notices.ts";

export const NOTICES_FILE = "THIRD_PARTY_NOTICES.txt";

const INTRO = [
  "The Neon Teamspace apps are built from the JavaScript packages and the Rust crates",
  "below, each under the license named with it. Each package comes with the license text",
  "and copyright notice it ships; then each license text the crates ship has a section,",
  "with the crates that ship it.",
];

/** The crates' sections, from cargo-about, run in the app's directory `root`. */
export function crateNotices(root: string, cargo = process.env["CARGO"] ?? "cargo"): string {
  const result = spawnSync(
    cargo,
    [
      "about",
      "generate",
      "--locked",
      "--fail",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--config",
      "about.toml",
      "about.hbs",
    ],
    { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  );
  if (result.error !== undefined) {
    throw new Error(`could not run ${cargo} about (install cargo-about): ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`cargo about could not list the crates' licenses:\n${result.stderr}`);
  }
  return result.stdout.trim();
}

/** The notices of the packages that the modules `moduleIds` belong to. */
export function packageNotices(moduleIds: Iterable<string>): Notice[] {
  const roots = new Set<string>();
  for (const id of moduleIds) {
    const root = packageRoot(id);
    if (root !== undefined) {
      roots.add(root);
    }
  }
  const missing = [...roots].filter((root) => packageNotice(root) === undefined);
  if (missing.length > 0) {
    throw new Error(`No license text in ${missing.join(", ")}`);
  }
  return [...roots].flatMap((root) => packageNotice(root) ?? []);
}

/** The Vite plugin: emits the notices next to the picker in production builds. */
export function thirdPartyNotices(): Plugin {
  let root = "";
  return {
    name: "neon:app-notices",
    apply: "build",
    configResolved(config) {
      root = config.root;
    },
    generateBundle(_, bundle) {
      const moduleIds = Object.values(bundle).flatMap((output) =>
        output.type === "chunk" ? output.moduleIds : [],
      );
      try {
        const sections = [...packageSections(packageNotices(moduleIds)), crateNotices(root)];
        this.emitFile({
          type: "asset",
          fileName: NOTICES_FILE,
          source: `${noticesFile(INTRO, sections)}\n`,
        });
      } catch (error) {
        this.error(error instanceof Error ? error.message : String(error));
      }
    },
  };
}
