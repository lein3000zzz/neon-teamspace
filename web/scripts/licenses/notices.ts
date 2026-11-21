/**
 * Writes THIRD_PARTY_NOTICES.txt into the production build: the license text
 * of every package whose code ends up in the shipped files, and of every font
 * served from public/. MIT, ISC and BSD ask for their notice in every copy and
 * Apache-2.0 for a copy of the license, while the minifier drops the comments
 * that carried them.
 */

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { Plugin } from "vite";
import {
  bundledFonts,
  FONT_LICENSE,
  fontLicenseText,
  fontProblem,
} from "../../../scripts/licenses/fonts.ts";
import { declaredLicense } from "../../../scripts/licenses/spdx.ts";

const LICENSE_FILE = /^(?:licen[cs]e|copying)\b/i;
const NODE_MODULES = "/node_modules/";
const RULE = "=".repeat(80);
const THIN_RULE = "-".repeat(80);

/** Where the SUPPLIED texts live, relative to the web app's root. */
const SUPPLIED_DIR = "scripts/licenses/supplied";

/**
 * License texts for bundled packages that publish none: package name to a
 * file in SUPPLIED_DIR, copied from the package's source repository. The build
 * fails once such a package ships its own text or leaves the bundle, so no
 * stale copy stays behind.
 */
const SUPPLIED: Readonly<Record<string, string>> = {
  "react-remove-scroll-bar": "react-remove-scroll-bar.LICENSE",
};

interface Notice {
  name: string;
  version: string;
  license: string;
  text: string;
}

/**
 * The installed package a module id belongs to, as its root directory, or
 * undefined for the app's own files and virtual modules. The last
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

/** The specifiers of a stylesheet's `@import` rules. */
export function cssImports(css: string): string[] {
  const rule = /@import\s+(?:url\(\s*)?["']([^"']+)["']/g;
  return [...css.matchAll(rule)].map((match) => match[1] ?? "");
}

/**
 * Packages whose stylesheets an app stylesheet pulls in with `@import`,
 * following the app's own relative imports. Tailwind inlines these while it
 * compiles the CSS, so they never show up in the bundle's module graph.
 */
function stylesheetPackages(stylesheet: string, seen = new Set<string>()): Set<string> {
  const roots = new Set<string>();
  if (seen.has(stylesheet)) {
    return roots;
  }
  seen.add(stylesheet);
  for (const specifier of cssImports(readFileSync(stylesheet, "utf8"))) {
    // A URL (https:, data:) is fetched by the browser, not bundled.
    if (/^[a-z][a-z\d+.-]*:/i.test(specifier)) {
      continue;
    }
    if (/^[./]/.test(specifier)) {
      const local = resolve(dirname(stylesheet), specifier);
      if (existsSync(local) && packageRoot(local) === undefined) {
        for (const root of stylesheetPackages(local, seen)) {
          roots.add(root);
        }
      }
      continue;
    }
    const name = specifier
      .split("/")
      .slice(0, specifier.startsWith("@") ? 2 : 1)
      .join("/");
    const root = findPackage(dirname(stylesheet), name);
    if (root === undefined) {
      throw new Error(`${stylesheet} imports ${specifier}, but ${name} is not installed`);
    }
    roots.add(root);
  }
  return roots;
}

/** Node's lookup: node_modules/<name> in this directory or the nearest parent that has it. */
function findPackage(from: string, name: string): string | undefined {
  for (let dir = from; ; dir = dirname(dir)) {
    const candidate = join(dir, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) {
      return candidate;
    }
    if (dirname(dir) === dir) {
      return undefined;
    }
  }
}

export interface NoticeInputs {
  /** Ids of the modules in the shipped chunks. */
  moduleIds: Iterable<string>;
  /** The app's own stylesheets in the module graph. */
  stylesheets: Iterable<string>;
  publicDir: string;
  /** License texts for packages that publish none: package name to a file in `suppliedDir`. */
  supplied: Readonly<Record<string, string>>;
  suppliedDir: string;
}

/**
 * The notices file's content. Throws when a bundled package has no license
 * text, a supplied text is stale, or a font is not under the font license.
 */
export function collectNotices(inputs: NoticeInputs): string {
  const roots = new Set<string>();
  for (const id of inputs.moduleIds) {
    const root = packageRoot(id);
    if (root !== undefined) {
      roots.add(root);
    }
  }
  for (const stylesheet of inputs.stylesheets) {
    for (const root of stylesheetPackages(stylesheet)) {
      roots.add(root);
    }
  }

  const notices = new Map<string, Notice>();
  const missing: string[] = [];
  for (const root of roots) {
    const notice = noticeFor(root, inputs);
    if (notice === undefined) {
      missing.push(root);
    } else {
      notices.set(`${notice.name}@${notice.version}`, notice);
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `No license text in ${missing.join(", ")}. Copy it from the package's source ` +
        `repository into ${SUPPLIED_DIR}/ and list it in SUPPLIED.`,
    );
  }
  const names = new Set([...notices.values()].map((notice) => notice.name));
  const unused = Object.keys(inputs.supplied).filter((name) => !names.has(name));
  if (unused.length > 0) {
    throw new Error(`SUPPLIED lists packages the bundle no longer has: ${unused.join(", ")}`);
  }

  const sections = [...notices.values()]
    .toSorted((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
    .map((notice) => section(`${notice.name} ${notice.version}`, notice.license, notice.text));
  for (const font of bundledFonts(inputs.publicDir)) {
    const problem = fontProblem(inputs.publicDir, font);
    if (problem !== null) {
      throw new Error(`${font.dir}: ${problem}`);
    }
    const text = fontLicenseText(inputs.publicDir, font);
    sections.push(section(`Font files in ${font.dir}/`, FONT_LICENSE, text));
  }
  return [
    "Third-party notices",
    "",
    "Neon Workspace's web app includes the packages and fonts below, each under",
    "the license named with it. The text after each heading is the license and",
    "copyright notice that came with it.",
    "",
    ...sections,
  ].join("\n");
}

/**
 * The notice for one installed package: every license file it ships, or the
 * supplied copy when it ships none. Undefined when there is neither.
 */
function noticeFor(root: string, { supplied, suppliedDir }: NoticeInputs): Notice | undefined {
  const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as Record<
    string,
    unknown
  >;
  const name = String(manifest["name"]);
  const files = readdirSync(root)
    .filter((file) => LICENSE_FILE.test(file))
    .toSorted();
  const copy = supplied[name];
  if (copy !== undefined && files.length > 0) {
    throw new Error(`${name} now ships its own license; remove it from SUPPLIED`);
  }
  const texts =
    copy === undefined
      ? files.map((file) => readFileSync(join(root, file), "utf8").trim())
      : [readFileSync(join(suppliedDir, copy), "utf8").trim()];
  if (texts.length === 0) {
    return undefined;
  }
  return {
    name,
    version: String(manifest["version"]),
    license: declaredLicense(manifest) ?? "not declared",
    text: texts.join("\n\n"),
  };
}

function section(title: string, license: string, text: string): string {
  return [RULE, title, `License: ${license}`, THIN_RULE, "", text, ""].join("\n");
}

/** The Vite plugin: emits the notices next to the bundle in production builds. */
export function thirdPartyNotices(fileName = "THIRD_PARTY_NOTICES.txt"): Plugin {
  let publicDir = "";
  let suppliedDir = "";
  return {
    name: "neon:third-party-notices",
    apply: "build",
    configResolved(config) {
      publicDir = config.publicDir;
      suppliedDir = resolve(config.root, SUPPLIED_DIR);
    },
    generateBundle(_, bundle) {
      const moduleIds = Object.values(bundle).flatMap((output) =>
        output.type === "chunk" ? output.moduleIds : [],
      );
      const stylesheets = [...this.getModuleIds()]
        .map((id) => id.split("?")[0] ?? id)
        .filter((id) => id.endsWith(".css") && packageRoot(id) === undefined && existsSync(id));
      try {
        const source = collectNotices({
          moduleIds,
          stylesheets,
          publicDir,
          supplied: SUPPLIED,
          suppliedDir,
        });
        this.emitFile({ type: "asset", fileName, source });
      } catch (error) {
        this.error(error instanceof Error ? error.message : String(error));
      }
    },
  };
}
