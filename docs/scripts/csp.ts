// Writes csp.caddy, the docs' Content-Security-Policy for Caddy, after `astro build`.
// See lib/csp.ts for what the policy allows and why.
import { join } from 'node:path'

import { caddySnippet, policy, scanPage } from './lib/csp'

const dist = join(import.meta.dir, '..', 'dist')
const out = join(import.meta.dir, '..', 'csp.caddy')

const hashes = new Set<string>()
const problems: string[] = []
let pages = 0

for await (const file of new Bun.Glob('**/*.html').scan({ cwd: dist })) {
  const page = scanPage(await Bun.file(join(dist, file)).text())
  pages++

  for (const hash of page.hashes) hashes.add(hash)
  for (const handler of page.handlers) problems.push(`${file}: ${handler}`)
}

if (pages === 0) {
  console.error(`csp: no pages in ${dist}; run astro build first`)
  process.exit(1)
}

if (problems.length > 0) {
  console.error(`csp: event handler attributes cannot be allowed by hash:\n${problems.join('\n')}`)
  process.exit(1)
}

await Bun.write(out, caddySnippet(policy(hashes)))

console.log(`csp: ${hashes.size} inline scripts in ${pages} pages allowed by hash, written to csp.caddy`)
