import type { BrowserContext, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// A page opens its live connection, which collaboration.spec.ts tests; Chromium logs a socket
// that a page closes before it opened as failed.
test.use({
  extraHTTPHeaders: clientAddress(22),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };

/** The editable content of the page in view. */
function content(page: Page) {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox");
}

/** Signs a new account in on `page`, makes it a workspace and a page in General. */
async function workspaceWithPage(
  page: Page,
  browserContexts: BrowserContext[],
  newContext: () => Promise<BrowserContext>,
  name: string,
  title: string,
  markdown: string,
) {
  const run = String(Date.now());
  const owner = account(name, run);
  const adminContext = await newContext();
  browserContexts.push(adminContext);
  await makeAccounts(await adminContext.newPage(), [owner]);
  await page.goto("/login");
  await signIn(page, owner.email, PASSWORD);
  await expect(page.getByRole("complementary")).toBeVisible();
  const workspace = await callApi<Workspace>(
    page,
    "POST",
    "/api/v1/workspaces",
    { name: `${name} ${run}` },
    201,
  );
  const listed = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
  const { teamspaces } = (await listed.json()) as { teamspaces: Teamspace[] };
  const general = teamspaces.find((teamspace) => teamspace.access === "default");
  expect(general, "the workspace's General").toBeDefined();
  const made = await callApi<{ id: string }>(
    page,
    "POST",
    `/api/v1/workspaces/${workspace.id}/pages`,
    { teamspace_id: general?.id, title, markdown },
    201,
  );
  return { workspace, pageId: made.id };
}

test("search finds a page by a word of its text and by its title misspelled, and the quick search opens it", async ({
  page,
  browser,
}) => {
  test.setTimeout(90_000);
  const contexts: BrowserContext[] = [];
  try {
    const { workspace, pageId } = await workspaceWithPage(
      page,
      contexts,
      () => browser.newContext(),
      "searcher",
      "Quarterly roadmap",
      "The lighthouse budget is approved.",
    );

    // A word of the page's text, on the search page, marked in the snippet.
    await page.goto(`/w/${workspace.slug}/search?q=lighthouse`);
    const results = page.getByRole("region", { name: "Results" });
    const found = results.getByRole("link", { name: "Quarterly roadmap" });
    await expect(found).toBeVisible();
    await expect(results.locator("mark").filter({ hasText: "lighthouse" })).toBeVisible();

    // The title, misspelled, typed in the search page's field.
    const field = page.getByRole("searchbox", { name: "Search" });
    await field.fill("quaterly");
    await expect(page).toHaveURL(/q=quaterly/);
    await expect(results.getByRole("link", { name: "Quarterly roadmap" })).toBeVisible();

    // The quick search, from the keyboard, opens it.
    // The shortcut works once the sidebar knows the workspace, as its Search button shows.
    await page.goto(`/w/${workspace.slug}`);
    await expect(
      page.getByRole("complementary").getByRole("button", { name: /Search/ }),
    ).toBeVisible();
    await page.keyboard.press("ControlOrMeta+k");
    const quick = page.getByRole("dialog", { name: "Search" });
    await expect(quick.getByRole("searchbox", { name: "Search pages" })).toBeFocused();
    await page.keyboard.type("quaterly roadmp");
    await expect(
      quick.getByRole("region", { name: "Results" }).getByRole("button", {
        name: /Quarterly roadmap/,
      }),
    ).toBeVisible();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(new RegExp(`/w/${workspace.slug}/p/${pageId}$`));
    await expect(page.getByRole("textbox", { name: "Page title" })).toHaveValue(
      "Quarterly roadmap",
    );
    await expect(quick).toBeHidden();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});

test("restoring a version of a page puts it back in the open editor", async ({ page, browser }) => {
  test.setTimeout(90_000);
  const contexts: BrowserContext[] = [];
  try {
    const { workspace, pageId } = await workspaceWithPage(
      page,
      contexts,
      () => browser.newContext(),
      "historian",
      "Launch plan",
      "Kick-off in March",
    );
    await page.goto(`/w/${workspace.slug}/p/${pageId}`);
    await expect(page.locator("output").filter({ hasText: "Connected" })).toBeVisible();
    await expect(content(page)).toHaveAttribute("contenteditable", "true");
    await expect(content(page)).toContainText("Kick-off in March");

    // A paragraph typed after the page was made, which the server has once its markdown
    // shows it.
    await content(page).click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Moved to April");
    await expect
      .poll(async () => {
        const response = await page.request.get(`/api/v1/pages/${pageId}/markdown`);
        return ((await response.json()) as { markdown: string }).markdown;
      })
      .toContain("Moved to April");

    // The page's first version, compared with the page as it is, and seen as it was.
    await page.getByRole("button", { name: "History" }).click();
    const panel = page.getByRole("dialog", { name: "Page history" });
    const changes = panel.getByRole("list", { name: "Changes from this version to now" });
    await expect(changes.getByText("Moved to April")).toBeVisible();
    await panel.getByRole("tab", { name: "Preview" }).click();
    await expect(panel.getByRole("region", { name: "This version, read-only" })).toContainText(
      "Kick-off in March",
    );
    await expect(panel.getByRole("region", { name: "This version, read-only" })).not.toContainText(
      "Moved to April",
    );
    // Its text keeps clear of the box's sides.
    const inset = await panel
      .getByRole("region", { name: "This version, read-only" })
      .evaluate((box) => {
        const frame = box.getBoundingClientRect();
        const text = box.querySelector(".bn-inline-content")?.getBoundingClientRect();
        return text === undefined ? 0 : Math.min(text.left - frame.left, frame.right - text.right);
      });
    expect(inset, "room inside the preview's sides").toBeGreaterThanOrEqual(12);

    await panel.getByRole("button", { name: "Restore this version" }).click();
    await page.getByRole("alertdialog").getByRole("button", { name: "Restore" }).click();
    await expect(panel.getByText("Restored. The page shows this version again.")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(panel).toBeHidden();

    // The editor shows the version, through its live document.
    await expect(content(page)).not.toContainText("Moved to April");
    await expect(content(page)).toContainText("Kick-off in March");
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
