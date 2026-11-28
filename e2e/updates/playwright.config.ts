import { defineConfig, devices } from "@playwright/test";

/**
 * The browser half of make updates-e2e: one test, against the stack run.ts starts on the
 * older release, which updates it to the newer one from the admin settings. It waits through
 * the restart, so its timeout is long; run.ts checks the stack itself afterwards.
 */
export default defineConfig({
  testDir: ".",
  testMatch: "update.spec.ts",
  outputDir: "../test-results/updates",
  forbidOnly: Boolean(process.env.CI),
  timeout: 15 * 60_000,
  reporter: [["list"], ["html", { outputFolder: "../playwright-report/updates", open: "never" }]],
  use: {
    baseURL: process.env.E2E_BASE_URL ?? "",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    ...devices["Desktop Chrome"],
  },
});
