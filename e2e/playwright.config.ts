import { defineConfig, devices } from "@playwright/test";

/**
 * The specs that change settings of the whole instance, such as who may sign up, and those
 * that read them. They run one at a time, so each sees the settings it set or the defaults;
 * the others change none of them, and run side by side with these.
 */
const INSTANCE_SPECS = ["**/app.spec.ts", "**/auth.spec.ts", "**/registration.spec.ts"];

/**
 * The specs that use the app on a phone, in each of the phones below: a touch screen, which
 * the app edits on with its toolbar above the keyboard rather than with a mouse.
 */
const PHONE_SPECS = ["**/phone-*.spec.ts"];

// What tells this run of the suite from those before it, for the client addresses of
// tests/accounts.ts: a group of an IPv6 address, from the seconds since the epoch. The runner
// sets it before it starts the workers, which then find it in their environment.
process.env.E2E_SUITE_RUN ??= (Math.floor(Date.now() / 1000) % 0x10000).toString(16);

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
  projects: [
    {
      name: "chromium",
      testIgnore: [...INSTANCE_SPECS, ...PHONE_SPECS],
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "instance settings",
      testMatch: INSTANCE_SPECS,
      workers: 1,
      use: { ...devices["Desktop Chrome"] },
    },
    // An iPhone's screen, its scale, touch and its browser's name, in Chromium, the one browser
    // the suite installs; and an Android phone's, whose browser is Chromium. Their names are
    // tests/phone.ts's PHONES.
    {
      name: "iphone",
      testMatch: PHONE_SPECS,
      use: { ...devices["iPhone 15"], browserName: "chromium" },
    },
    { name: "android", testMatch: PHONE_SPECS, use: { ...devices["Pixel 7"] } },
  ],
});
