import type { Page } from "@playwright/test";
import { callApi } from "./accounts";
import { ADMIN, expect, test } from "./fixtures";
import { generalOf, personOnPhone, phoneAddress, type Workspace } from "./phone";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// A row opened from the board is left for the board again; Chromium logs a socket that a page
// closes before it opened as failed.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(phoneAddress(34, testInfo)),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

/** A database called Sprint in the workspace's General, with a row of each title, not started. */
async function sprint(page: Page, workspace: Workspace, titles: string[]) {
  const database = await callApi<{ page: { id: string } }>(
    page,
    "POST",
    `/api/v1/workspaces/${workspace.id}/databases`,
    { teamspace_id: await generalOf(page, workspace), title: "Sprint" },
    201,
  );
  const api = `/api/v1/databases/${database.page.id}`;
  for (const title of titles) {
    await callApi(
      page,
      "POST",
      `${api}/rows`,
      { values: { Name: title, Status: "Not started" } },
      201,
    );
  }
  return `/w/${workspace.slug}/p/${database.page.id}`;
}

/**
 * Drags a finger from the middle of `from` by `dx` pixels across, through the browser's own
 * touch input, as Playwright's touchscreen only taps.
 */
async function swipe(page: Page, from: { x: number; y: number }, dx: number) {
  const session = await page.context().newCDPSession(page);
  const at = (x: number) => [{ x: Math.round(x), y: Math.round(from.y) }];
  await session.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: at(from.x) });
  for (let step = 1; step <= 6; step += 1) {
    await session.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: at(from.x + (dx * step) / 6),
    });
  }
  await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await session.detach();
}

test("on a phone, a board's card moves from its menu and opens to be edited", async ({
  page,
  browser,
}) => {
  test.setTimeout(90_000);
  const { workspace } = await personOnPhone(browser, page, "organiser");
  const path = await sprint(page, workspace, ["Write the demo script", "Book the venue"]);
  await page.goto(path);
  const views = page.getByRole("region", { name: "Database Sprint" });
  await views.getByRole("tab", { name: "Board" }).tap();
  const column = (name: string) => views.getByRole("region", { name });
  const card = column("Not started").getByRole("article", { name: "Write the demo script" });
  await expect(card).toBeVisible();

  // The card's menu, always in sight on a touch screen, moves it on and down its column.
  await card.getByRole("button", { name: "More for Write the demo script" }).tap();
  await page.getByRole("menuitem", { name: "Move to In progress" }).tap();
  await expect(
    column("In progress").getByRole("article", { name: "Write the demo script" }),
  ).toBeVisible();
  await expect(column("Not started").getByRole("article")).toHaveCount(1);

  // The card opens as a page of its own, whose status and title change there.
  await column("In progress")
    .getByRole("button", { name: "Write the demo script", exact: true })
    .tap();
  const properties = page.getByRole("region", { name: "Properties" });
  await properties.getByRole("button", { name: /^Status of Write the demo script/ }).tap();
  await page
    .getByRole("list", { name: "Status of Write the demo script" })
    .getByRole("button", { name: "Done" })
    .tap();
  await expect(
    properties.getByRole("button", { name: /^Status of Write the demo script.*Done$/ }),
  ).toBeVisible();
  const title = page.getByRole("textbox", { name: "Page title" });
  await title.tap();
  await title.fill("Write and rehearse the demo");
  await title.press("Enter");

  // Back on the board, the card is in its new column, by its new title.
  await page.goBack();
  await expect(
    column("Done").getByRole("article", { name: "Write and rehearse the demo" }),
  ).toBeVisible();
});

test("on a phone, a table's column widens from a grip at its edge", async ({ page, browser }) => {
  test.setTimeout(60_000);
  const { workspace } = await personOnPhone(browser, page, "tabulator");
  const path = await sprint(page, workspace, ["Write the demo script"]);
  await page.goto(path);
  const views = page.getByRole("region", { name: "Database Sprint" });
  await expect(views.getByRole("tab", { name: "Table", selected: true })).toBeVisible();

  // The slider at the edge is for the keyboard; a finger takes the grip over it.
  const heading = views.locator("th").filter({ hasText: "Status" });
  const grip = heading.locator("[data-column-grip]");
  await expect(grip).toBeVisible();
  // The table scrolls sideways on a phone: the grip into sight first.
  await grip.scrollIntoViewIfNeeded();
  const before = (await heading.boundingBox())?.width ?? 0;
  const box = await grip.boundingBox();
  await swipe(page, { x: (box?.x ?? 0) + (box?.width ?? 0) / 2, y: (box?.y ?? 0) + 10 }, 60);
  await expect
    .poll(async () => (await heading.boundingBox())?.width ?? 0)
    .toBeGreaterThan(before + 50);

  // The width is the view's, as the server keeps it.
  await page.reload();
  await expect
    .poll(
      async () =>
        (await views.locator("th").filter({ hasText: "Status" }).boundingBox())?.width ?? 0,
    )
    .toBeGreaterThan(before + 50);
});
