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
  /** The package's source repository, as its package.json names it, when it does. */
  repository?: string;
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
 * The web address of a package's source repository, from its package.json's
 * `repository`, with the directory of a monorepo's package after it. Undefined
 * unless the field gives an https address, as published packages' do.
 */
export function repositoryUrl(manifest: Readonly<Record<string, unknown>>): string | undefined {
  const field = manifest["repository"];
  const entry =
    typeof field === "object" && field !== null ? (field as Record<string, unknown>) : undefined;
  const raw = typeof field === "string" ? field : entry?.["url"];
  if (typeof raw !== "string") {
    return undefined;
  }
  const url = raw.replace(/^git\+/, "").replace(/\.git$/, "");
  if (!url.startsWith("https://")) {
    return undefined;
  }
  const directory = entry?.["directory"];
  return typeof directory === "string" ? `${url} (${directory})` : url;
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
  const repository = repositoryUrl(manifest);
  return {
    name,
    version: String(manifest["version"]),
    license: declaredLicense(manifest) ?? "not declared",
    text: texts.map((text) => text.trim()).join("\n\n"),
    ...(repository === undefined ? {} : { repository }),
  };
}

/**
 * One section of the notices file: a heading, its license, any further lines
 * about it, and the text.
 */
export function section(
  title: string,
  license: string,
  text: string,
  details: readonly string[] = [],
): string {
  return [RULE, title, `License: ${license}`, ...details, THIN_RULE, "", text, ""].join("\n");
}

/**
 * Where to get the source of a package under MPL-2.0, which asks whoever ships
 * it in another form to say so: the package as npm publishes it, and its
 * repository. Other licenses ask for nothing of the kind.
 */
export function sourceLines(notice: Notice): string[] {
  if (!/\bMPL-2\.0\b/.test(notice.license)) {
    return [];
  }
  return [
    `Source: https://www.npmjs.com/package/${notice.name}/v/${notice.version}`,
    ...(notice.repository === undefined ? [] : [`Repository: ${notice.repository}`]),
  ];
}

/** The sections of `notices`, sorted by package name and version. */
export function packageSections(notices: Iterable<Notice>): string[] {
  return [...notices]
    .toSorted((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
    .map((notice) =>
      section(`${notice.name} ${notice.version}`, notice.license, notice.text, sourceLines(notice)),
    );
}

/** The notices file: its title, a paragraph on what it covers, then the sections. */
export function noticesFile(intro: readonly string[], sections: readonly string[]): string {
  return ["Third-party notices", "", ...intro, "", ...sections].join("\n");
}
