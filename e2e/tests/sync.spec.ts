import { account, clientAddress, makeAccounts, PASSWORD, runOf } from "./accounts";
import { ADMIN, type ApiErrorBody, expect, signIn, test } from "./fixtures";

test.use({
  extraHTTPHeaders: async ({}, use, testInfo) => use(clientAddress(24, testInfo.repeatEachIndex)),
});

/** A page id no page has. */
const NO_PAGE = "0193a5b2-0000-7000-8000-0000000000a1";

test("Caddy passes /ws on to the server, which wants a session first", async ({ request }) => {
  const response = await request.get(`/ws/pages/${NO_PAGE}`);

  expect(response.status()).toBe(401);
  expect(response.headers()["content-type"]).toContain("application/json");
  const body = (await response.json()) as ApiErrorBody;
  expect(body.error.code).toBe("unauthenticated");
});

test("a page's document opens over a WebSocket through Caddy, within the app's policy", async ({
  page,
  browser,
}) => {
  test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");
  // An account of the run's own, as a workspace of the first admin's would follow them into
  // every other spec.
  const run = runOf();
  const writer = account("syncer", run);
  const adminContext = await browser.newContext();
  try {
    await makeAccounts(await adminContext.newPage(), [writer]);
  } finally {
    await adminContext.close();
  }
  await page.goto("/login");
  await signIn(page, writer.email, PASSWORD);
  await expect(page.getByRole("complementary")).toBeVisible();
  const origin = new URL(page.url()).origin;
  const headers = { Origin: origin };

  const workspace = await page.request.post("/api/v1/workspaces", {
    headers,
    data: { name: `Sync ${run}` },
  });
  expect(workspace.status()).toBe(201);
  const { id: workspaceId } = (await workspace.json()) as { id: string };
  const teamspaces = await page.request.get(`/api/v1/workspaces/${workspaceId}/teamspaces`);
  const { teamspaces: list } = (await teamspaces.json()) as {
    teamspaces: { id: string }[];
  };
  const created = await page.request.post(`/api/v1/workspaces/${workspaceId}/pages`, {
    headers,
    data: { teamspace_id: list[0]?.id, title: "Notes" },
  });
  expect(created.status()).toBe(201);
  const { id: pageId } = (await created.json()) as { id: string };

  // The browser opens it as y-websocket would, with the session cookie, and the server
  // starts the sync: its first message is sync step 1, tagged 0 and 0.
  const first = await page.evaluate(async (path) => {
    const url = `${location.origin.replace(/^http/, "ws")}${path}`;
    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";
    const data = await new Promise<ArrayBuffer>((resolve, reject) => {
      socket.addEventListener("message", (event) => resolve(event.data as ArrayBuffer));
      socket.addEventListener("close", (event) => reject(new Error(`closed: ${event.code}`)));
    });
    socket.close();
    return Array.from(new Uint8Array(data).slice(0, 2));
  }, `/ws/pages/${pageId}`);
  expect(first).toEqual([0, 0]);
});
