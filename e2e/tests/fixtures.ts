import { readFileSync } from "node:fs";
import { test as base, expect } from "@playwright/test";

export { expect };

/** The Content-Security-Policy web/Caddyfile sets for the app, word for word. */
export const APP_CSP =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; " +
  "font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; " +
  "frame-ancestors 'none'";

/** The version the server reports: the workspace's, in the repository's Cargo.toml. */
export const SERVER_VERSION = workspaceVersion();

function workspaceVersion(): string {
  const manifest = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
  const section = manifest.split(/^\[workspace\.package\]$/m)[1] ?? "";
  const version = /^version = "([^"]+)"$/m.exec(section)?.[1];
  if (version === undefined) {
    throw new Error("Cargo.toml has no version under [workspace.package]");
  }
  return version;
}

/** The server's answer to GET /api/v1/meta. */
export interface Meta {
  name: string;
  version: string;
}

/** The server's error shape. */
export interface ApiErrorBody {
  error: { code: string; message: string };
}

type Options = {
  /** Console errors a test expects, such as the one a 404 page logs for itself. */
  expectedConsoleError: RegExp | undefined;
};

/** A window with the binding the fixture below exposes. */
type Reporting = { __reportCspViolation(text: string): void };

/**
 * Every test fails on an error in the browser's console, an uncaught exception, or a
 * Content-Security-Policy violation. Chromium logs violations to the console as well; the
 * securitypolicyviolation event also catches those that are only reported.
 */
export const test = base.extend<Options & { browserProblems: void }>({
  expectedConsoleError: [undefined, { option: true }],

  browserProblems: [
    async ({ page, expectedConsoleError }, use) => {
      const problems: string[] = [];
      page.on("console", (message) => {
        const text = message.text();
        if (message.type() === "error" && !(expectedConsoleError?.test(text) ?? false)) {
          problems.push(`console: ${text}`);
        }
      });
      page.on("pageerror", (error) => problems.push(`uncaught: ${error.message}`));
      await page.exposeBinding("__reportCspViolation", (_source, text: string) => {
        problems.push(`CSP: ${text}`);
      });
      await page.addInitScript(() => {
        document.addEventListener(
          "securitypolicyviolation",
          (event) => {
            const blocked = event.blockedURI === "" ? "inline" : event.blockedURI;
            (window as unknown as Reporting).__reportCspViolation(
              `${event.effectiveDirective} blocked ${blocked} on ${event.documentURI}`,
            );
          },
          true,
        );
      });

      await use();

      expect(problems, "problems in the browser").toEqual([]);
    },
    { auto: true },
  ],
});
