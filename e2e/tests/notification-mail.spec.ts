/**
 * The inbox's mail, which the e2e stack sends as soon as a notification comes: a mention, and
 * an invitation into a workspace, each mailed to someone signed in nowhere, who follows the
 * mail's link.
 */
import type { BrowserContext, Page } from "@playwright/test";
import {
  account,
  callApi,
  clientAddress,
  makeAccounts,
  PASSWORD,
  runOf,
  signedIn,
} from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import { instanceName, linkIn, MAIL, mailTo } from "./mail";

test.skip(
  ADMIN === undefined || MAIL === "",
  "needs the first admin and the Mailpit that make e2e sets up",
);

test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(clientAddress(29, testInfo.repeatEachIndex)),
});

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; access: string };
type Members = { members: { user: { id: string; email?: string | null } }[] };

/** Signs `page` in as `email` through the form it is on, and waits for the page it leads to. */
async function signInHere(page: Page, email: string) {
  await expect(page.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
  await signIn(page, email, PASSWORD);
}

test("a mention in a comment comes by mail, whose link opens the thread, and whose foot stops the mail", async ({
  page: bobPage,
  browser,
  baseURL,
}) => {
  test.setTimeout(90_000);
  const origin = new URL(baseURL ?? "").origin;
  const instance = await instanceName(bobPage.request);
  const run = runOf();
  const ada = account("announcer", run);
  const bob = account("listener", run);
  const workspaceName = `Mail ${run}`;
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [ada, bob]);

    const [adaContext, adaPage] = await signedIn(browser, ada);
    contexts.push(adaContext);
    const workspace = await callApi<Workspace>(
      adaPage,
      "POST",
      "/api/v1/workspaces",
      { name: workspaceName },
      201,
    );
    // Bob joins by an invite link, which tells his inbox nothing, so the mention is the only
    // notification to mail him.
    const { token } = await callApi<{ token: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/invite-links`,
      { role: "member" },
      201,
    );
    const [joinContext, joinPage] = await signedIn(browser, bob);
    await callApi(joinPage, "POST", "/api/v1/workspace-invite-links/join", { token }, 200);
    await joinContext.close();

    const listed = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/members`);
    const { members } = (await listed.json()) as Members;
    const bobId = members.find((member) => member.user.email === bob.email)?.user.id;
    expect(bobId, "Bob's id").toBeDefined();
    const answer = await adaPage.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
    const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
    const general = teamspaces.find((teamspace) => teamspace.access === "default");
    const notes = await callApi<{ id: string }>(
      adaPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/pages`,
      { teamspace_id: general?.id, title: "Launch notes", markdown: "The beta ships on Friday." },
      201,
    );

    // Ada mentions Bob in a comment on the page.
    const posted = await callApi<{ thread: { id: string } }>(
      adaPage,
      "POST",
      `/api/v1/pages/${notes.id}/comments`,
      { markdown: `Can you check the date, @[${bob.name}](user:${bobId ?? ""})?` },
      201,
    );

    // The mail says who mentioned him where, quotes the comment, and links to its thread.
    const mail = await mailTo(
      bob.email,
      `${ada.name} mentioned you in a comment on Launch notes, in ${workspaceName}`,
    );
    expect(mail.text).toContain(`“Can you check the date, @${bob.name}?”`);
    const link = linkIn(mail, `Open in ${instance}`, origin);
    expect(link).toBe(`${origin}/w/${workspace.slug}/p/${notes.id}?comment=${posted.thread.id}`);

    // Signed in nowhere, Bob follows it: he signs in, and lands on the thread.
    await bobPage.goto(link);
    await expect(bobPage).toHaveURL(/\/login\?next=/);
    await signInHere(bobPage, bob.email);
    await expect(bobPage).toHaveURL(link);
    const thread = bobPage
      .getByRole("complementary", { name: "Comments" })
      .getByRole("article", { name: `Thread by ${ada.name}` });
    await expect(thread).toHaveAttribute("data-active", "");
    await expect(thread.getByText(`@${bob.name}`)).toBeVisible();

    // The link at its foot stops the mail without signing in, once confirmed.
    const stop = linkIn(mail, "Stop these mails", origin);
    const strangerContext = await browser.newContext();
    contexts.push(strangerContext);
    const stopPage = await strangerContext.newPage();
    await stopPage.goto(stop);
    await expect(
      stopPage.getByRole("heading", { level: 1, name: "Stop notification mail" }),
    ).toBeVisible();
    await stopPage.getByRole("button", { name: "Stop the mail" }).click();
    await expect(
      stopPage.getByRole("heading", { level: 1, name: "No more notification mail" }),
    ).toBeVisible();
    await bobPage.goto("/account/notifications");
    await expect(
      bobPage
        .getByRole("radiogroup", { name: "Notification mail" })
        .getByRole("radio", { name: "Nothing" }),
    ).toBeChecked();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});

test("an invitation into a workspace comes by mail, whose link opens the inbox, where it is accepted", async ({
  page: guestPage,
  browser,
  baseURL,
}) => {
  test.setTimeout(90_000);
  const origin = new URL(baseURL ?? "").origin;
  const instance = await instanceName(guestPage.request);
  const run = runOf();
  const owner = account("host", run);
  const guest = account("visitor", run);
  const workspaceName = `Guild ${run}`;
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner, guest]);

    const [ownerContext, ownerPage] = await signedIn(browser, owner);
    contexts.push(ownerContext);
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: workspaceName },
      201,
    );
    await ownerPage.goto(`/w/${workspace.slug}/settings/members`);
    const invite = ownerPage.getByRole("form", { name: "Invite by email" });
    await invite.getByLabel("Email").fill(guest.email);
    await invite.getByRole("button", { name: "Invite" }).click();
    await expect(invite.getByText(`Invited ${guest.email}.`)).toBeVisible();

    // The mail names who invited them where, and leads to the inbox.
    const mail = await mailTo(guest.email, `${owner.name} invited you to join ${workspaceName}`);
    const link = linkIn(mail, `Open in ${instance}`, origin);
    expect(link).toBe(`${origin}/inbox`);

    // Signed in nowhere, the invitee follows it, signs in, and accepts there.
    await guestPage.goto(link);
    await signInHere(guestPage, guest.email);
    await expect(guestPage).toHaveURL(/\/inbox$/);
    await guestPage
      .getByRole("main")
      .getByRole("region", { name: "Invitations" })
      .getByRole("button", { name: `Accept: ${workspaceName}` })
      .click();
    await expect(guestPage.getByText(`You joined ${workspaceName}.`)).toBeVisible();
    await expect(
      guestPage
        .getByRole("complementary")
        .getByRole("button", { name: `${workspaceName}, member: switch workspace` }),
    ).toBeVisible();

    // The owner sees them among the members, no longer among those invited.
    await ownerPage.reload();
    await expect(
      ownerPage.getByRole("region", { name: "Pending invitations" }).getByText(guest.email),
    ).toHaveCount(0);
    await expect(ownerPage.getByRole("main").getByText(guest.email)).toBeVisible();
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
