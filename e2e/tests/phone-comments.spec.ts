import type { BrowserContext } from "@playwright/test";
import { account, callApi, makeAccounts, PASSWORD, runOf } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import {
  content,
  desk,
  live,
  openDrawer,
  pageIn,
  personOnPhone,
  phoneAddress,
  selectText,
  toolbar,
} from "./phone";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// Bob leaves the page he opened from his inbox; Chromium logs a socket that a page closes
// before it opened as failed.
test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(phoneAddress(32, testInfo)),
  expectedConsoleError: /WebSocket connection to '[^']*\/ws\/pages\/[^']*' failed/,
});

test("a comment on a passage, written on a phone, reaches the inbox of the person it mentions", async ({
  page,
  browser,
}) => {
  test.setTimeout(120_000);
  const contexts: BrowserContext[] = [];
  try {
    const { person: ada, workspace } = await personOnPhone(browser, page, "traveller");
    const bob = account(`companion-${test.info().project.name}`, runOf());
    const adminContext = await desk(browser);
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [bob]);
    await callApi(
      page,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invitations`,
      { email: bob.email, role: "member" },
      201,
    );

    // Bob, on a phone of his own, takes the invitation.
    const bobContext = await browser.newContext();
    contexts.push(bobContext);
    const bobPage = await bobContext.newPage();
    await bobPage.goto("/login");
    await signIn(bobPage, bob.email, PASSWORD);
    await bobPage.getByRole("button", { name: `Accept: ${workspace.name}` }).tap();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));

    // Ada selects a word with her finger and comments on it from the toolbar, naming Bob.
    const plan = await pageIn(page, workspace, "Trip plan", "We leave for the coast on Saturday.");
    await page.goto(plan.path);
    await live(page);
    await content(page).getByText("We leave for the coast").tap();
    const bar = toolbar(page);
    await expect(bar.getByRole("button", { name: "Comment" })).toBeDisabled();
    await selectText(page, "Saturday");
    await expect(bar.getByRole("button", { name: "Comment" })).toBeEnabled();
    await bar.getByRole("button", { name: "Comment" }).tap();

    const panel = page.getByRole("complementary", { name: "Comments" });
    const draft = panel.getByRole("region", { name: "A comment on the selected text" });
    await expect(draft.getByText("Saturday", { exact: true })).toBeVisible();
    const field = draft.getByRole("combobox", { name: "A comment on the selected text" });
    await field.tap();
    await field.pressSequentially("Could we go on Friday, @compan");
    await panel.getByRole("option", { name: new RegExp(bob.name) }).tap();
    await field.pressSequentially("?");
    await draft.getByRole("button", { name: "Comment" }).tap();
    const thread = panel.getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(thread.getByText(`@${bob.name}`)).toBeVisible();
    await expect(content(page).locator(".comment-highlight")).toHaveText("Saturday");

    // Bob opens his inbox from the drawer, which counts it, and the mention opens the page.
    const drawer = await openDrawer(bobPage);
    await drawer.getByRole("link", { name: "Inbox, 1 unread" }).tap();
    await expect(drawer).toBeHidden();
    const summary = `${ada.name} mentioned you in a comment on Trip plan`;
    await bobPage.getByRole("link", { name: summary }).tap();
    await expect(bobPage).toHaveURL(new RegExp(`/w/${workspace.slug}/p/${plan.id}`));
    const bobsThread = bobPage
      .getByRole("complementary", { name: "Comments" })
      .getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(bobsThread.getByText("Could we go on Friday")).toBeVisible();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
