import { defineConfig, devices } from "@playwright/test";

/** The port the built picker is previewed on; a run beside another can name its own. */
const port = Number(process.env.PICKER_PORT ?? 1421);

/**
 * The bundled picker, built into dist/ and served on its own under the app's policy, with a
 * fake of the app's Rust side (tests/fake-backend.ts). In Chromium and WebKit, the engines of
 * the app's webviews, at desktop and phone sizes.
 */
export default defineConfig({
  testDir: "tests",
  outputDir: "test-results",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  reporter: [["list"], ["html", { outputFolder: "playwright-report", open: "never" }]],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: {
    command: `bunx vite preview --port ${port}`,
    url: `http://127.0.0.1:${port}/`,
    reuseExistingServer: false,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
    { name: "android", use: { ...devices["Pixel 7"] } },
    { name: "iphone", use: { ...devices["iPhone 15"] } },
  ],
});
