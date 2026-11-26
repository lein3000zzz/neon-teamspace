import { expect, type APIRequestContext } from "@playwright/test";

/** The protocol revision the client asks for: the last one with sessions. */
export const PROTOCOL = "2025-11-25";

/** What an MCP client sends first, opening a session. */
export const INITIALIZE = {
  jsonrpc: "2.0",
  id: 0,
  method: "initialize",
  params: {
    protocolVersion: PROTOCOL,
    capabilities: {},
    clientInfo: { name: "neon-e2e", version: "1" },
  },
};

export type Message = {
  id?: number;
  method?: string;
  params?: { uri?: string };
  result?: Record<string, unknown>;
  error?: { code: number; message: string };
};

export type ToolResult = { isError?: boolean; structuredContent: Record<string, unknown> };

/** The JSON-RPC messages of a body of server-sent events. */
export function events(text: string): Message[] {
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
export class McpClient {
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
