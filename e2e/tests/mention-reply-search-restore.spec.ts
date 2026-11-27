import { deflateSync } from "node:zlib";
import type { APIRequestContext, BrowserContext, Locator, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import { McpClient } from "./mcp-client";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// A page's live connection may fail while it reconnects, as pages.spec.ts says.
test.use({
  extraHTTPHeaders: clientAddress(42),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };

/** The editable content of the page in view. */
function content(page: Page) {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox");
}

/** The comments panel beside the page in view. */
function panel(page: Page) {
  return page.getByRole("complementary", { name: "Comments" });
}

/** The sidebar, which the comments panel stands beside as another complementary region. */
function sidebar(page: Page) {
  return page.getByRole("complementary").first();
}

/** Waits until the page is in sync with the server, and so takes input. */
async function live(page: Page) {
  await expect(page.locator("output").filter({ hasText: "Connected" })).toBeVisible();
  await expect(content(page)).toHaveAttribute("contenteditable", "true");
}

/** Selects `text` in the page's content, as dragging over it would. */
async function select(page: Page, text: string) {
  await content(page).getByText(text, { exact: false }).first().click();
  await page.evaluate((wanted) => {
    const root = document.querySelector('[aria-label="Page content"] [contenteditable]');
    const walker = document.createTreeWalker(root ?? document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const at = node.textContent?.indexOf(wanted) ?? -1;
      if (at >= 0) {
        document.getSelection()?.setBaseAndExtent(node, at, node, at + wanted.length);
        return;
      }
    }
    throw new Error(`no "${wanted}" in the page`);
  }, text);
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  return c >>> 0;
});

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc = (CRC_TABLE[(crc ^ byte) & 0xff] ?? 0) ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Uint8Array): Buffer {
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

/** A PNG of `size` by `size` pixels of one colour, as a browser decodes it. */
function png(size: number): Buffer {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header.set([8, 2, 0, 0, 0], 8);
  const row = Buffer.concat([Buffer.from([0]), Buffer.alloc(size * 3, 0x40)]);
  const pixels = Buffer.concat(Array.from({ length: size }, () => row));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(pixels)),
    chunk("IEND", new Uint8Array()),
  ]);
}

/** Pastes a file into the editor where the cursor is, as pasting a screenshot does. */
async function paste(page: Page, name: string, type: string, bytes: Buffer) {
  await content(page).evaluate(
    (editor, { name, type, data }) => {
      const transfer = new DataTransfer();
      transfer.items.add(new File([new Uint8Array(data)], name, { type }));
      editor.dispatchEvent(
        new ClipboardEvent("paste", { clipboardData: transfer, bubbles: true, cancelable: true }),
      );
    },
    { name, type, data: [...bytes] },
  );
}

/** The page's images uploaded to the server. */
function images(page: Page) {
  return page.getByRole("region", { name: "Page content" }).locator("img[src^='/api/v1/files/']");
}

/** Waits until the page's image has loaded from the server, and says where from. */
async function loadedImage(page: Page): Promise<string> {
  const image = images(page).first();
  await expect(image).toHaveAttribute("src", /^\/api\/v1\/files\/[0-9a-f-]{36}$/);
  await expect
    .poll(() => image.evaluate((element) => (element as HTMLImageElement).naturalWidth))
    .toBe(4);
  return (await image.getAttribute("src")) ?? "";
}

/** The page's markdown as the server reads it now. */
async function markdownOf(page: Page, pageId: string): Promise<string> {
  const response = await page.request.get(`/api/v1/pages/${pageId}/markdown`);
  return ((await response.json()) as { markdown: string }).markdown;
}

/**
 * Restores a version the history panel lists, latest first, `pick` choosing it among the
 * list's buttons, and closes the panel.
 */
async function restore(page: Page, pick: (versions: Locator) => Locator) {
  await page.getByRole("button", { name: "History" }).click();
  const history = page.getByRole("dialog", { name: "Page history" });
  await pick(history.getByRole("region", { name: "Versions" }).getByRole("button")).click();
  await history.getByRole("button", { name: "Restore this version" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Restore" }).click();
  await expect(history.getByText("Restored. The page shows this version again.")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(history).toBeHidden();
}

test("a comment's mention reaches the inbox, an agent replies over MCP, search finds the page, and a restore keeps its image", async ({
  page: adaPage,
  browser,
  playwright,
}) => {
  test.setTimeout(180_000);
  const run = String(Date.now());
  const ada = account("surveyor", run);
  const bob = account("checker", run);
  const contexts: BrowserContext[] = [];
  const clients: APIRequestContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob]);

    await adaPage.goto("/login");
    await signIn(adaPage, ada.email, PASSWORD);
    await expect(sidebar(adaPage)).toBeVisible();
    const origin = new URL(adaPage.url()).origin;
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Survey ${run}` },
      201,
    );
    const base = `/api/v1/workspaces/${workspace.id}`;
    await callApi(
      adaPage,
      "POST",
      `${base}/invitations`,
      { email: bob.email, role: "member" },
      201,
    );
    const [bobContext, bobPage] = await signedIn(browser, bob);
    contexts.push(bobContext);
    await bobPage.getByRole("button", { name: `Accept: Survey ${run}` }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));

    const listed = await adaPage.request.get(`${base}/teamspaces`);
    const { teamspaces } = (await listed.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    expect(general, "the workspace's General").toBeDefined();
    const plan = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `${base}/pages`,
      {
        teamspace_id: general?.id,
        title: "Harbour plan",
        markdown: "The lighthouse survey starts on Friday.",
      },
      201,
    );
    const address = `/w/${workspace.slug}/p/${plan.id}`;
    await bobPage.goto("/inbox");
    await expect(bobPage.getByText("Nothing unread.")).toBeVisible();

    // Ada comments on a word of the page, mentioning Bob.
    await adaPage.goto(address);
    await live(adaPage);
    await select(adaPage, "Friday");
    await adaPage.keyboard.press("Control+Alt+KeyM");
    const draft = panel(adaPage).getByRole("region", { name: "A comment on the selected text" });
    const field = draft.getByRole("combobox", { name: "A comment on the selected text" });
    await field.pressSequentially("Check the estuary tides first, @check");
    await expect(panel(adaPage).getByRole("option", { name: new RegExp(bob.name) })).toBeVisible();
    await field.press("Enter");
    await draft.getByRole("button", { name: "Comment" }).click();
    const adasThread = panel(adaPage).getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(adasThread.getByText(`@${bob.name}`)).toBeVisible();
    await expect(content(adaPage).locator(".comment-highlight")).toHaveText("Friday");

    // Bob sees it in his inbox, opens it at the thread, and replies.
    const summary = `${ada.name} mentioned you in a comment on Harbour plan`;
    await bobPage.getByRole("link", { name: summary }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/p/${plan.id}\\?comment=`));
    await live(bobPage);
    const bobsThread = panel(bobPage).getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(bobsThread).toHaveAttribute("data-active", "");
    await bobsThread.getByRole("button", { name: "Reply" }).click();
    await bobsThread.getByRole("combobox", { name: "Your reply" }).fill("The tides suit Friday.");
    await bobsThread.getByRole("button", { name: "Reply" }).click();
    await expect(adasThread.getByText("The tides suit Friday.")).toBeVisible();

    // Ada's agent lists the page's comments over MCP and replies in the thread.
    const agent = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `${base}/agents`,
      { name: `Pilot ${run}` },
      201,
    );
    const { token } = await callApi<{ token: string }>(
      adaPage,
      "POST",
      `${base}/agents/${agent.id}/tokens`,
      { name: "mcp", scopes: ["read", "write"] },
      201,
    );
    const http = await playwright.request.newContext({ baseURL: origin });
    clients.push(http);
    const client = new McpClient(http, token);
    await client.initialize();
    type Threads = { threads: { id: string; comment_count: number }[] };
    const threads = await client.tool<Threads>("list_comments", { page_id: plan.id });
    expect(threads.threads).toHaveLength(1);
    expect(threads.threads[0]?.comment_count).toBe(2);
    await client.tool("add_comment", {
      page_id: plan.id,
      thread_id: threads.threads[0]?.id,
      markdown: "High water is at **noon**.",
    });
    await expect(adasThread.getByText("High water is at")).toBeVisible();
    await expect(bobsThread.getByText(`Pilot ${run}`, { exact: true })).toBeVisible();

    // Search finds the page by a word of its comments and by a word of its text.
    await bobPage.goto(`/w/${workspace.slug}/search?q=estuary`);
    const results = bobPage.getByRole("region", { name: "Results" });
    await expect(results.getByRole("link", { name: "Harbour plan" })).toBeVisible();
    await expect(results.locator("mark").filter({ hasText: "estuary" })).toBeVisible();
    await bobPage.getByRole("searchbox", { name: "Search" }).fill("lighthouse");
    await expect(bobPage).toHaveURL(/q=lighthouse/);
    await expect(results.locator("mark").filter({ hasText: "lighthouse" })).toBeVisible();
    await expect(results.getByRole("link", { name: "Harbour plan" })).toBeVisible();

    // Ada pastes an image after the commented paragraph.
    await content(adaPage).getByText("starts on", { exact: false }).click();
    await adaPage.keyboard.press("End");
    await paste(adaPage, "chart.png", "image/png", png(4));
    const src = await loadedImage(adaPage);
    await expect.poll(() => markdownOf(adaPage, plan.id)).toContain(src);

    // Restoring the page's first version, the oldest, takes the image out, and keeps the page
    // with it as a version first, next to the latest; restoring that one puts the image back,
    // its file still there.
    await restore(adaPage, (versions) => versions.last());
    await expect(images(adaPage)).toHaveCount(0);
    await expect.poll(() => markdownOf(adaPage, plan.id)).not.toContain(src);
    await restore(adaPage, (versions) => versions.nth(1));
    expect(await loadedImage(adaPage)).toBe(src);
    expect((await adaPage.request.get(src)).status()).toBe(200);
    await expect(content(adaPage).locator(".comment-highlight")).toHaveText("Friday");
    await expect(adasThread.getByText("High water is at")).toBeVisible();
  } finally {
    for (const client of clients) {
      await client.dispose();
    }
    for (const context of contexts) {
      await context.close();
    }
  }
});
