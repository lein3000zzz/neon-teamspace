import { ADMIN, expect, test } from "./fixtures";
import { DYNAMIC_ISLAND, personOnPhone, phoneAddress, safeArea } from "./phone";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(phoneAddress(35, testInfo)),
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
