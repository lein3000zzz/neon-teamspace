/**
 * Who may read a page's file: a member downloads it from its block, as the editor offers it,
 * while nobody outside the workspace gets it, a member who left included.
 */
import type { BrowserContext, Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import {
  account,
  callApi,
  clientAddress,
  makeAccounts,
  PASSWORD,
  runOf,
  signedIn,
} from "./accounts";
import { ADMIN, type ApiErrorBody, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// The refusals answer 401 and 404, which Chromium logs as failed loads; and a page's live
// connection may fail while it reconnects, as pages.spec.ts says.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(clientAddress(30, testInfo.repeatEachIndex)),
  expectedConsoleError:
    /the server responded with a status of 40[14]|WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };
type Members = { members: { user: { id: string; email?: string | null } }[] };

/** A PDF of a few bytes, as a report would be. */
const PDF = Buffer.from(
  "%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n",
);

/** The editable content of the page in view. */
function content(page: Page) {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox");
}

/** Waits until the page is in sync with the server, and so takes input. */
async function live(page: Page) {
  await expect(page.locator("output").filter({ hasText: "Connected" })).toBeVisible();
  await expect(content(page)).toHaveAttribute("contenteditable", "true");
}

/** The status and the error's code a browser gets that opens `url` and is refused. */
async function refusal(page: Page, url: string): Promise<[number, string]> {
  const response = await page.goto(url);
  if (response === null) {
    throw new Error(`no answer to ${url}`);
  }
  return [response.status(), ((await response.json()) as ApiErrorBody).error.code];
}

test("a member downloads a page's file from its block, and nobody outside the workspace gets it", async ({
  page: bobPage,
  browser,
}) => {
  test.setTimeout(90_000);
  const run = runOf();
  const ada = account("archivist", run);
  const bob = account("borrower", run);
  const cat = account("outsider", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob, cat]);

    // Ada's workspace, with a page that holds a PDF, and Bob in it by an invite link.
    const [adaContext, adaPage] = await signedIn(browser, ada);
    contexts.push(adaContext);
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Archive ${run}` },
      201,
    );
    const answer = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const page = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Reports", markdown: "The quarter's report:" },
      201,
    );
    const uploaded = await adaPage.request.post(`/api/v1/pages/${page.id}/files?kind=file`, {
      headers: { Origin: new URL(adaPage.url()).origin },
      multipart: { file: { name: "report.pdf", mimeType: "application/pdf", buffer: PDF } },
    });
    expect(uploaded.status()).toBe(201);
    const file = (await uploaded.json()) as { url: string };
    await callApi(
      adaPage,
      "PATCH",
      `/api/v1/pages/${page.id}/markdown`,
      { operations: [{ op: "append", markdown: `:::file [report.pdf](${file.url})` }] },
      200,
    );
    const { token } = await callApi<{ token: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invite-links`,
      { role: "member" },
      201,
    );

    await bobPage.goto("/login");
    await signIn(bobPage, bob.email, PASSWORD);
    await expect(bobPage.getByRole("complementary")).toBeVisible();
    await callApi(bobPage, "POST", "/api/v1/workspace-invite-links/join", { token }, 200);

    // Bob picks the file's block, and downloads it from the toolbar.
    await bobPage.goto(`/w/${workspace.slug}/p/${page.id}`);
    await live(bobPage);
    const block = bobPage
      .getByRole("region", { name: "Page content" })
      .locator("[data-content-type='file']")
      .filter({ hasText: "report.pdf" });
    await block.click();
    // The editor opens the file's address in a new tab, which the browser saves, as the
    // server says it is an attachment.
    const downloaded = bobPage.waitForEvent("download");
    await bobPage.getByRole("button", { name: "Download file" }).click();
    const download = await downloaded;
    expect(download.suggestedFilename()).toBe("report.pdf");
    expect(await readFile(await download.path())).toEqual(PDF);

    // Someone signed in but outside the workspace is told there is no such file, and a
    // visitor signed in nowhere to sign in.
    const [catContext, catPage] = await signedIn(browser, cat);
    contexts.push(catContext);
    expect(await refusal(catPage, file.url)).toEqual([404, "not_found"]);
    const visitorContext = await browser.newContext();
    contexts.push(visitorContext);
    expect(await refusal(await visitorContext.newPage(), file.url)).toEqual([
      401,
      "unauthenticated",
    ]);

    // Once Ada removes Bob from the workspace, the file is no more his to read.
    const listed = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/members`);
    const { members } = (await listed.json()) as Members;
    const bobId = members.find((member) => member.user.email === bob.email)?.user.id;
    expect(bobId, "Bob's id").toBeDefined();
    const removed = await adaPage.request.delete(
      `/api/v1/workspaces/${workspace.id}/members/${bobId ?? ""}`,
      { headers: { Origin: new URL(adaPage.url()).origin } },
    );
    expect(removed.status()).toBe(204);
    expect(await refusal(bobPage, file.url)).toEqual([404, "not_found"]);
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
