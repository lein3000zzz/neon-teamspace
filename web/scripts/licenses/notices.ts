/**
 * Writes THIRD_PARTY_NOTICES.txt into the production build: the license text
 * of every package whose code ends up in the shipped files, of the code copied
 * into the app from other projects, and of every font served from public/.
 * MIT, ISC and BSD ask for their notice in every copy and Apache-2.0 for a
 * copy of the license, while the minifier drops the comments that carried
 * them. MPL-2.0 also asks to say where its packages' source is.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { Plugin } from "vite";
import {
  bundledFonts,
  FONT_LICENSE,
  fontLicenseText,
  fontProblem,
} from "../../../scripts/licenses/fonts.ts";
import {
  type Notice,
  noticesFile,
  packageNotice,
  packageRoot,
  packageSections,
  section,
} from "../../../scripts/licenses/notices.ts";

export { packageRoot };

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

/** Code another project wrote, copied into the app rather than installed as a package. */
export interface CopiedCode {
  /** The project, as the notices name it. */
  name: string;
  /** Where the code is, relative to the web app's root. */
  dir: string;
  license: string;
  /** The project's license text, copied from its repository, relative to the web app's root. */
  file: string;
}

/**
 * Copied code, which is in the bundle whatever its modules' paths say, so its
 * notice always ships.
 */
export const COPIED: readonly CopiedCode[] = [
  // shadcn/ui's command line copies its components into the app, which then changes them.
  // The text is LICENSE.md of github.com/shadcn-ui/ui.
  {
    name: "shadcn/ui",
    dir: "src/components/ui",
    license: "MIT",
    file: "src/components/ui/shadcn-ui.LICENSE",
  },
];

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
  /** The web app's root, which `copied` names its files from. */
  root: string;
  /** Ids of the modules in the shipped chunks. */
  moduleIds: Iterable<string>;
  /** The app's own stylesheets in the module graph. */
  stylesheets: Iterable<string>;
  publicDir: string;
  /** License texts for packages that publish none: package name to a file in `suppliedDir`. */
  supplied: Readonly<Record<string, string>>;
  suppliedDir: string;
  copied: readonly CopiedCode[];
}

/**
 * The notices file's content. Throws when a bundled package has no license
 * text, a supplied text is stale, a copied project's text is missing, or a
 * font is not under the font license.
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

  const sections = packageSections(notices.values());
  for (const code of inputs.copied) {
    const file = join(inputs.root, code.file);
    if (!existsSync(file)) {
      throw new Error(
        `${code.dir}: ${code.file} is missing: copy ${code.name}'s license from its repository`,
      );
    }
    const text = readFileSync(file, "utf8").trim();
    sections.push(section(`${code.name}, the code copied into ${code.dir}/`, code.license, text));
  }
  for (const font of bundledFonts(inputs.publicDir)) {
    const problem = fontProblem(inputs.publicDir, font);
    if (problem !== null) {
      throw new Error(`${font.dir}: ${problem}`);
    }
    const text = fontLicenseText(inputs.publicDir, font);
    sections.push(section(`Font files in ${font.dir}/`, FONT_LICENSE, text));
  }
  return noticesFile(
    [
      "Neon Teamspace's web app includes the packages, code and fonts below, each",
      "under the license named with it. The text after each heading is the license",
      "and copyright notice that came with it.",
      "",
      "The source code of each package under MPL-2.0 can be had from the npm",
      "registry, at the address its section gives under Source, which serves the",
      "package at the version listed, and from the repository its section names",
      "under Repository.",
    ],
    sections,
  );
}

/**
 * The notice for one installed package: every license file it ships, or the
 * supplied copy when it ships none. Undefined when there is neither.
 */
function noticeFor(root: string, { supplied, suppliedDir }: NoticeInputs): Notice | undefined {
  const texts = Object.fromEntries(
    Object.entries(supplied).map(([name, file]) => [
      name,
      () => [readFileSync(join(suppliedDir, file), "utf8")],
    ]),
  );
  return packageNotice(root, texts);
}

/** The Vite plugin: emits the notices next to the bundle in production builds. */
export function thirdPartyNotices(fileName = "THIRD_PARTY_NOTICES.txt"): Plugin {
  let root = "";
  let publicDir = "";
  let suppliedDir = "";
  return {
    name: "neon:third-party-notices",
    apply: "build",
    configResolved(config) {
      root = config.root;
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
          root,
          moduleIds,
          stylesheets,
          publicDir,
          supplied: SUPPLIED,
          suppliedDir,
          copied: COPIED,
        });
        this.emitFile({ type: "asset", fileName, source });
      } catch (error) {
        this.error(error instanceof Error ? error.message : String(error));
      }
    },
  };
}
