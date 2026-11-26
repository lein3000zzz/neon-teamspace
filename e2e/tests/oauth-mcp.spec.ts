import type { APIRequestContext, BrowserContext } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import { INITIALIZE, McpClient } from "./mcp-client";
import { allowAsAgent, pkce, REDIRECT, type Tokens } from "./oauth-client";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

const ADDRESS = clientAddress(18);

// Wide enough for a board's columns beside the sidebar, so none needs scrolling to.
test.use({ extraHTTPHeaders: ADDRESS, viewport: { width: 1600, height: 900 } });

type Workspace = { id: string; slug: string };
type ResourceMetadata = { resource: string; authorization_servers: string[] };
type ServerMetadata = {
  authorization_endpoint: string;
  token_endpoint: string;
  registration_endpoint: string;
};

test("an MCP client connects over OAuth as an agent, and the task it claims shows on its owner's board", async ({
  page: ownerPage,
  browser,
  playwright,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("delegator", run);
  const agentName = `Courier ${run}`;
  const app = `E2E MCP client ${run}`;
  const contexts: BrowserContext[] = [];
  const requests: APIRequestContext[] = [];
  try {
    const adminContext = await browser.newContext({ extraHTTPHeaders: ADDRESS });
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner]);

    // The owner makes a workspace and an agent in it.
    await ownerPage.goto("/login");
    await signIn(ownerPage, owner.email, PASSWORD);
    await expect(ownerPage.getByRole("complementary")).toBeVisible();
    const origin = new URL(ownerPage.url()).origin;
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Delegated ${run}` },
      201,
    );
    const agent = await callApi<{ id: string }>(
      ownerPage,
      "POST",
      `/api/v1/workspaces/${workspace.id}/agents`,
      { name: agentName },
      201,
    );
    const http = await playwright.request.newContext({
      baseURL: origin,
      extraHTTPHeaders: ADDRESS,
    });
    requests.push(http);

    // The client finds the OAuth server as MCP clients do: /mcp without a token names its
    // metadata, which names /mcp and the server whose metadata names its endpoints.
    const challenged = await http.post("/mcp", {
      headers: { Accept: "application/json, text/event-stream" },
      data: INITIALIZE,
    });
    expect(challenged.status()).toBe(401);
    const challenge = challenged.headers()["www-authenticate"] ?? "";
    const named = /^Bearer resource_metadata="([^"]+)"$/.exec(challenge)?.[1];
    expect(named, challenge).toBe(`${origin}/.well-known/oauth-protected-resource`);
    const resource = (await (await http.get(named ?? "")).json()) as ResourceMetadata;
    expect(resource.resource).toBe(`${origin}/mcp`);
    expect(resource.authorization_servers).toEqual([origin]);
    const server = (await (
      await http.get(`${origin}/.well-known/oauth-authorization-server`)
    ).json()) as ServerMetadata;
    expect(server.authorization_endpoint).toBe(`${origin}/api/v1/oauth/authorize`);

    // It registers, and the owner allows it as the agent at the consent page.
    const registered = await http.post(server.registration_endpoint, {
      data: { client_name: app, redirect_uris: [REDIRECT] },
    });
    expect(registered.status(), await registered.text()).toBe(201);
    const clientId = ((await registered.json()) as { client_id: string }).client_id;
    const { verifier, challenge: codeChallenge } = pkce();
    const consentContext = await browser.newContext({ extraHTTPHeaders: ADDRESS });
    contexts.push(consentContext);
    const answer = await allowAsAgent(
      await consentContext.newPage(),
      new URLSearchParams({
        response_type: "code",
        client_id: clientId,
        redirect_uri: REDIRECT,
        code_challenge: codeChallenge,
        code_challenge_method: "S256",
        scope: "read write",
        state: `state-${run}`,
        resource: resource.resource,
      }),
      app,
      agentName,
      owner.email,
    );
    expect(answer.searchParams.get("state")).toBe(`state-${run}`);

    // It exchanges the code for tokens for /mcp.
    const issued = await http.post(server.token_endpoint, {
      form: {
        grant_type: "authorization_code",
        client_id: clientId,
        code: answer.searchParams.get("code") ?? "",
        redirect_uri: REDIRECT,
        code_verifier: verifier,
        resource: resource.resource,
      },
    });
    expect(issued.status(), await issued.text()).toBe(200);
    const tokens = (await issued.json()) as Tokens;
    expect(tokens.scope).toBe("read write");

    // With the access token, it opens a session at /mcp and acts as the agent, for its owner.
    const client = new McpClient(http, tokens.access_token);
    await client.initialize();
    const { tools } = (await client.send("tools/list", {})) as { tools: { name: string }[] };
    expect(tools.map((tool) => tool.name)).toEqual(
      expect.arrayContaining(["whoami", "create_page", "read_page", "claim_task"]),
    );
    const me = await client.tool<{
      identity: { id: string; kind: string; owner: { display_name: string } };
      workspace: { id: string };
    }>("whoami", {});
    expect(me.identity).toMatchObject({ id: agent.id, kind: "agent" });
    expect(me.identity.owner.display_name).toBe(owner.name);
    expect(me.workspace.id).toBe(workspace.id);

    // A page it makes, read back.
    const { teamspaces } = await client.tool<{ teamspaces: { id: string }[] }>(
      "list_teamspaces",
      {},
    );
    const general = teamspaces[0]?.id;
    const made = await client.tool<{ page: { id: string; created_by: string } }>("create_page", {
      teamspace_id: general,
      title: `Handover ${run}`,
      markdown: "Picked up over OAuth",
    });
    expect(made.page.created_by).toBe(agent.id);
    const read = await client.tool<{ page: { title: string }; markdown: string }>("read_page", {
      page_id: made.page.id,
    });
    expect(read.page.title).toBe(`Handover ${run}`);
    expect(read.markdown).toContain("Picked up over OAuth");

    // A board with a task nobody holds, which the owner opens.
    const database = await client.tool<{ page: { id: string } }>("create_database", {
      teamspace_id: general,
      title: `Errands ${run}`,
    });
    await callApi(
      ownerPage,
      "POST",
      `/api/v1/databases/${database.page.id}/properties`,
      { name: "Assignee", type: "person" },
      201,
    );
    const task = "Fetch the logs";
    const row = await client.tool<{ id: string }>("create_row", {
      database_id: database.page.id,
      values: { Name: task },
    });
    await ownerPage.goto(`/w/${workspace.slug}/p/${database.page.id}`);
    const board = ownerPage.getByRole("region", { name: `Database Errands ${run}` });
    await board.getByRole("tab", { name: "Board" }).click();
    await expect(board.locator("output").filter({ hasText: "Live" })).toBeVisible();
    await expect(board.getByRole("article", { name: task })).toBeVisible();
    const inProgress = board.getByRole("region", { name: "In progress" });
    await expect(inProgress.getByRole("article", { name: task })).toHaveCount(0);

    // The client claims it: the card moves to In progress without a reload, and its row
    // names the agent as its assignee, acting for its owner.
    await client.tool("claim_task", { row_id: row.id });
    const card = inProgress.getByRole("article", { name: task });
    await expect(card).toBeVisible();
    await card.getByRole("button", { name: task, exact: true }).click();
    const peek = ownerPage.getByRole("dialog", { name: `${task}, beside the database` });
    const assignee = peek
      .getByRole("region", { name: "Properties" })
      .getByRole("button", { name: new RegExp(`^Assignee of ${task}: `) });
    await expect(assignee.getByText(agentName, { exact: true })).toBeVisible();
    await expect(assignee.getByText(`· via ${owner.name}`, { exact: true })).toBeVisible();
  } finally {
    for (const request of requests) {
      await request.dispose();
    }
    for (const context of contexts) {
      await context.close();
    }
  }
});
