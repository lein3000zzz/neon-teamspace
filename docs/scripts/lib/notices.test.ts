import { describe, expect, onTestFinished, test } from 'bun:test'
import { join } from 'node:path'

import { fixture, manifest } from '../../../scripts/licenses/fixture.ts'
import { collectNotices } from './notices'

/** Docs with a package Vite finds, one that reaches dist/ another way, and one without a text. */
const docs = (files: Record<string, string> = {}) =>
  fixture(
    {
      'node_modules/theme/package.json': manifest('theme'),
      'node_modules/theme/LICENSE': 'theme license',
      'node_modules/search/package.json': manifest('search', 'MIT', '2.0.0'),
      // A directory of license files, as some packages have.
      'node_modules/search/LICENSE/LICENSE': 'search license',
      'node_modules/search/LICENSE/LICENSE-bundled': 'bundled license',
      'node_modules/bare/package.json': manifest('bare'),
      'scripts/licenses/bare.LICENSE': 'bare license from its repository',
      ...files,
    },
    onTestFinished,
  )

const inputs = (root: string) => ({
  root,
  found: [join(root, 'node_modules/theme')],
  outsideVite: { search: 'search/' },
  supplied: {},
})

describe('collectNotices', () => {
  test('lists the packages Vite found and those outside it, with their license texts', () => {
    const text = collectNotices(inputs(docs()))

    expect(text).toContain('theme 1.0.0\nLicense: MIT')
    expect(text).toContain('theme license')
    expect(text).toContain('search 2.0.0\nLicense: MIT')
    expect(text).toContain('search license\n\nbundled license')
    expect(text).not.toContain('bare')
  })

  test('uses the supplied texts for a package that publishes none', () => {
    const root = docs()
    const text = collectNotices({
      ...inputs(root),
      found: [join(root, 'node_modules/bare')],
      supplied: { bare: { version: '1.0.0', files: ['bare.LICENSE'] } },
    })

    expect(text).toContain('bare 1.0.0\nLicense: MIT')
    expect(text).toContain('bare license from its repository')
  })

  test('fails on a shipped package without license text', () => {
    const root = docs()

    expect(() => collectNotices({ ...inputs(root), found: [join(root, 'node_modules/bare')] })).toThrow(
      'No license text in',
    )
  })

  test('fails on a supplied text for another version', () => {
    const root = docs()

    expect(() =>
      collectNotices({
        ...inputs(root),
        found: [join(root, 'node_modules/bare')],
        supplied: { bare: { version: '0.9.0', files: ['bare.LICENSE'] } },
      }),
    ).toThrow('SUPPLIED covers bare 0.9.0, but 1.0.0 is installed')
  })

  test('fails on list entries the docs no longer need', () => {
    const root = docs()

    expect(() =>
      collectNotices({ ...inputs(root), supplied: { gone: { version: '1.0.0', files: ['gone.LICENSE'] } } }),
    ).toThrow('no longer ship: gone')
    expect(() =>
      collectNotices({ ...inputs(root), supplied: { theme: { version: '1.0.0', files: ['bare.LICENSE'] } } }),
    ).toThrow('theme now ships its own license')
    expect(() => collectNotices({ ...inputs(root), outsideVite: { gone: 'gone/' } })).toThrow(
      'OUTSIDE_VITE lists gone, which is not installed',
    )
  })
})
