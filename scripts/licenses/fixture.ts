import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

/**
 * A throwaway directory tree for one test, written from a map of relative
 * paths to contents. `onFinished` is the test runner's `onTestFinished`, from
 * bun:test or vitest, so the tree is removed when the test ends.
 */
export function fixture(
  files: Record<string, string>,
  onFinished: (cleanup: () => void) => void,
): string {
  const root = mkdtempSync(join(tmpdir(), "neon-licenses-"));
  onFinished(() => rmSync(root, { recursive: true, force: true }));
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  return root;
}

/** A package.json for a fixture package. */
export function manifest(name: string, license = "MIT", version = "1.0.0"): string {
  return JSON.stringify({ name, version, license });
}

/** Enough of the OFL's text for the font check to recognise it. */
export const OFL_TEXT = "SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007";
