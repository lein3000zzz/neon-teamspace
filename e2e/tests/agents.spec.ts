import type { BrowserContext } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD, signedIn } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

test.use({ extraHTTPHeaders: clientAddress(15) });

type Workspace = { id: string; slug: string };
type Teamspace = { id: string; name: string };
type Members = { members: { user: { id: string; email?: string | null } }[] };

test("an agent's token makes a page where it was added, named with its owner, until the owner leaves", async ({
  page: ownerPage,
  browser,
  playwright,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("lead", run);
  const keeper = account("keeper", run);
  const contexts: BrowserContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner, keeper]);

    // The workspace's owner brings the agent's owner in, and makes a closed teamspace
    // with them in it.
    await ownerPage.goto("/login");
    await signIn(ownerPage, owner.email, PASSWORD);
    await expect(ownerPage.getByRole("complementary")).toBeVisible();
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Agents ${run}` },
      201,
    );
    const base = `/api/v1/workspaces/${workspace.id}`;
    await callApi(
      ownerPage,
      "POST",
      `${base}/invitations`,
      { email: keeper.email, role: "member" },
      201,
    );
    const [keeperContext, keeperPage] = await signedIn(browser, keeper);
    contexts.push(keeperContext);
    await keeperPage.getByRole("button", { name: `Accept: Agents ${run}` }).click();
    await expect(keeperPage).toHaveURL(new RegExp(`/w/${workspace.slug}$`));
    const ops = await callApi<Teamspace>(
      ownerPage,
      "POST",
      `${base}/teamspaces`,
      { name: `Ops ${run}`, access: "closed" },
      201,
    );
    const members = (await (await ownerPage.request.get(`${base}/members`)).json()) as Members;
    const keeperId = members.members.find((member) => member.user.email === keeper.email)?.user.id;
    expect(keeperId, "the keeper's id").toBeDefined();
    await callApi(
      ownerPage,
      "POST",
      `${base}/teamspaces/${ops.id}/members`,
      { user_id: keeperId, role: "member" },
      201,
    );

    // The keeper makes an agent, gives it a token, and adds it to Ops.
    await keeperPage.goto(`/w/${workspace.slug}/settings/agents`);
    const make = keeperPage.getByRole("form", { name: "Make an agent" });
    await make.getByLabel("Name").fill(`Scout ${run}`);
    await make.getByRole("button", { name: "Make agent" }).click();
    const card = keeperPage.getByRole("listitem", { name: `Scout ${run}` });
    await expect(card.getByText("Active", { exact: true })).toBeVisible();
    const tokens = card.getByRole("region", { name: `Tokens of Scout ${run}` });
    const makeToken = tokens.getByRole("form", { name: "Make token" });
    await makeToken.getByLabel("Token name").fill("worker");
    await makeToken.getByRole("button", { name: "Make token" }).click();
    // Shown once, in place of the form.
    const shown = tokens.getByRole("textbox", { name: "Token", exact: true });
    await expect(shown).toHaveValue(/^nw_/);
    const secret = await shown.inputValue();
    await tokens.getByRole("button", { name: "Done" }).click();
    const join = card.getByRole("form", { name: `Add to a teamspace: Scout ${run}` });
    await join.getByRole("combobox", { name: "Teamspace" }).selectOption({ label: `Ops ${run}` });
    await join.getByRole("button", { name: "Add" }).click();
    await expect(keeperPage.getByText(`Scout ${run} is in Ops ${run}.`)).toBeVisible();

    // The token, over REST, makes a page in Ops as the agent.
    const agent = await playwright.request.newContext({
      baseURL: new URL(ownerPage.url()).origin,
      extraHTTPHeaders: { Authorization: `Bearer ${secret}` },
    });
    try {
      const made = await agent.post(`${base}/pages`, {
        data: { teamspace_id: ops.id, title: `Report ${run}` },
      });
      expect(made.status(), await made.text()).toBe(201);
      const page = (await made.json()) as { id: string; created_by: string };

      // People see the page named with the agent and whom it acts for.
      await ownerPage.goto(`/w/${workspace.slug}/p/${page.id}`);
      const byline = ownerPage.getByRole("region", {
        name: "Who made and last changed this page",
      });
      await expect(byline.getByText(`Scout ${run}`, { exact: true })).toBeVisible();
      await expect(byline.getByText("Agent", { exact: true })).toBeVisible();
      await expect(byline.getByText(`· via ${keeper.name}`, { exact: true })).toBeVisible();

      // Once its owner leaves the workspace, the agent's token is refused.
      const leave = await keeperPage.request.post(`${base}/leave`, {
        headers: { Origin: new URL(keeperPage.url()).origin },
      });
      expect(leave.status(), await leave.text()).toBe(204);
      const refused = await agent.get("/api/v1/workspaces");
      expect(refused.status()).toBe(401);
      await ownerPage.goto(`/w/${workspace.slug}/settings/agents`);
      await expect(
        ownerPage
          .getByRole("listitem", { name: `Scout ${run}` })
          .getByText("Suspended", { exact: true }),
      ).toBeVisible();
    } finally {
      await agent.dispose();
    }
  } finally {
    for (const context of contexts) {
      await context.close();
    }
  }
});
