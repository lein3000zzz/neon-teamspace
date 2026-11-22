import type { BrowserContext, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

test.use({ extraHTTPHeaders: clientAddress(12) });

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };

/** The editable content of the page in view. */
function content(page: Page) {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox");
}

/** Waits until the page is in sync with the server, and so takes typing. */
async function live(page: Page) {
  await expect(page.locator("output").filter({ hasText: "Connected" })).toBeVisible();
  await expect(content(page)).toHaveAttribute("contenteditable", "true");
}

/** Types at the end of the page's content. */
async function typeAtEnd(page: Page, text: string) {
  await content(page).click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.press("Enter");
  await page.keyboard.type(text);
}

test("two people typing into one page see each other's text and cursors", async ({
  page: adaPage,
  browser,
}) => {
  test.setTimeout(90_000);
  const run = String(Date.now());
  const ada = account("writer", run);
  const bob = account("editor", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob]);

    // Ada's workspace, with Bob in it, and so in General, where the page is.
    await adaPage.goto("/login");
    await signIn(adaPage, ada.email, PASSWORD);
    await expect(adaPage.getByRole("complementary")).toBeVisible();
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Live ${run}` },
      201,
    );
    await callApi(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invitations`,
      { email: bob.email, role: "member" },
      201,
    );
    const [bobContext, bobPage] = await signedIn(browser, bob);
    contexts.push(bobContext);
    await bobPage.getByRole("button", { name: `Accept: Live ${run}` }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));

    const response = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await response.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    expect(general, "the workspace's General").toBeDefined();
    const page = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Live notes" },
      201,
    );
    const address = `/w/${workspace.slug}/p/${page.id}`;
    await adaPage.goto(address);
    await bobPage.goto(address);
    await live(adaPage);
    await live(bobPage);

    // Each sees what the other types, as they type it.
    await typeAtEnd(adaPage, "Agenda from Ada");
    await expect(content(bobPage)).toContainText("Agenda from Ada");
    await typeAtEnd(bobPage, "Notes from Bob");
    await expect(content(adaPage)).toContainText("Notes from Bob");
    await expect(content(adaPage)).toContainText("Agenda from Ada");

    // And where the other's cursor is, named, in the other's hue.
    const bobsCursor = adaPage.locator(".bn-collaboration-cursor__base", { hasText: bob.name });
    await expect(bobsCursor).toHaveCount(1);
    await expect(bobsCursor).toHaveAttribute("data-hue", "");
    const adasCursor = bobPage.locator(".bn-collaboration-cursor__base", {
      hasText: ada.name,
    });
    await expect(adasCursor).toHaveCount(1);

    // Both are listed as present to the other.
    await expect(adaPage.getByText(`Also here: ${bob.name}`)).toBeAttached();
    await expect(bobPage.getByText(`Also here: ${ada.name}`)).toBeAttached();

    // What was typed is the server's: a fresh load of the page has it.
    await bobPage.reload();
    await live(bobPage);
    await expect(content(bobPage)).toContainText("Agenda from Ada");
    await expect(content(bobPage)).toContainText("Notes from Bob");
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
