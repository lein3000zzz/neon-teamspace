import type { APIRequestContext, BrowserContext, Page } from "@playwright/test";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.use({ extraHTTPHeaders: clientAddress(16) });

/** The protocol revision the client asks for: the last one with sessions. */
const PROTOCOL = "2025-11-25";

/** What an MCP client sends first, opening a session. */
const INITIALIZE = {
  jsonrpc: "2.0",
  id: 0,
  method: "initialize",
  params: {
    protocolVersion: PROTOCOL,
    capabilities: {},
    clientInfo: { name: "neon-e2e", version: "1" },
  },
};

type Workspace = { id: string; slug: string };
type Message = {
  id?: number;
  method?: string;
  params?: { uri?: string };
  result?: Record<string, unknown>;
  error?: { code: number; message: string };
};
type ToolResult = { isError?: boolean; structuredContent: Record<string, unknown> };

/** The JSON-RPC messages of a body of server-sent events. */
function events(text: string): Message[] {
  return text.split(/\r?\n\r?\n/).flatMap((event) => {
    const data = event
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice("data:".length).trimStart())
      .join("\n");
    return data === "" ? [] : [JSON.parse(data) as Message];
  });
}

/** The JSON-RPC messages of an answer to a POST, as JSON or as server-sent events. */
async function answered(response: Awaited<ReturnType<APIRequestContext["post"]>>) {
  const text = await response.text();
  const type = response.headers()["content-type"] ?? "";
  return type.includes("text/event-stream") ? events(text) : [JSON.parse(text) as Message];
}

/** An MCP client of the stack's /mcp, over Streamable HTTP, in one session. */
class Client {
  session = "";
  private next = 1;

  constructor(
    private readonly http: APIRequestContext,
    readonly token: string,
  ) {}

  headers(): Record<string, string> {
    return {
      Authorization: `Bearer ${this.token}`,
      Accept: "application/json, text/event-stream",
      "MCP-Protocol-Version": PROTOCOL,
      ...(this.session === "" ? {} : { "Mcp-Session-Id": this.session }),
    };
  }

  /** Initializes, and says so, as a client does before anything else. */
  async initialize(): Promise<Record<string, unknown>> {
    const response = await this.http.post("/mcp", { headers: this.headers(), data: INITIALIZE });
    expect(response.status(), await response.text()).toBe(200);
    this.session = response.headers()["mcp-session-id"] ?? "";
    expect(this.session, "the session's id").not.toBe("");
    const [opened] = await answered(response);
    const initialized = await this.http.post("/mcp", {
      headers: this.headers(),
      data: { jsonrpc: "2.0", method: "notifications/initialized" },
    });
    expect(initialized.status()).toBe(202);
    return opened?.result ?? {};
  }

  /** A request's result, which must be no error. */
  async send(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>> {
    const id = this.next++;
    const response = await this.http.post("/mcp", {
      headers: this.headers(),
      data: { jsonrpc: "2.0", id, method, params },
    });
    expect(response.status(), `${method}: ${await response.text()}`).toBe(200);
    const answer = (await answered(response)).find((message) => message.id === id);
    expect(answer?.error, method).toBeUndefined();
    return answer?.result ?? {};
  }

  async call(name: string, args: Record<string, unknown>): Promise<ToolResult> {
    return (await this.send("tools/call", { name, arguments: args })) as ToolResult;
  }

  /** A tool's structured answer, which must be no error. */
  async tool<T>(name: string, args: Record<string, unknown>): Promise<T> {
    const result = await this.call(name, args);
    expect(result.isError ?? false, `${name}: ${JSON.stringify(result)}`).toBe(false);
    return result.structuredContent as T;
  }
}

/**
 * The session's stream of notifications, opened as a client listens, read with fetch as it
 * arrives, compressed if Caddy compresses it: what it hears, and whether it ended.
 */
async function listen(origin: string, client: Client) {
  const response = await fetch(`${origin}/mcp`, {
    headers: { ...client.headers(), Accept: "text/event-stream", "Accept-Encoding": "gzip" },
  });
  expect(response.status).toBe(200);
  const heard: Message[] = [];
  const stream = { heard, ended: false };
  const reader = response.body?.pipeThrough(new TextDecoderStream()).getReader();
  void (async () => {
    let pending = "";
    for (;;) {
      const read = await reader?.read().catch(() => undefined);
      if (read === undefined || read.done) {
        stream.ended = true;
        return;
      }
      // Events end with a blank line; the text after the last one is an event on its way.
      const parts = (pending + read.value).split(/\r?\n\r?\n/);
      pending = parts.pop() ?? "";
      heard.push(...parts.flatMap(events));
    }
  })();
  return stream;
}

/** The editable content of the page in view. */
function content(page: Page) {
  return page.getByRole("region", { name: "Page content" }).getByRole("textbox");
}

test("/mcp passes through Caddy, and a request without a token is told where to get one", async ({
  request,
  baseURL,
}) => {
  const response = await request.post("/mcp", {
    headers: { Accept: "application/json, text/event-stream" },
    data: INITIALIZE,
  });

  expect(response.status()).toBe(401);
  const origin = new URL(baseURL ?? "").origin;
  expect(response.headers()["www-authenticate"]).toBe(
    `Bearer resource_metadata="${origin}/.well-known/oauth-protected-resource"`,
  );
  const body = (await response.json()) as { error: { code: string } };
  expect(body.error.code).toBe("unauthenticated");
});

test("an agent's MCP client edits a page live, hears of its owner's typing, and claims a task", async ({
  page: ownerPage,
  browser,
  playwright,
}) => {
  test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("dispatcher", run);
  const contexts: BrowserContext[] = [];
  const clients: APIRequestContext[] = [];
  try {
    const adminContext = await browser.newContext();
    contexts.push(adminContext);
    await makeAccounts(await adminContext.newPage(), [owner]);
    await ownerPage.goto("/login");
    await signIn(ownerPage, owner.email, PASSWORD);
    await expect(ownerPage.getByRole("complementary")).toBeVisible();
    const origin = new URL(ownerPage.url()).origin;
    const workspace = await callApi<Workspace>(
      ownerPage,
      "POST",
      "/api/v1/workspaces",
      { name: `Dispatch ${run}` },
      201,
    );
    const base = `/api/v1/workspaces/${workspace.id}`;
    const agent = await callApi<{ id: string }>(
      ownerPage,
      "POST",
      `${base}/agents`,
      { name: `Scout ${run}` },
      201,
    );
    const { token } = await callApi<{ token: string }>(
      ownerPage,
      "POST",
      `${base}/agents/${agent.id}/tokens`,
      { name: "mcp", scopes: ["read", "write"] },
      201,
    );
    const http = await playwright.request.newContext({ baseURL: origin });
    clients.push(http);

    // A token across the instance is refused: the server works in one workspace.
    const across = await callApi<{ token: string }>(
      ownerPage,
      "POST",
      "/api/v1/account/tokens",
      { name: "everywhere", scopes: ["read"] },
      201,
    );
    const refused = await http.post("/mcp", {
      headers: new Client(http, across.token).headers(),
      data: INITIALIZE,
    });
    expect(refused.status()).toBe(403);
    expect(((await refused.json()) as { error: { code: string } }).error.code).toBe(
      "token_not_bound",
    );

    // The agent's token opens a session, with the stream of notifications beside it.
    const client = new Client(http, token);
    const opened = await client.initialize();
    expect(opened.protocolVersion).toBe(PROTOCOL);
    const stream = await listen(origin, client);
    const { tools } = (await client.send("tools/list", {})) as { tools: { name: string }[] };
    expect(tools.map((tool) => tool.name)).toEqual(
      expect.arrayContaining(["whoami", "read_page", "edit_page", "claim_task"]),
    );
    const me = await client.tool<{ identity: { kind: string; owner: { display_name: string } } }>(
      "whoami",
      {},
    );
    expect(me.identity.kind).toBe("agent");
    expect(me.identity.owner.display_name).toBe(owner.name);
    const { teamspaces } = await client.tool<{ teamspaces: { id: string }[] }>(
      "list_teamspaces",
      {},
    );
    const general = teamspaces[0]?.id;

    // A page it makes from markdown, which its owner sees made by the agent, for them.
    const made = await client.tool<{ page: { id: string }; markdown: string }>("create_page", {
      teamspace_id: general,
      title: `Brief ${run}`,
      markdown: "# Brief\n\nFirst item",
    });
    expect(made.markdown).toMatch(/^<!-- block [0-9a-f-]{36} -->\n# Brief\n/);
    const pageId = made.page.id;
    await ownerPage.goto(`/w/${workspace.slug}/p/${pageId}`);
    await expect(ownerPage.locator("output").filter({ hasText: "Connected" })).toBeVisible();
    await expect(content(ownerPage)).toContainText("First item");
    const byline = ownerPage.getByRole("region", {
      name: "Who made and last changed this page",
    });
    await expect(byline.getByText(`Scout ${run}`, { exact: true }).first()).toBeVisible();
    await expect(byline.getByText(`· via ${owner.name}`, { exact: true }).first()).toBeVisible();

    // Its edit shows in the open editor without a reload.
    await client.tool("edit_page", {
      page_id: pageId,
      operations: [{ op: "append", markdown: "Added by the agent" }],
    });
    await expect(content(ownerPage)).toContainText("Added by the agent");

    // Subscribed to the page, it hears of the owner's typing on its stream.
    const uri = `neon://page/${pageId}`;
    await client.send("resources/subscribe", { uri });
    const before = stream.heard.length;
    await expect(content(ownerPage)).toHaveAttribute("contenteditable", "true");
    await content(ownerPage).getByText("First item", { exact: true }).click();
    await ownerPage.keyboard.press("End");
    await ownerPage.keyboard.press("Enter");
    await ownerPage.keyboard.type("Typed by the owner");
    await expect
      .poll(
        () =>
          stream.heard
            .slice(before)
            .some(
              (message) =>
                message.method === "notifications/resources/updated" && message.params?.uri === uri,
            ),
        { message: "the page's update is told", timeout: 15_000 },
      )
      .toBe(true);
    await expect
      .poll(
        async () =>
          (await client.tool<{ markdown: string }>("read_page", { page_id: pageId })).markdown,
        { message: "the server holds both edits", timeout: 15_000 },
      )
      .toMatch(/First item[\s\S]*Typed by the owner[\s\S]*Added by the agent/);

    // A task it claims is its own, and a second claim hears who holds it.
    const database = await client.tool<{ page: { id: string } }>("create_database", {
      teamspace_id: general,
      title: `Queue ${run}`,
    });
    await callApi(
      ownerPage,
      "POST",
      `/api/v1/databases/${database.page.id}/properties`,
      { name: "Assignee", type: "person" },
      201,
    );
    const row = await client.tool<{ id: string }>("create_row", {
      database_id: database.page.id,
      values: { Name: "Triage the queue" },
    });
    await client.tool("claim_task", { row_id: row.id });
    const markdown = await client.tool<{ properties: { name: string; text: string | null }[] }>(
      "read_page",
      { page_id: row.id },
    );
    const text = (name: string) => markdown.properties.find((entry) => entry.name === name)?.text;
    expect(text("Assignee")).toBe(`Scout ${run} · via ${owner.name}`);
    expect(text("Status")).toBe("In progress");
    const again = await client.call("claim_task", { row_id: row.id });
    expect(again.isError).toBe(true);
    expect(again.structuredContent).toMatchObject({
      error: { code: "already_claimed", holders: [{ id: agent.id }] },
    });

    // Ended by its client, the session closes its stream.
    const ended = await http.delete("/mcp", { headers: client.headers() });
    expect(ended.status()).toBe(202);
    await expect.poll(() => stream.ended, { message: "the stream closes" }).toBe(true);
  } finally {
    for (const client of clients) {
      await client.dispose();
    }
    for (const context of contexts) {
      await context.close();
    }
  }
});
