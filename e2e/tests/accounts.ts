import { expect, test, type Browser, type BrowserContext, type Page } from "@playwright/test";
import { ADMIN, signIn } from "./fixtures";

/** The password of every account the tests make. */
export const PASSWORD = "a passphrase long enough";

export type Account = { email: string; name: string };

/** The first admin, which a spec's skip makes sure of. */
function admin() {
  if (ADMIN === undefined) {
    throw new Error("E2E_ADMIN_EMAIL and E2E_ADMIN_PASSWORD are unset");
  }
  return ADMIN;
}

/**
 * What tells this run of a test from the others against one stack: the time, and which
 * repetition it is when `--repeat-each` runs it side by side with itself.
 */
export function runOf(): string {
  const { repeatEachIndex } = test.info();
  return repeatEachIndex === 0 ? String(Date.now()) : `${Date.now()}-${repeatEachIndex}`;
}

/** An account of this run, so runs against one stack never meet. */
export function account(role: string, run: string): Account {
  return {
    email: `${role}-${run}@e2e.example`,
    name: `${role[0]?.toUpperCase() ?? ""}${role.slice(1)} ${run}`,
  };
}

/**
 * The address a spec's browsers say they come from, as X-Forwarded-For, which the e2e stack
 * believes. The server allows ten sign-ins at once from one address, and the specs run side
 * by side, so a spec that signs several people in takes an address of its own, from the
 * ranges kept for documentation; and so does each `repetition` of it past the first, as
 * `--repeat-each` runs them, to show a test is stable.
 */
export function clientAddress(spec: number, repetition = 0): Record<string, string> {
  return {
    "X-Forwarded-For": repetition === 0 ? `203.0.113.${spec}` : `2001:db8::${spec}:${repetition}`,
  };
}

/** Signs the first admin in on `page`, and makes each account through the admin API. */
export async function makeAccounts(page: Page, list: Account[]) {
  const { email, password } = admin();
  await page.goto("/login");
  await signIn(page, email, password);
  await expect(page.getByRole("complementary")).toBeVisible();
  for (const entry of list) {
    const response = await page.request.post("/api/v1/admin/users", {
      headers: { Origin: originOf(page) },
      data: { email: entry.email, password: PASSWORD, display_name: entry.name },
    });
    expect(response.status(), entry.email).toBe(201);
  }
}

/** A browser of its own for `account`, signed in. */
export async function signedIn(browser: Browser, entry: Account): Promise<[BrowserContext, Page]> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto("/login");
  await signIn(page, entry.email, PASSWORD);
  // The shell shows once the session is there.
  await expect(page.getByRole("complementary")).toBeVisible();
  return [context, page];
}

/**
 * The origin of the page's stack. The server checks that a change comes from its own
 * origin, as a browser's would, so requests made beside the page carry it.
 */
function originOf(page: Page): string {
  return new URL(page.url()).origin;
}

/** A call to the API as the account signed in on `page`, which must answer `status`. */
export async function callApi<T>(
  page: Page,
  method: "POST" | "PATCH",
  path: string,
  data: unknown,
  status: number,
): Promise<T> {
  const response = await page.request.fetch(path, {
    method,
    headers: { Origin: originOf(page) },
    data,
  });
  expect(response.status(), `${method} ${path}`).toBe(status);
  return (await response.json()) as T;
}
