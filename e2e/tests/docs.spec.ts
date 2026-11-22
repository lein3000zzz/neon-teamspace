import { APP_CSP, expect, test } from "./fixtures";

test("the docs load beside the app under a policy of their own", async ({ page }) => {
  const response = await page.goto("/docs/");

  expect(response?.status()).toBe(200);
  await expect(page.getByRole("heading", { level: 1, name: "Neon Workspace" })).toBeVisible();
  // Their inline scripts are allowed by hash, search's WebAssembly by 'wasm-unsafe-eval', and
  // nothing else inline.
  const policy = response?.headers()["content-security-policy"] ?? "";
  expect(policy).not.toBe(APP_CSP);
  expect(policy).toMatch(/script-src 'self' 'wasm-unsafe-eval'( 'sha256-[A-Za-z0-9+/]+=*')+;/);
  expect(policy).not.toMatch(/script-src[^;]*'unsafe-inline'/);
  expect(policy).toContain("frame-ancestors 'none'");
});

test("/docs without its slash leads to the docs", async ({ page }) => {
  await page.goto("/docs");

  await expect(page).toHaveURL(/\/docs\/$/);
  await expect(page.getByRole("heading", { level: 1, name: "Neon Workspace" })).toBeVisible();
});

test("the API reference is generated into the docs", async ({ page }) => {
  expect((await page.goto("/docs/reference/api/"))?.status()).toBe(200);
  await expect(page.getByRole("heading", { level: 1, name: "Overview" })).toBeVisible();

  await page.goto("/docs/reference/api/operations/get_meta/");
  await expect(page.getByRole("heading", { level: 1, name: "Instance metadata" })).toBeVisible();
  await expect(page.getByText("/api/v1/meta").first()).toBeVisible();
});

test("pages with diagrams draw them as inline SVG", async ({ page }) => {
  await page.goto("/docs/architecture/overview/");

  await expect(page.locator(".sl-markdown-content svg").first()).toBeVisible();
});

test("search runs under the docs' policy", async ({ page }) => {
  await page.goto("/docs/");
  await page.getByRole("button", { name: "Search" }).first().click();
  await page.getByRole("dialog").getByPlaceholder("Search").fill("Compose");

  // Pagefind loads its index with WebAssembly, which the policy must allow.
  const result = page.locator(".pagefind-ui__result-link").first();
  await expect(result).toBeVisible();
  await result.click();
  await expect(page).toHaveURL(/\/docs\//);
});

test.describe("a page missing from the docs", () => {
  // Chromium logs the 404 of the page itself as an error.
  test.use({ expectedConsoleError: /status of 404/ });

  test("gets the docs' own not-found page with a 404", async ({ page }) => {
    const response = await page.goto("/docs/no-such-page/");

    expect(response?.status()).toBe(404);
    await expect(page.getByRole("heading", { level: 1, name: "Page not found" })).toBeVisible();
  });
});

test("the docs' third-party notices are served", async ({ request }) => {
  const notices = await request.get("/docs/THIRD_PARTY_NOTICES.txt");

  expect(notices.status()).toBe(200);
  expect(notices.headers()["content-type"]).toContain("text/plain");
  const text = await notices.text();
  expect(text).toContain("@astrojs/starlight");
  expect(text).toContain("pagefind");
  expect(text).toContain("License: MIT");
});
