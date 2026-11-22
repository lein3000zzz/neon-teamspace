/**
 * What the third-party notices of the web app's and the docs' builds share: the
 * installed package a module belongs to, the license texts a package ships, and
 * the notices file's layout. Each build finds the packages its own files hold.
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { declaredLicense } from "./spdx.ts";

const LICENSE_FILE = /^(?:licen[cs]e|copying)\b/i;
const NODE_MODULES = "/node_modules/";
const RULE = "=".repeat(80);
const THIN_RULE = "-".repeat(80);

export interface Notice {
  name: string;
  version: string;
  license: string;
  text: string;
}

/**
 * The installed package a module id belongs to, as its root directory, or
 * undefined for the project's own files and virtual modules. The last
 * node_modules in the path wins, so a nested copy counts as itself.
 */
export function packageRoot(id: string): string | undefined {
  const path = id.split("?")[0] ?? id;
  const at = path.lastIndexOf(NODE_MODULES);
  if (path.startsWith("\0") || at === -1) {
    return undefined;
  }
  const base = at + NODE_MODULES.length;
  const segments = path.slice(base).split("/");
  const length = segments[0]?.startsWith("@") ? 2 : 1;
  return segments.length > length
    ? path.slice(0, base) + segments.slice(0, length).join("/")
    : undefined;
}

/**
 * The license texts a package ships at its root, sorted by file name: each
 * license file, and each file in a license directory, as some packages have.
 */
export function licenseTexts(root: string): string[] {
  return filesIn(root)
    .filter((entry) => LICENSE_FILE.test(entry.name))
    .flatMap((entry) => {
      const path = join(root, entry.name);
      if (entry.isFile()) {
        return [path];
      }
      return entry.isDirectory()
        ? filesIn(path)
            .filter((file) => file.isFile())
            .map((file) => join(path, file.name))
        : [];
    })
    .map((path) => readFileSync(path, "utf8").trim());
}

function filesIn(dir: string) {
  return readdirSync(dir, { withFileTypes: true }).toSorted((a, b) =>
    a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
  );
}

/**
 * License texts for packages that ship none, by package name: each loads the
 * texts, copied from the package's source repository.
 */
export type Supplied = Readonly<Record<string, () => readonly string[]>>;

/**
 * The notice for the package installed at `root`: the license texts it ships,
 * or else those `supplied` has for its name. Undefined when there are neither.
 * Throws when the package ships texts of its own while `supplied` has some, so
 * no stale copy stays behind.
 */
export function packageNotice(root: string, supplied: Supplied = {}): Notice | undefined {
  const manifestPath = join(root, "package.json");
  if (!existsSync(manifestPath)) {
    throw new Error(`${root} is not an installed package`);
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Record<string, unknown>;
  const name = String(manifest["name"]);
  const own = licenseTexts(root);
  const copies = Object.hasOwn(supplied, name) ? supplied[name] : undefined;
  if (copies !== undefined && own.length > 0) {
    throw new Error(`${name} now ships its own license; remove it from SUPPLIED`);
  }
  const texts = copies?.() ?? own;
  if (texts.length === 0) {
    return undefined;
  }
  return {
    name,
    version: String(manifest["version"]),
    license: declaredLicense(manifest) ?? "not declared",
    text: texts.map((text) => text.trim()).join("\n\n"),
  };
}

/** One section of the notices file: a heading, its license, and the text. */
export function section(title: string, license: string, text: string): string {
  return [RULE, title, `License: ${license}`, THIN_RULE, "", text, ""].join("\n");
}

/** The sections of `notices`, sorted by package name and version. */
export function packageSections(notices: Iterable<Notice>): string[] {
  return [...notices]
    .toSorted((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
    .map((notice) => section(`${notice.name} ${notice.version}`, notice.license, notice.text));
}

/** The notices file: its title, a paragraph on what it covers, then the sections. */
export function noticesFile(intro: readonly string[], sections: readonly string[]): string {
  return ["Third-party notices", "", ...intro, "", ...sections].join("\n");
}
