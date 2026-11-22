import { readFile } from "node:fs/promises";
import type { BrowserContext, Download, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// A page opens its live connection, which Chromium logs as failed when the page goes before
// it opened, as pages.spec.ts says.
test.use({
  extraHTTPHeaders: clientAddress(13),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; name: string; access: string };

/** The sidebar, with the page trees. */
function sidebar(page: Page) {
  return page.getByRole("complementary");
}

/** Picks an item of the page's own menu, in its header, and waits for what it saves. */
async function saved(page: Page, item: string): Promise<Download> {
  await expect(page.getByRole("menu")).toHaveCount(0);
  await page
    .getByRole("main")
    .getByRole("button", { name: /^More for / })
    .click();
  const download = page.waitForEvent("download");
  await page.getByRole("menuitem", { name: item }).click();
  return download;
}

/** Imports a file through the dialog that `open` brings up, and closes it once it is done. */
async function importFile(page: Page, dialogName: string, name: string, buffer: Buffer) {
  const dialog = page.getByRole("dialog", { name: dialogName });
  await dialog.getByLabel("File").setInputFiles({ name, mimeType: mimeOf(name), buffer });
  await dialog.getByRole("button", { name: "Import" }).click();
  await expect(dialog.getByText(/^Imported \d+ pages?/)).toBeVisible();
  return dialog;
}

function mimeOf(name: string) {
  return name.endsWith(".zip") ? "application/zip" : "text/markdown";
}

test("pages go out as markdown and come back in as pages", async ({ page, browser }) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("porter", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner]);

    await page.goto("/login");
    await signIn(page, owner.email, PASSWORD);
    await expect(sidebar(page)).toBeVisible();
    const workspace = await callApi<Workspace>(
      page,
      "POST",
      "/api/v1/workspaces",
      { name: `Transfer ${run}` },
      201,
    );
    const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    expect(general, "the workspace's General").toBeDefined();
    const roadmap = await callApi<{ id: string }>(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Roadmap" },
      201,
    );
    await callApi(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { parent_id: roadmap.id, title: "Q1" },
      201,
    );

    // Roadmap gets a line, typed in its editor; an export reads the live document.
    await page.goto(`/w/${workspace.slug}/p/${roadmap.id}`);
    const content = page.getByRole("region", { name: "Page content" }).getByRole("textbox");
    await expect(content).toHaveAttribute("contenteditable", "true");
    await content.click();
    await page.keyboard.type("Kick-off on Monday");
    await expect
      .poll(async () => (await page.request.get(`/api/v1/pages/${roadmap.id}/export`)).text())
      .toContain("Kick-off on Monday");

    // The page alone, as a markdown file named after it, and with what is under it, as a zip.
    const markdown = await saved(page, "Export as markdown");
    expect(markdown.suggestedFilename()).toBe("Roadmap.md");
    const file = await readFile(await markdown.path());
    expect(file.toString("utf8")).toMatch(/^# Roadmap\n\nKick-off on Monday\n*$/);
    const zip = await saved(page, "Export with subpages");
    expect(zip.suggestedFilename()).toBe("Roadmap.zip");
    const archive = await readFile(await zip.path());
    expect(archive.subarray(0, 2).toString("latin1")).toBe("PK");

    // A whole teamspace, from its page.
    await page.goto(`/w/${workspace.slug}/teamspaces/${general?.id ?? ""}`);
    const teamspaceExport = page.waitForEvent("download");
    await page.getByRole("button", { name: "Export", exact: true }).click();
    expect((await teamspaceExport).suggestedFilename()).toBe("General.zip");

    // The zip comes back in among the private pages, Roadmap with Q1 under it.
    await page.goto(`/w/${workspace.slug}`);
    await page.getByRole("button", { name: "Import markdown" }).click();
    const intoPrivate = await importFile(
      page,
      "Import into your private pages",
      "Roadmap.zip",
      archive,
    );
    await expect(intoPrivate.getByText("Imported 2 pages")).toBeVisible();
    await intoPrivate.getByRole("link", { name: "Open Roadmap" }).click();
    await expect(page.getByRole("textbox", { name: "Page title" })).toHaveValue("Roadmap");
    await expect(content).toContainText("Kick-off on Monday");
    const mine = sidebar(page).getByRole("list", { name: "Private" });
    await mine.getByRole("button", { name: "Expand Roadmap" }).click();
    await expect(mine.getByRole("link", { name: "Q1" })).toBeVisible();

    // And the markdown file goes under that page, from its menu, titled by its heading.
    await expect(page.getByRole("menu")).toHaveCount(0);
    await page.getByRole("main").getByRole("button", { name: "More for Roadmap" }).click();
    await page.getByRole("menuitem", { name: "Import into this page…" }).click();
    const intoPage = await importFile(page, "Import into Roadmap", "Notes.md", file);
    await expect(intoPage.getByText("Imported 1 page.")).toBeVisible();
    await intoPage.getByRole("button", { name: "Done" }).click();
    await expect(intoPage).toBeHidden();
    await expect(mine.getByRole("link", { name: "Roadmap", exact: true })).toHaveCount(2);
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
