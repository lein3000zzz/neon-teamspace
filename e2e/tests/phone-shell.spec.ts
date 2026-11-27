import { ADMIN, expect, test } from "./fixtures";
import {
  DYNAMIC_ISLAND,
  openDrawer,
  pageIn,
  panelsInHistory,
  personOnPhone,
  phoneAddress,
  safeArea,
} from "./phone";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// Each page opened is left for the next; Chromium logs a socket that a page closes before it
// opened as failed.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(phoneAddress(35, testInfo)),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

test("under an iPhone's status bar, the skip link stays out of sight until the keyboard reaches it", async ({
  page,
  browser,
}) => {
  const { workspace } = await personOnPhone(browser, page, "skipper");
  await safeArea(page, DYNAMIC_ISLAND);
  await page.goto(`/w/${workspace.slug}`);
  const menu = page.getByRole("button", { name: "Open navigation" });
  await expect(menu).toBeVisible();
  // The page keeps clear of the status bar, so the inset is in force.
  expect((await menu.boundingBox())?.y ?? 0).toBeGreaterThanOrEqual(DYNAMIC_ISLAND.top);

  // Wholly above the screen: not an edge of it shows under the status bar.
  const skip = page.getByRole("link", { name: "Skip to content" });
  const hidden = await skip.boundingBox();
  expect(hidden, "the skip link's box").not.toBeNull();
  expect((hidden?.y ?? 0) + (hidden?.height ?? 0)).toBeLessThanOrEqual(0);

  // The keyboard brings it down, below the status bar, and it leads to the page.
  await page.keyboard.press("Tab");
  await expect(skip).toBeFocused();
  const shown = await skip.boundingBox();
  expect(shown?.y ?? 0).toBeGreaterThanOrEqual(DYNAMIC_ISLAND.top);
  expect((shown?.y ?? 0) + (shown?.height ?? 0)).toBeLessThanOrEqual(
    page.viewportSize()?.height ?? 0,
  );
  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();
});

test("on a phone, back closes the comments, the drawer and a dialog over it before it leaves the page", async ({
  page,
  browser,
}) => {
  test.setTimeout(90_000);
  const { workspace } = await personOnPhone(browser, page, "returner");
  const sprint = await pageIn(page, workspace, "Sprint", "The sprint's goals.");
  const plan = await pageIn(page, workspace, "Launch plan", "Ship it on Sunday.");
  const sprintTitle = page.getByRole("heading", { level: 1, name: "Sprint" });
  const planTitle = page.getByRole("heading", { level: 1, name: "Launch plan" });
  await page.goto(sprint.path);
  await expect(sprintTitle).toBeVisible();
  await page.goto(plan.path);
  await expect(planTitle).toBeVisible();

  // The comments, over the whole screen of a phone: back closes them, on the same page.
  await page.getByRole("button", { name: /^Comments/ }).tap();
  const comments = page.getByRole("complementary", { name: "Comments" });
  await expect(comments).toBeVisible();
  await page.goBack();
  await expect(comments).toBeHidden();
  await expect(planTitle).toBeVisible();
  expect(new URL(page.url()).pathname).toBe(plan.path);

  // The drawer, and a dialog opened from it: back closes the dialog, then the drawer.
  const drawer = await openDrawer(page);
  await drawer.getByRole("button", { name: "More for Launch plan" }).tap();
  await page.getByRole("menuitem", { name: "Move to…" }).tap();
  const move = page.getByRole("dialog", { name: "Move Launch plan" });
  await expect(move).toBeVisible();
  await page.goBack();
  await expect(move).toBeHidden();
  await expect(drawer).toBeVisible();
  await page.goBack();
  await expect(drawer).toBeHidden();
  await expect(planTitle).toBeVisible();

  // A page opened from the drawer takes the drawer's place: back returns to the page under it.
  await (await openDrawer(page)).getByRole("link", { name: "Sprint" }).tap();
  await expect(sprintTitle).toBeVisible();
  await page.goBack();
  await expect(planTitle).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Closed with its own button, the drawer takes its entry back with it: back leaves the page.
  const again = await openDrawer(page);
  await expect.poll(() => panelsInHistory(page)).toBe(1);
  await again.getByRole("button", { name: "Close navigation" }).tap();
  await expect(again).toBeHidden();
  await expect.poll(() => panelsInHistory(page)).toBe(0);
  await page.goBack();
  await expect(sprintTitle).toBeVisible();
});
