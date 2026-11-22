import type { BrowserContext, Page } from "@playwright/test";
import { account, makeAccounts, PASSWORD, signedIn, type Account } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

// Inviting an address without an account answers 404 no_account, and a private teamspace
// answers 404 to a member outside it; Chromium logs both as failed loads.
test.use({ expectedConsoleError: /the server responded with a status of 404/ });

/** Accounts of this run, made by the first admin, so runs against one stack never meet. */
function accounts(run: string): Record<"owner" | "member" | "deputy" | "linked", Account> {
  return {
    owner: account("owner", run),
    member: account("member", run),
    deputy: account("deputy", run),
    linked: account("linked", run),
  };
}

/** The page beside the sidebar. */
function main(page: Page) {
  return page.getByRole("main");
}

/** The card of a list item on the page, by text in it. */
function card(page: Page, text: string) {
  return main(page).getByRole("listitem").filter({ hasText: text });
}

test("an owner builds a workspace that others join by invitation and by link", async ({
  page,
  browser,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const people = accounts(run);
  const workspaceName = `Design ${run}`;

  const adminContext = await browser.newContext();
  const contexts: BrowserContext[] = [adminContext];
  try {
    const adminPage = await adminContext.newPage();
    await makeAccounts(adminPage, Object.values(people));

    // The owner, in no workspace yet, creates one.
    await page.goto("/login");
    await signIn(page, people.owner.email, PASSWORD);
    await expect(
      page.getByRole("heading", { level: 1, name: "You are in no workspace yet" }),
    ).toBeVisible();
    await page.getByRole("link", { name: "Create a workspace" }).click();
    await page.getByLabel("Name").fill(workspaceName);
    await page.getByRole("button", { name: "Create workspace" }).click();
    await expect(page).toHaveURL(new RegExp(`/w/design-${run}$`));
    const slug = new URL(page.url()).pathname.split("/")[2] ?? "";
    const sidebar = page.getByRole("complementary");
    await expect(
      sidebar.getByRole("button", { name: `${workspaceName}, owner: switch workspace` }),
    ).toBeVisible();
    await expect(sidebar.getByRole("link", { name: "General" })).toBeVisible();

    // Invitations by address: an account, an admin-to-be, and an address with no account.
    await page.goto(`/w/${slug}/settings/members`);
    const invite = page.getByRole("form", { name: "Invite by email" });
    await invite.getByLabel("Email").fill(`nobody-${run}@e2e.example`);
    await invite.getByRole("button", { name: "Invite" }).click();
    await expect(invite.getByText(/Workspace invitations never create accounts/)).toBeVisible();
    await invite.getByLabel("Email").fill(people.member.email);
    await invite.getByRole("button", { name: "Invite" }).click();
    await expect(invite.getByText(`Invited ${people.member.email}.`)).toBeVisible();
    await invite.getByLabel("Email").fill(people.deputy.email);
    await invite.getByLabel("Role").selectOption("admin");
    await invite.getByRole("button", { name: "Invite" }).click();
    await expect(invite.getByText(`Invited ${people.deputy.email}.`)).toBeVisible();
    const waiting = page.getByRole("region", { name: "Pending invitations" });
    await expect(waiting.getByText(people.member.email)).toBeVisible();

    // The member accepts in a browser of their own, from the page they land on.
    const [memberContext, memberPage] = await signedIn(browser, people.member);
    contexts.push(memberContext);
    const offer = card(memberPage, workspaceName);
    await expect(offer.getByText("Join as a member")).toBeVisible();
    await expect(offer.getByText(`Invited by ${people.owner.name}`)).toBeVisible();
    await offer.getByRole("button", { name: `Accept: ${workspaceName}` }).click();
    await expect(memberPage).toHaveURL(new RegExp(`/w/${slug}$`));

    // The admin-to-be follows the badge to their invitations.
    const [deputyContext, deputyPage] = await signedIn(browser, people.deputy);
    contexts.push(deputyContext);
    await deputyPage
      .getByRole("complementary")
      .getByRole("link", { name: "Invitations, 1 pending" })
      .click();
    await deputyPage.getByRole("button", { name: `Accept: ${workspaceName}` }).click();
    await expect(deputyPage.getByText(`You joined ${workspaceName}.`)).toBeVisible();

    // A teamspace of each access.
    await page.goto(`/w/${slug}/settings/teamspaces`);
    const create = page.getByRole("form", { name: "Create a teamspace" });
    for (const [name, access] of [
      [`Open ${run}`, "Open"],
      [`Closed ${run}`, "Closed"],
      [`Private ${run}`, "Private"],
    ] as const) {
      await create.getByLabel("Name").fill(name);
      await create.getByRole("radio", { name: access }).click();
      await create.getByRole("button", { name: "Create teamspace" }).click();
      await expect(page.getByText(`${name} is created.`)).toBeVisible();
    }
    await sidebar.getByRole("link", { name: `Private ${run}, private` }).click();
    await expect(page.getByRole("heading", { level: 1, name: `Private ${run}` })).toBeVisible();
    const privatePath = new URL(page.url()).pathname;

    // The member sees the open and closed ones, joins the open one, and is told whom to
    // ask for the closed one; the private one does not exist for them.
    await memberPage.goto(`/w/${slug}/teamspaces`);
    await expect(main(memberPage).getByText("General", { exact: true })).toBeVisible();
    await expect(main(memberPage).getByText(`Private ${run}`)).toHaveCount(0);
    await card(memberPage, `Open ${run}`)
      .getByRole("button", { name: `Join Open ${run}` })
      .click();
    await expect(memberPage.getByText(`You joined Open ${run}.`)).toBeVisible();
    await expect(
      card(memberPage, `Closed ${run}`).getByText(`Closed: ask ${people.owner.name} to add you.`),
    ).toBeVisible();
    await expect(
      memberPage.getByRole("complementary").getByRole("link", { name: `Open ${run}` }),
    ).toBeVisible();
    await memberPage.goto(privatePath);
    await expect(
      memberPage.getByRole("heading", { level: 1, name: "Teamspace not found" }),
    ).toBeVisible();

    // The workspace admin sees the private one by name alone, to manage it.
    await deputyPage.goto(`/w/${slug}/teamspaces`);
    await expect(main(deputyPage).getByText(`Closed ${run}`, { exact: true })).toBeVisible();
    await expect(main(deputyPage).getByText(`Private ${run}`)).toHaveCount(0);
    await deputyPage.goto(`/w/${slug}/settings/teamspaces`);
    await expect(
      card(deputyPage, `Private ${run}`).getByText(/only its name shows to you/),
    ).toBeVisible();
    await deputyPage.goto(privatePath);
    await expect(
      deputyPage.getByRole("heading", { level: 1, name: "Teamspace not found" }),
    ).toBeVisible();

    // An invite link, opened signed out in another browser: sign in, then join.
    await page.goto(`/w/${slug}/settings/members`);
    await page
      .getByRole("form", { name: "Invite links" })
      .getByRole("button", { name: "Make link" })
      .click();
    const link = await page.getByLabel("Invite link", { exact: true }).inputValue();
    expect(link).toMatch(/\/workspace-invite\/wsl_[A-Za-z0-9_-]+$/);
    const linkContext = await browser.newContext();
    contexts.push(linkContext);
    const linkPage = await linkContext.newPage();
    await linkPage.goto(link);
    await expect(linkPage).toHaveURL(/\/workspace-invite$/);
    await linkPage.getByRole("link", { name: "Sign in to join" }).click();
    await signIn(linkPage, people.linked.email, PASSWORD);
    await expect(
      linkPage.getByText("This link invites you to join as a member.", { exact: false }),
    ).toBeVisible();
    await linkPage.getByRole("button", { name: `Join ${workspaceName}` }).click();
    await expect(linkPage).toHaveURL(new RegExp(`/w/${slug}$`));
    await expect(
      linkPage
        .getByRole("complementary")
        .getByRole("button", { name: `${workspaceName}, member: switch workspace` }),
    ).toBeVisible();
    await page.reload();
    await expect(page.getByText("1 of 10 joined")).toBeVisible();

    // The instance admin lists it, with its owner and members, and no way inside.
    await adminPage.goto("/admin/workspaces");
    const listed = card(adminPage, workspaceName);
    await expect(listed.getByText(people.owner.email)).toBeVisible();
    await expect(listed.getByText("4 members")).toBeVisible();
    await expect(listed.getByText(`/w/${slug}`)).toBeVisible();
    await expect(listed.getByRole("link")).toHaveCount(0);
    await adminPage.goto(`/w/${slug}`);
    await expect(
      adminPage.getByRole("heading", { level: 1, name: "Workspace not found" }),
    ).toBeVisible();
    // Nor does the API let them in: the workspace and its teamspaces answer as if absent.
    const owned = await page.request.get("/api/v1/workspaces");
    expect(owned.status()).toBe(200);
    const { workspaces } = (await owned.json()) as {
      workspaces: { id: string; slug: string }[];
    };
    const id = workspaces.find((workspace) => workspace.slug === slug)?.id;
    expect(id, "the owner's workspace").toBeDefined();
    for (const path of [`/api/v1/workspaces/${id}`, `/api/v1/workspaces/${id}/teamspaces`]) {
      const answer = await adminPage.request.get(path);
      expect(answer.status(), path).toBe(404);
      expect(((await answer.json()) as { error: { code: string } }).error.code).toBe("not_found");
    }
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
