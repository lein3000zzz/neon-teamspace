import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";

/** The script the iOS app adds to every page, as src-tauri/src/shell.rs includes it. */
const OPEN_IN_PLACE = readFileSync(
  new URL("../src-tauri/src/open-in-place.js", import.meta.url),
  "utf8",
);

const SERVER = "https://neon.example.test";
const ELSEWHERE = "https://elsewhere.example.test";

test.beforeEach(async ({ page }) => {
  for (const origin of [SERVER, ELSEWHERE]) {
    await page.route(`${origin}/**`, (route) =>
      route.fulfill({
        contentType: "text/html",
        body: `<!doctype html><title>${new URL(route.request().url()).pathname}</title>`,
      }),
    );
  }
  await page.addInitScript(OPEN_IN_PLACE);
  await page.goto(`${SERVER}/w/acme`);
});

test("window.open loads a page of the server in the window, as a file block's download does", async ({
  page,
}) => {
  const opened = await page.evaluate(() =>
    window.open("/api/v1/files/abc", "_blank", "noopener,noreferrer"),
  );
  expect(opened).toBeNull();
  await expect(page).toHaveURL(`${SERVER}/api/v1/files/abc`);
  expect(page.context().pages()).toHaveLength(1);
});

test("window.open of another origin navigates the window, for the origin lock to open outside", async ({
  page,
}) => {
  await page.evaluate((url) => window.open(url, "_blank"), `${ELSEWHERE}/docs`);
  await expect(page).toHaveURL(`${ELSEWHERE}/docs`);
  expect(page.context().pages()).toHaveLength(1);
});

test("window.open with no address, or one that runs script, opens nothing", async ({ page }) => {
  const results = await page.evaluate(() => [
    window.open(),
    window.open(""),
    window.open("about:blank"),
    window.open("javascript:document.title='ran'"),
    window.open("http://[not an address"),
  ]);
  expect(results).toEqual([null, null, null, null, null]);
  // The page cannot put window.open back.
  expect(
    await page.evaluate(() => {
      try {
        window.open = () => null;
      } catch {
        // Strict code throws on a read-only property; sloppy code ignores the assignment.
      }
      return window.open.toString().includes("location.assign");
    }),
  ).toBe(true);
  await page.evaluate(() => window.open("/next"));
  await expect(page).toHaveURL(`${SERVER}/next`);
  await expect(page).toHaveTitle("/next");
});
