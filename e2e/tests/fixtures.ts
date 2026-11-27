import { readFileSync } from "node:fs";
import {
  test as base,
  expect,
  type ConsoleMessage,
  type Locator,
  type Page,
} from "@playwright/test";

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
  product: string;
  name: string;
  version: string;
}

/** The server's error shape. */
export interface ApiErrorBody {
  error: { code: string; message: string };
}

/**
 * The first admin of the stack, whom make e2e creates with a fresh password; undefined
 * against a stack started another way, where the tests that need one are skipped.
 */
export const ADMIN = admin();

function admin(): { email: string; password: string } | undefined {
  const email = process.env.E2E_ADMIN_EMAIL;
  const password = process.env.E2E_ADMIN_PASSWORD;
  return email && password ? { email, password } : undefined;
}

/**
 * Shows more of the page's list with its Load more button until `item` is in it. The admin's
 * lists start with the oldest, so what a test made comes past the first page once the runs
 * before it filled that.
 */
export async function loadMoreUntil(page: Page, item: Locator) {
  const items = page.getByRole("main").getByRole("listitem");
  const more = page.getByRole("button", { name: "Load more" });
  // The first page is in: it holds the item, or there is more.
  await expect(item.or(more).first()).toBeVisible();
  while (!(await item.isVisible())) {
    const shown = await items.count();
    await more.click();
    await expect
      .poll(() => items.count(), { message: "the next page of the list" })
      .toBeGreaterThan(shown);
  }
}

/** Signs in through the form, and waits for the page it leads to. */
export async function signIn(page: Page, email: string, password: string) {
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("button", { name: "Sign in" })).toBeHidden();
}

/**
 * The app asks the server who is signed in as it opens, and a visitor without a session
 * gets the 401 that says nobody is, which Chromium logs as a failed load. That one answer
 * is how signed-out pages work, not a problem.
 */
function signedOutCheck(message: ConsoleMessage): boolean {
  return (
    message.text().includes("status of 401") &&
    new URL(message.location().url, "http://stack.invalid").pathname === "/api/v1/account/me"
  );
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
        if (
          message.type() === "error" &&
          !signedOutCheck(message) &&
          !(expectedConsoleError?.test(text) ?? false)
        ) {
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
