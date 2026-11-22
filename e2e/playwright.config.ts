import { defineConfig, devices } from "@playwright/test";

/**
 * The end-to-end tests drive a running stack through its web origin, as a browser does.
 * `make e2e` starts a throwaway one, built from the checkout, and points them at it with
 * E2E_BASE_URL. Unset, they run against the stack `make up` serves.
 */
export default defineConfig({
  testDir: "tests",
  outputDir: "test-results",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  reporter: [["list"], ["html", { outputFolder: "playwright-report", open: "never" }]],
  use: {
    baseURL: process.env.E2E_BASE_URL ?? "http://127.0.0.1:3000",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
