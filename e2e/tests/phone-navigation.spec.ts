import { ADMIN, expect, test } from "./fixtures";
import { openDrawer, pageIn, personOnPhone, phoneAddress } from "./phone";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// Each page opened is left for the next; Chromium logs a socket that a page closes before it
// opened as failed.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(phoneAddress(33, testInfo)),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

test("on a phone, someone signs in, opens a page from the drawer, searches, and opens the inbox", async ({
  page,
  browser,
}) => {
  test.setTimeout(90_000);
  // Signed in through the form, on the phone.
  const { workspace } = await personOnPhone(browser, page, "walker");
  await pageIn(page, workspace, "Packing list", "Boots, a map and a flask.");
  await pageIn(page, workspace, "Route notes", "Turn left at the lighthouse.");
  await page.goto(`/w/${workspace.slug}`);

  // The sidebar is a drawer; a page in its tree opens, and the drawer goes.
  let drawer = await openDrawer(page);
  await drawer.getByRole("link", { name: "Packing list" }).tap();
  await expect(drawer).toBeHidden();
  await expect(page.getByRole("heading", { level: 1, name: "Packing list" })).toBeVisible();

  // Search, from the drawer, finds a page by a word of its text.
  drawer = await openDrawer(page);
  await drawer.getByRole("button", { name: "Search" }).tap();
  const search = page.getByRole("dialog", { name: "Search" });
  const field = search.getByRole("searchbox", { name: "Search pages" });
  await expect(field).toBeFocused();
  await field.fill("lighthouse");
  await search
    .getByRole("region", { name: "Results" })
    .getByRole("button", { name: /^Route notes/ })
    .tap();
  await expect(search).toBeHidden();
  await expect(page.getByRole("heading", { level: 1, name: "Route notes" })).toBeVisible();

  // The inbox, from the drawer, with nothing in it yet.
  drawer = await openDrawer(page);
  await drawer.getByRole("link", { name: "Inbox" }).tap();
  await expect(page.getByRole("heading", { level: 1, name: "Inbox" })).toBeVisible();
  await expect(page.getByText("Nothing unread.")).toBeVisible();
});
