import type { BrowserContext, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import { png } from "./png";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// The upload the full workspace refuses answers 409, which Chromium logs as a failed load;
// and a page's live connection may fail while it reconnects, as pages.spec.ts says.
test.use({
  extraHTTPHeaders: clientAddress(41),
  expectedConsoleError:
    /the server responded with a status of 409|WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };

/** The editable content of the page in view. */
function content(page: Page) {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox");
}

/** Waits until the page is in sync with the server, and so takes input. */
async function live(page: Page) {
  await expect(page.locator("output").filter({ hasText: "Connected" })).toBeVisible();
  await expect(content(page)).toHaveAttribute("contenteditable", "true");
}

const PDF = Buffer.from(
  "%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n",
);

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

/** Drops a file onto the editor, below its last block. */
async function drop(page: Page, name: string, type: string, bytes: Buffer) {
  await content(page).evaluate(
    (editor, { name, type, data }) => {
      const transfer = new DataTransfer();
      transfer.items.add(new File([new Uint8Array(data)], name, { type }));
      const blocks = editor.querySelectorAll("[data-node-type='blockContainer']");
      const last = (blocks[blocks.length - 1] ?? editor).getBoundingClientRect();
      const at = { clientX: last.left + 20, clientY: last.bottom - 4 };
      for (const kind of ["dragenter", "dragover", "drop"]) {
        editor.dispatchEvent(
          new DragEvent(kind, { dataTransfer: transfer, bubbles: true, cancelable: true, ...at }),
        );
      }
    },
    { name, type, data: [...bytes] },
  );
}

/** The page's image, once it has loaded from the server. */
async function loadedImage(page: Page) {
  const image = page.getByRole("region", { name: "Page content" }).locator("img").first();
  await expect(image).toHaveAttribute("src", /^\/api\/v1\/files\/[0-9a-f-]{36}$/);
  await expect
    .poll(() => image.evaluate((element) => (element as HTMLImageElement).naturalWidth))
    .toBe(4);
  return image;
}

/** The page's file block, by the file's name. */
function fileBlock(page: Page, name: string) {
  return page
    .getByRole("region", { name: "Page content" })
    .locator("[data-content-type='file']")
    .filter({ hasText: name });
}

test("an image pasted and a PDF dropped show after a reload and for another person, until the workspace is full", async ({
  page: adaPage,
  browser,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const ada = account("uploader", run);
  const bob = account("reader", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    const adminPage = await adminContext.newPage();
    await makeAccounts(adminPage, [ada, bob]);

    await adaPage.goto("/login");
    await signIn(adaPage, ada.email, PASSWORD);
    await expect(adaPage.getByRole("complementary")).toBeVisible();
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Files ${run}` },
      201,
    );
    await callApi(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invitations`,
      { email: bob.email, role: "member" },
      201,
    );
    const teamspaces = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces: list } = (await teamspaces.json()) as { teamspaces: Teamspace[] };
    const general = list.find((teamspace) => teamspace.access === "default");
    expect(general, "the workspace's General").toBeDefined();
    const page = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Launch kit", markdown: "Files for the launch." },
      201,
    );
    const address = `/w/${workspace.slug}/p/${page.id}`;
    await adaPage.goto(address);
    await live(adaPage);

    // A screenshot pasted at the cursor becomes an image block, its file on the server.
    await content(adaPage).getByText("Files for the launch.").click();
    await adaPage.keyboard.press("End");
    await paste(adaPage, "screenshot.png", "image/png", png(4));
    await loadedImage(adaPage);

    // A PDF dropped onto the page becomes a file block.
    await drop(adaPage, "report.pdf", "application/pdf", PDF);
    await expect(fileBlock(adaPage, "report.pdf")).toBeVisible();

    // Both are the page's for good: a fresh load shows them.
    await adaPage.reload();
    await live(adaPage);
    const image = await loadedImage(adaPage);
    await expect(fileBlock(adaPage, "report.pdf")).toBeVisible();
    const src = await image.getAttribute("src");
    const served = await adaPage.request.get(src ?? "");
    expect(served.status()).toBe(200);
    expect(served.headers()["content-type"]).toBe("image/png");
    // Caddy adds the app's own headers beside the server's: a browser takes the first
    // nosniff, and holds the file to both policies, the server's sandbox among them.
    expect(served.headers()["x-content-type-options"]).toMatch(/^nosniff\b/);
    expect(served.headers()["content-security-policy"]).toContain("sandbox");
    expect(served.headers()["content-disposition"]).toMatch(/^inline;/);
    expect(served.headers()["cache-control"]).toBe("private, no-cache");

    // Another member, in a browser of their own, sees them too.
    const [bobContext, bobPage] = await signedIn(browser, bob);
    contexts.push(bobContext);
    await bobPage.getByRole("button", { name: `Accept: Files ${run}` }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));
    await bobPage.goto(address);
    await live(bobPage);
    await loadedImage(bobPage);
    await expect(fileBlock(bobPage, "report.pdf")).toBeVisible();
    // The page's markdown names both by their paths on the server.
    const markdown = await adaPage.request.get(`/api/v1/pages/${page.id}/markdown`);
    const { markdown: text } = (await markdown.json()) as { markdown: string };
    expect(text.match(/\/api\/v1\/files\/[0-9a-f-]{36}/g)).toHaveLength(2);
    expect(text).toContain(":::file [report.pdf](/api/v1/files/");

    // Once the workspace is at its storage limit, the next upload is refused, and says why.
    await callApi(
      adminPage,
      "PATCH",
      `/api/v1/admin/workspaces/${workspace.id}/limits`,
      { storage_bytes: 64 },
      200,
    );
    await content(adaPage).getByText("Files for the launch.").click();
    await adaPage.keyboard.press("End");
    await paste(adaPage, "another.png", "image/png", png(4));
    const refused = adaPage
      .getByRole("alert")
      .filter({ hasText: "The file could not be uploaded." });
    await expect(refused).toBeVisible();
    await expect(refused).toContainText("storage limit of 64 bytes");
    await expect(
      adaPage.getByRole("region", { name: "Page content" }).locator("img[src^='/api/v1/files/']"),
    ).toHaveCount(1);
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
