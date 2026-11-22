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

test("an editor whose share turns to view goes read-only at once, and back, on one connection", async ({
  page: ownerPage,
  browser,
}) => {
  test.setTimeout(90_000);
  const run = String(Date.now());
  const owner = account("sharer", run);
  const guest = account("helper", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner, guest]);

    await ownerPage.goto("/login");
    await signIn(ownerPage, owner.email, PASSWORD);
    await expect(ownerPage.getByRole("complementary")).toBeVisible();
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Access ${run}` },
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
    await guestPage.getByRole("button", { name: `Accept: Access ${run}` }).click();
    await expect(guestPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));

    const answer = await ownerPage.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const page = await callApi<{ id: string }>(
      ownerPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Shared plan" },
      201,
    );
    const members = await ownerPage.request.get(`/api/v1/workspaces/${workspace.id}/members`);
    const { members: list } = (await members.json()) as {
      members: { user: { id: string; email: string } }[];
    };
    const guestId = list.find((member) => member.user.email === guest.email)?.user.id;
    expect(guestId, "the guest's id").toBeDefined();
    await callApi(
      ownerPage,
      "POST",
      `/api/v1/pages/${page.id}/shares`,
      { user_id: guestId, access: "can_edit" },
      201,
    );

    const sockets: string[] = [];
    guestPage.on("websocket", (socket) => sockets.push(socket.url()));
    await guestPage.goto(`/w/${workspace.slug}/p/${page.id}`);
    await live(guestPage);
    const title = guestPage.getByRole("textbox", { name: "Page title" });
    await expect(title).toBeVisible();

    const change = (access: string) =>
      callApi(
        ownerPage,
        "PATCH",
        `/api/v1/pages/${page.id}/shares/${guestId ?? ""}`,
        { access },
        200,
      );

    // To view: the editor and the header stop taking changes, without reconnecting.
    await change("can_view");
    await expect(guestPage.locator("output").filter({ hasText: "Read-only" })).toBeVisible();
    await expect(content(guestPage)).toHaveAttribute("contenteditable", "false");
    await expect(title).toHaveCount(0);

    // Back to edit: both take changes again, and what is typed reaches the server.
    await change("can_edit");
    await live(guestPage);
    await expect(title).toBeVisible();
    await typeAtEnd(guestPage, "Typed after the share came back");
    await expect
      .poll(async () => (await ownerPage.request.get(`/api/v1/pages/${page.id}/export`)).text())
      .toContain("Typed after the share came back");
    expect(sockets).toHaveLength(1);
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
