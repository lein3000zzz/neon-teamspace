import type { BrowserContext, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// These tests drive pages through their tree, header and dialogs. A page also opens its live
// connection, which collaboration.spec.ts tests and which may fail here while it reconnects,
// or as a page closes before it opened; Chromium logs such a socket as failed.
test.use({
  extraHTTPHeaders: clientAddress(11),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

type Workspace = { id: string; slug: string };

/** The sidebar, with the page trees. */
function sidebar(page: Page) {
  return page.getByRole("complementary");
}

/** The row of a page in General's tree in the sidebar, which holds its buttons and its menu. */
function row(page: Page, title: string) {
  return sidebar(page)
    .getByRole("list", { name: "Pages of General" })
    .locator("[data-row]")
    .filter({ has: page.getByRole("link", { name: title, exact: true }) });
}

/**
 * Opens a page's menu in the sidebar and picks an item, once the menu picked from before has
 * faded out: Radix keeps a closing menu until its animation ends, and one opened again
 * before then closes with it.
 */
async function fromMenu(page: Page, title: string, item: string) {
  await expect(page.getByRole("menu")).toHaveCount(0);
  await row(page, title).hover();
  await row(page, title)
    .getByRole("button", { name: `More for ${title}` })
    .click();
  await page.getByRole("menuitem", { name: item }).click();
}

/** Types the title of the page just made, which opens with the cursor in it. */
async function titleNewPage(page: Page, title: string) {
  const field = page.getByRole("textbox", { name: "Page title" });
  await expect(field).toBeFocused();
  await field.fill(title);
  await field.press("Enter");
  await expect(sidebar(page).getByRole("link", { name: title, exact: true })).toBeVisible();
}

test("an owner builds a tree of pages, shares one to view, and takes it back from the trash", async ({
  page,
  browser,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("pages", run);
  const guest = account("reader", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner, guest]);

    // The owner, in this test's own page, makes a workspace and has the guest in it.
    const ownerPage = page;
    await ownerPage.goto("/login");
    await signIn(ownerPage, owner.email, PASSWORD);
    await expect(sidebar(ownerPage)).toBeVisible();
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Pages ${run}` },
      201,
    );
    await callApi(
      ownerPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invitations`,
      { email: guest.email, role: "guest" },
      201,
    );
    const [guestContext, guestPage] = await signedIn(browser, guest);
    contexts.push(guestContext);
    await guestPage.getByRole("button", { name: `Accept: Pages ${run}` }).click();
    await expect(guestPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));
    await expect(guestPage.getByText("None are yet.")).toBeVisible();

    // Roadmap > Q1 > Goals, each made from the sidebar.
    await ownerPage.goto(`/w/${workspace.slug}`);
    await sidebar(ownerPage).getByRole("button", { name: "Add a page to General" }).click();
    await titleNewPage(ownerPage, "Roadmap");
    await row(ownerPage, "Roadmap")
      .getByRole("button", { name: "Add a page inside Roadmap" })
      .click();
    await titleNewPage(ownerPage, "Q1");
    await row(ownerPage, "Q1").getByRole("button", { name: "Add a page inside Q1" }).click();
    await titleNewPage(ownerPage, "Goals");
    const crumbs = ownerPage.getByRole("navigation", { name: "Where this page is" });
    await expect(crumbs).toHaveText(/General\s*\/\s*Roadmap\s*\/\s*Q1\s*\/\s*Goals/);

    // A rename in place, and a move from the menu, keyboard and all.
    await fromMenu(ownerPage, "Goals", "Rename");
    const field = sidebar(ownerPage).getByRole("textbox", { name: "New title for Goals" });
    await field.fill("Targets");
    await field.press("Enter");
    await expect(row(ownerPage, "Targets")).toBeVisible();
    await expect(ownerPage.getByRole("textbox", { name: "Page title" })).toHaveValue("Targets");
    await fromMenu(ownerPage, "Targets", "Move to…");
    const move = ownerPage.getByRole("dialog", { name: "Move Targets" });
    await move.getByLabel("Find a place").fill("Roadmap");
    await move.getByRole("button", { name: "Roadmap, In General" }).click();
    await expect(move).toBeHidden();
    await expect(crumbs).toHaveText(/General\s*\/\s*Roadmap\s*\/\s*Targets/);

    // A favorite, from the menu, listed above the teamspaces.
    await fromMenu(ownerPage, "Q1", "Add to favorites");
    await expect(
      sidebar(ownerPage).getByRole("list", { name: "Favorites" }).getByRole("link", { name: "Q1" }),
    ).toBeVisible();

    // Roadmap shared with the guest, to view, and everything under it with it.
    await sidebar(ownerPage).getByRole("link", { name: "Roadmap", exact: true }).click();
    await expect(ownerPage.getByRole("textbox", { name: "Page title" })).toHaveValue("Roadmap");
    await ownerPage.getByRole("main").getByRole("button", { name: "Share" }).click();
    const share = ownerPage.getByRole("dialog", { name: "Share Roadmap" });
    await share.getByLabel("Person").selectOption({ label: `${guest.name} (Guest)` });
    await share.getByLabel("Access").selectOption("can_view");
    await share
      .getByRole("form", { name: "Share with someone" })
      .getByRole("button", { name: "Share" })
      .click();
    await expect(share.getByText(`Shared with ${guest.name}.`)).toBeVisible();
    await expect(share.getByRole("combobox", { name: `Access of ${guest.name}` })).toHaveValue(
      "can_view",
    );
    await share.getByRole("button", { name: "Done" }).click();

    // The guest finds it, and reads it without a way to change it.
    await guestPage.reload();
    const shared = sidebar(guestPage).getByRole("list", { name: "Shared with you" });
    await shared.getByRole("link", { name: "Roadmap" }).click();
    await expect(guestPage.getByRole("heading", { level: 1, name: "Roadmap" })).toBeVisible();
    await expect(guestPage.getByText("You can view this page, but not change it.")).toBeVisible();
    await expect(guestPage.getByRole("textbox", { name: "Page title" })).toHaveCount(0);
    await expect(guestPage.getByRole("main").getByRole("button", { name: "Share" })).toHaveCount(0);
    await expect(
      guestPage.getByRole("region", { name: "Page content" }).getByRole("textbox"),
    ).toHaveAttribute("contenteditable", "false");
    await shared.getByRole("button", { name: "Expand Roadmap" }).click();
    await expect(shared.getByRole("link", { name: "Q1" })).toBeVisible();
    await expect(shared.getByRole("link", { name: "Targets" })).toBeVisible();

    // Into the trash with everything under it, and back again.
    await fromMenu(ownerPage, "Roadmap", "Move to the trash");
    await expect(sidebar(ownerPage).getByText("Roadmap is in the trash.")).toBeVisible();
    await expect(row(ownerPage, "Roadmap")).toHaveCount(0);
    await expect(ownerPage.getByText("This page is in the trash.")).toBeVisible();
    await sidebar(ownerPage).getByRole("link", { name: "Trash" }).click();
    const trash = ownerPage.getByRole("list", { name: "Trash" });
    await expect(trash.getByRole("link", { name: "Roadmap" })).toBeVisible();
    await expect(trash.getByRole("link", { name: "Q1" })).toHaveCount(0);
    await trash.getByRole("button", { name: "Restore Roadmap" }).click();
    await expect(ownerPage.getByText("Roadmap is back.")).toBeVisible();
    await expect(ownerPage.getByText("The trash is empty.")).toBeVisible();
    // Back where it was, still open as it was before.
    await expect(row(ownerPage, "Q1")).toBeVisible();
    await expect(row(ownerPage, "Targets")).toBeVisible();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
