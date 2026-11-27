import type { APIRequestContext, BrowserContext, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import { listen, McpClient } from "./mcp-client";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

test.use({ extraHTTPHeaders: clientAddress(21) });

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

/** Waits until the page is in sync with the server. */
async function live(page: Page, editable = true) {
  await expect(page.locator("output").filter({ hasText: /Connected|Read-only/ })).toBeVisible();
  await expect(content(page)).toHaveAttribute("contenteditable", editable ? "true" : "false");
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

/** The General teamspace of a workspace, where its members' pages go. */
async function general(page: Page, workspace: Workspace): Promise<string> {
  const response = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
  const { teamspaces } = (await response.json()) as { teamspaces: Teamspace[] };
  const found = teamspaces.find((teamspace) => teamspace.access === "default");
  expect(found, "the workspace's General").toBeDefined();
  return found?.id ?? "";
}

test("two people comment on a passage, a mention reaches the inbox live, and opens at its highlight", async ({
  page: adaPage,
  browser,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const ada = account("planner", run);
  const bob = account("reviewer", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob]);

    await adaPage.goto("/login");
    await signIn(adaPage, ada.email, PASSWORD);
    await expect(adaPage.getByRole("complementary").first()).toBeVisible();
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Review ${run}` },
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
    await bobPage.getByRole("button", { name: `Accept: Review ${run}` }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));

    const plan = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      {
        teamspace_id: await general(adaPage, workspace),
        title: "Launch plan",
        markdown: "We ship the beta on Friday morning.",
      },
      201,
    );
    const address = `/w/${workspace.slug}/p/${plan.id}`;

    // Bob waits in his inbox, which follows its feed.
    await bobPage.goto("/inbox");
    await expect(bobPage.getByText("Nothing unread.")).toBeVisible();

    // Ada selects a word and comments on it from the keyboard, mentioning Bob.
    await adaPage.goto(address);
    await live(adaPage);
    await select(adaPage, "Friday");
    await adaPage.keyboard.press("Control+Alt+KeyM");
    const draft = panel(adaPage).getByRole("region", { name: "A comment on the selected text" });
    await expect(draft.getByText("Friday", { exact: true })).toBeVisible();
    const field = draft.getByRole("combobox", { name: "A comment on the selected text" });
    await field.pressSequentially("Can we move this, @revi");
    await expect(panel(adaPage).getByRole("option", { name: new RegExp(bob.name) })).toBeVisible();
    await field.press("Enter");
    await field.pressSequentially("?");
    await draft.getByRole("button", { name: "Comment" }).click();

    const adasThread = panel(adaPage).getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(adasThread.getByText(`@${bob.name}`)).toBeVisible();
    await expect(content(adaPage).locator(".comment-highlight")).toHaveText("Friday");

    // Bob's inbox shows it as it comes, with the sidebar's count.
    const summary = `${ada.name} mentioned you in a comment on Launch plan`;
    await expect(bobPage.getByRole("link", { name: summary })).toBeVisible();
    await expect(
      bobPage.getByRole("complementary").first().getByRole("link", { name: "Inbox, 1 unread" }),
    ).toBeVisible();

    // Its link opens the page at the thread: the passage marked and in view, and focus on
    // the thread in the panel. No text is selected, so no formatting toolbar shows.
    await bobPage.getByRole("link", { name: summary }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/p/${plan.id}\\?comment=`));
    await live(bobPage);
    const bobsThread = panel(bobPage).getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(bobsThread).toHaveAttribute("data-active", "");
    await expect(bobsThread).toBeFocused();
    await expect(bobsThread).toHaveAccessibleDescription("Friday");
    const passage = content(bobPage).locator(".comment-highlight.is-active");
    await expect(passage).toHaveText("Friday");
    await expect(passage).toBeInViewport();
    expect(
      await bobPage.evaluate(() => document.getSelection()?.toString() ?? ""),
      "no text selected",
    ).toBe("");
    await expect(bobPage.locator(".bn-formatting-toolbar")).toHaveCount(0);

    // Bob replies; Ada sees the reply in her panel, and in her inbox's count.
    await bobsThread.getByRole("button", { name: "Reply" }).click();
    await bobsThread
      .getByRole("combobox", { name: "Your reply" })
      .fill("Monday works for the beta.");
    await bobsThread.getByRole("button", { name: "Reply" }).click();
    await expect(adasThread.getByText("Monday works for the beta.")).toBeVisible();
    await expect(
      adaPage.getByRole("complementary").first().getByRole("link", { name: "Inbox, 1 unread" }),
    ).toBeVisible();

    // Text typed before the passage leaves the thread on it, in both editors.
    await content(bobPage).getByText("We ship", { exact: false }).click();
    await bobPage.keyboard.press("Home");
    await bobPage.keyboard.type("Update: ");
    await expect
      .poll(async () => (await adaPage.request.get(`/api/v1/pages/${plan.id}/export`)).text(), {
        message: "the server holds Bob's change",
      })
      .toContain("Update: We ship");
    // Ada's editor shows Bob's cursor, named, where he typed, so its text holds his name too.
    await expect(content(adaPage)).toContainText("Update: ");
    await expect(content(adaPage).locator(".comment-highlight")).toHaveText("Friday");
    await expect(content(bobPage).locator(".comment-highlight")).toHaveText("Friday");
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});

test("a mention picked after @ in a page reaches the inbox once the text settles, at its block", async ({
  page: adaPage,
  browser,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const ada = account("author", run);
  const bob = account("helper", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob]);
    await adaPage.goto("/login");
    await signIn(adaPage, ada.email, PASSWORD);
    await expect(adaPage.getByRole("complementary").first()).toBeVisible();
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Mentions ${run}` },
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
    await bobPage.getByRole("button", { name: `Accept: Mentions ${run}` }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));
    const page = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: await general(adaPage, workspace), title: "Handover", markdown: "Owner:" },
      201,
    );
    await bobPage.goto("/inbox");
    await expect(bobPage.getByText("Nothing unread.")).toBeVisible();

    // Ada types @ and the start of Bob's name, and picks him from the menu.
    await adaPage.goto(`/w/${workspace.slug}/p/${page.id}`);
    await live(adaPage);
    await content(adaPage).getByText("Owner:", { exact: true }).click();
    await adaPage.keyboard.press("End");
    await adaPage.keyboard.type(" @help");
    await adaPage.getByRole("option", { name: new RegExp(bob.name) }).click();
    await expect(content(adaPage).locator(".mention")).toHaveText(`@${bob.name}`);

    // Once the page's text settles, Bob is told, quoting the block, and the link opens the
    // page at the block.
    const summary = `${ada.name} mentioned you on Handover`;
    await expect(bobPage.getByRole("link", { name: summary })).toBeVisible({ timeout: 30_000 });
    await expect(bobPage.getByRole("listitem").filter({ hasText: summary })).toContainText(
      `Owner: @${bob.name}`,
    );
    await bobPage.getByRole("link", { name: summary }).click();
    await expect(bobPage).toHaveURL(new RegExp(`/p/${page.id}\\?block=`));
    await live(bobPage);
    await expect(content(bobPage)).toBeFocused();
    await expect(content(bobPage).locator(".mention")).toHaveText(`@${bob.name}`);
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});

test("an agent hears of a mention over MCP, replies in the thread, and clears its inbox", async ({
  page: ownerPage,
  browser,
  playwright,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("editor", run);
  const contexts: BrowserContext[] = [];
  const clients: APIRequestContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner]);
    await ownerPage.goto("/login");
    await signIn(ownerPage, owner.email, PASSWORD);
    await expect(ownerPage.getByRole("complementary").first()).toBeVisible();
    const origin = new URL(ownerPage.url()).origin;
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Notes ${run}` },
      201,
    );
    const base = `/api/v1/workspaces/${workspace.id}`;
    const agent = await callApi<{ id: string }>(
      ownerPage,
      "POST",
      `${base}/agents`,
      { name: `Scribe ${run}` },
      201,
    );
    const { token } = await callApi<{ token: string }>(
      ownerPage,
      "POST",
      `${base}/agents/${agent.id}/tokens`,
      { name: "mcp", scopes: ["read", "write"] },
      201,
    );
    const notes = await callApi<{ id: string }>(
      ownerPage,
      "POST",
      `${base}/pages`,
      {
        teamspace_id: await general(ownerPage, workspace),
        title: "Meeting notes",
        markdown: "Decided to ship on Monday.",
      },
      201,
    );

    const http = await playwright.request.newContext({ baseURL: origin });
    clients.push(http);
    const client = new McpClient(http, token);
    await client.initialize();
    const stream = await listen(origin, client);
    const { resources } = (await client.send("resources/list", {})) as {
      resources: { uri: string }[];
    };
    expect(resources.map((resource) => resource.uri)).toContain("neon://me/inbox");
    await client.send("resources/subscribe", { uri: "neon://me/inbox" });

    // The owner asks the agent in a comment, and opens the page with its comments.
    await callApi(
      ownerPage,
      "POST",
      `/api/v1/pages/${notes.id}/comments`,
      { markdown: `Please summarise this, @[Scribe](user:${agent.id})` },
      201,
    );
    await ownerPage.goto(`/w/${workspace.slug}/p/${notes.id}`);
    await live(ownerPage);
    await ownerPage.getByRole("button", { name: "Comments, 1 open" }).click();
    const thread = panel(ownerPage).getByRole("article", { name: `Thread by ${owner.name}` });
    await expect(thread).toBeVisible();

    // The subscription tells of it, and the inbox holds it, with its thread.
    await expect
      .poll(
        () =>
          stream.heard.some(
            (message) =>
              message.method === "notifications/resources/updated" &&
              message.params?.uri === "neon://me/inbox",
          ),
        { message: "the inbox's update is told", timeout: 15_000 },
      )
      .toBe(true);
    type Inbox = { notifications: { id: string; kind: string; thread_id: string }[] };
    const inbox = await client.tool<Inbox>("get_inbox", { unread: true });
    expect(inbox.notifications).toHaveLength(1);
    const [mention] = inbox.notifications;
    expect(mention?.kind).toBe("mention");

    // It replies in the thread, which the owner sees as it comes, by the agent for them.
    const listed = await client.tool<{ threads: { id: string }[] }>("list_comments", {
      page_id: notes.id,
    });
    expect(listed.threads.map((each) => each.id)).toEqual([mention?.thread_id]);
    await client.tool("add_comment", {
      page_id: notes.id,
      thread_id: mention?.thread_id,
      markdown: "We decided to ship on **Monday**.",
    });
    await expect(thread.getByText("We decided to ship on")).toBeVisible();
    await expect(thread.getByText(`Scribe ${run}`, { exact: true })).toBeVisible();
    await expect(thread.getByText(`· via ${owner.name}`, { exact: true })).toBeVisible();
    // The owner, who commented, follows the page's comments.
    await expect(panel(ownerPage).getByRole("button", { name: "Following" })).toBeVisible();

    // Marked read, its inbox has nothing unread; the owner's tells of the reply.
    await client.tool("mark_read", { notification_ids: [mention?.id] });
    const after = await client.tool<Inbox>("get_inbox", { unread: true });
    expect(after.notifications).toEqual([]);
    await expect(
      ownerPage.getByRole("complementary").first().getByRole("link", { name: "Inbox, 1 unread" }),
    ).toBeVisible();
  } finally {
    for (const client of clients) {
      await client.dispose();
    }
    for (const context of contexts) {
      await context.close();
    }
  }
});
