// Writes THIRD_PARTY_NOTICES.txt into the built docs: the license text of every package whose
// code or styles reach dist/. MIT, the license of each of them so far, asks for its notice in
// every copy, and the minifier drops the comments that carried it.
//
// A Vite plugin finds the packages with modules in the browser's scripts or in the stylesheets.
// OUTSIDE_VITE names the ones whose files reach dist/ another way. The build fails when a
// shipped package has no license text, or when a list here names a package it no longer needs.
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import type { AstroIntegration } from 'astro'
import type { Plugin } from 'vite'

import {
  type Notice,
  noticesFile,
  packageNotice,
  packageRoot,
  packageSections,
  type Supplied,
} from '../../../scripts/licenses/notices.ts'

export const FILE_NAME = 'THIRD_PARTY_NOTICES.txt'

/** Packages whose files reach dist/ without passing through Vite, and where they go. */
export const OUTSIDE_VITE: Readonly<Record<string, string>> = {
  // Starlight runs it after the build, and it writes the search's scripts, styles and
  // WebAssembly into pagefind/.
  pagefind: 'pagefind/',
  // The code blocks' scripts and styles, which the integration writes as ec.*.js and ec.*.css.
  'astro-expressive-code': '_astro/ec.*',
  '@expressive-code/core': '_astro/ec.*',
  '@expressive-code/plugin-frames': '_astro/ec.*',
  '@expressive-code/plugin-shiki': '_astro/ec.*',
  '@expressive-code/plugin-text-markers': '_astro/ec.*',
}

/** A license text the package does not publish, copied from its source repository. */
interface SuppliedText {
  /** The one version it covers: a new one is checked again. */
  version: string
  /** Files in scripts/licenses/. */
  files: readonly string[]
}

const SUPPLIED: Readonly<Record<string, SuppliedText>> = {
  // Pagefind publishes its license in the pagefind package only. ui-core.js has Svelte's
  // runtime compiled in, from Svelte 4.2.20, as Pagefind 1.6.0 builds it.
  '@pagefind/default-ui': { version: '1.6.0', files: ['pagefind.LICENSE', 'svelte.LICENSE'] },
}

export interface NoticeInputs {
  /** The docs' root, whose node_modules holds the packages. */
  root: string
  /** The root directories of the packages Vite found in the shipped files. */
  found: Iterable<string>
  outsideVite?: Readonly<Record<string, string>>
  supplied?: Readonly<Record<string, SuppliedText>>
  /** Where the supplied files are. */
  suppliedDir?: string
}

/** The notices file's content. Throws when a package has no license text, or a list is stale. */
export function collectNotices({
  root,
  found,
  outsideVite = OUTSIDE_VITE,
  supplied = SUPPLIED,
  suppliedDir = join(root, 'scripts', 'licenses'),
}: NoticeInputs): string {
  const roots = new Set(found)
  for (const name of Object.keys(outsideVite)) {
    const packageDir = join(root, 'node_modules', name)
    if (!existsSync(join(packageDir, 'package.json'))) {
      throw new Error(`OUTSIDE_VITE lists ${name}, which is not installed: remove it`)
    }
    roots.add(packageDir)
  }

  const texts: Supplied = Object.fromEntries(
    Object.entries(supplied).map(([name, { files }]) => [
      name,
      () => files.map((file) => readFileSync(join(suppliedDir, file), 'utf8')),
    ]),
  )
  const notices = new Map<string, Notice>()
  const missing: string[] = []
  for (const packageDir of roots) {
    const notice = packageNotice(packageDir, texts)
    if (notice === undefined) {
      missing.push(packageDir)
      continue
    }
    const copy = supplied[notice.name]
    if (copy !== undefined && copy.version !== notice.version) {
      throw new Error(
        `SUPPLIED covers ${notice.name} ${copy.version}, but ${notice.version} is installed: ` +
          'check its license texts again and update the entry',
      )
    }
    notices.set(`${notice.name}@${notice.version}`, notice)
  }
  if (missing.length > 0) {
    throw new Error(
      `No license text in ${missing.join(', ')}. Copy it from the package's source repository ` +
        'into scripts/licenses/ and list it in SUPPLIED.',
    )
  }
  const names = new Set([...notices.values()].map((notice) => notice.name))
  const unused = Object.keys(supplied).filter((name) => !names.has(name))
  if (unused.length > 0) {
    throw new Error(`SUPPLIED lists packages the docs no longer ship: ${unused.join(', ')}`)
  }

  return noticesFile(
    [
      "Neon Workspace's documentation includes code and styles from the packages below,",
      'each under the license named with it. The text after each heading is the license',
      'and copyright notice that came with it.',
    ],
    packageSections(notices.values()),
  )
}

/** A stylesheet's module: a CSS file, or the styles of an .astro component. */
const STYLE = /\.css(?:$|\?)|[?&]type=style\b/

/** Records the packages with modules in the browser's chunks, or in any stylesheet. */
function shippedPackages(found: Set<string>): Plugin {
  const add = (id: string) => {
    const packageDir = packageRoot(id)
    if (packageDir !== undefined) found.add(packageDir)
  }
  return {
    name: 'neon:shipped-packages',
    apply: 'build',
    generateBundle(_, bundle) {
      // Only the client environment's chunks run in the browser; the others render the pages.
      if (this.environment.name === 'client') {
        for (const output of Object.values(bundle)) {
          if (output.type === 'chunk') output.moduleIds.forEach(add)
        }
      }
      // Astro gathers the stylesheets of every environment into the shipped CSS.
      for (const id of this.getModuleIds()) {
        if (STYLE.test(id)) add(id)
      }
    },
  }
}

/** The integration: finds the shipped packages while the docs build, then writes the notices. */
export function thirdPartyNotices(): AstroIntegration {
  const found = new Set<string>()
  let root = ''
  return {
    name: 'neon:third-party-notices',
    hooks: {
      'astro:config:setup': ({ config, updateConfig }) => {
        root = fileURLToPath(config.root)
        updateConfig({ vite: { plugins: [shippedPackages(found)] } })
      },
      'astro:build:done': ({ dir }) => {
        writeFileSync(join(fileURLToPath(dir), FILE_NAME), collectNotices({ root, found }))
      },
    },
  }
}
