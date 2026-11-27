/**
 * A person of a test's own with a workspace and a database in it, for the specs that look at
 * how the app lays out. They sign in from the address their spec takes.
 */
import type { Browser, BrowserContextOptions, Page } from "@playwright/test";
import { account, callApi, makeAccounts, PASSWORD } from "./accounts";
import { expect, signIn } from "./fixtures";

type Workspace = { id: string; slug: string };
export type Teamspace = { id: string; access: string };

/** A database called Sprint in General: its page, and the ways to it. */
type Sprint = { workspace: Workspace; pageId: string; api: string; path: string };

/**
 * A person of this run, whom the admin makes, signed in on a page of a browser of their
 * own, with a workspace of theirs holding a database: on `page` when given, else on a
 * page of a context made with `options`.
 */
export async function sprintOf(
  browser: Browser,
  role: string,
  options: { page?: Page; context?: BrowserContextOptions },
): Promise<Sprint & { page: Page }> {
  const run = String(Date.now());
  const owner = account(role, run);
  const adminContext = await browser.newContext();
  try {
    await makeAccounts(await adminContext.newPage(), [owner]);
  } finally {
    await adminContext.close();
  }
  const page = options.page ?? (await (await browser.newContext(options.context)).newPage());
  await page.goto("/login");
  await signIn(page, owner.email, PASSWORD);
  await expect(page).not.toHaveURL(/\/login/);

  const workspace = await callApi<Workspace>(
    page,
    "POST",
    "/api/v1/workspaces",
    { name: `${owner.name} desk` },
    201,
  );
  const answer = await page.request.get(`/api/v1/workspaces/${workspace.id}/teamspaces`);
  const { teamspaces } = (await answer.json()) as { teamspaces: Teamspace[] };
  const general = teamspaces.find((teamspace) => teamspace.access === "default");
  const database = await callApi<{ page: { id: string } }>(
    page,
    "POST",
    `/api/v1/workspaces/${workspace.id}/databases`,
    { teamspace_id: general?.id, title: "Sprint" },
    201,
  );
  return {
    page,
    workspace,
    pageId: database.page.id,
    api: `/api/v1/databases/${database.page.id}`,
    path: `/w/${workspace.slug}/p/${database.page.id}`,
  };
}
