import { createHash } from "node:crypto";
import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

const ADDRESS = clientAddress(17);

test.use({ extraHTTPHeaders: ADDRESS });

/** Where the client of this spec listens, as a CLI's would: nothing does, the test answers. */
const REDIRECT = "http://127.0.0.1:43110/callback";

type Workspace = { id: string; slug: string };
type Agent = { id: string; name: string };
type Teamspaces = { teamspaces: { id: string }[] };
type Tokens = { access_token: string; refresh_token: string; scope: string; expires_in: number };

/** A PKCE verifier and its S256 challenge, low in entropy: no test secret looks real. */
function pkce(): { verifier: string; challenge: string } {
  const verifier = "e".repeat(48);
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  return { verifier, challenge };
}

test("a client registers, is allowed as an agent at the consent page, and calls the API with its token", async ({
  page: ownerPage,
  browser,
  playwright,
}) => {
  test.setTimeout(120_000);
  const run = String(Date.now());
  const owner = account("connector", run);
  const adminContext = await browser.newContext({ extraHTTPHeaders: ADDRESS });
  const consentContext = await browser.newContext({ extraHTTPHeaders: ADDRESS });
  try {
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
      { name: `Connected ${run}` },
      201,
    );
    const base = `/api/v1/workspaces/${workspace.id}`;
    const scout = await callApi<Agent>(
      ownerPage,
      "POST",
      `${base}/agents`,
      { name: `Scout ${run}` },
      201,
    );
    const teamspaces = (await (
      await ownerPage.request.get(`${base}/teamspaces`)
    ).json()) as Teamspaces;
    const general = teamspaces.teamspaces[0]?.id;
    expect(general, "the default teamspace").toBeDefined();

    const client = await playwright.request.newContext({
      baseURL: origin,
      extraHTTPHeaders: ADDRESS,
    });
    try {
      // The stack's web server hands the metadata to the server, open to any origin.
      const server = await client.get("/.well-known/oauth-authorization-server", {
        headers: { Origin: "https://inspector.example" },
      });
      expect(server.status()).toBe(200);
      expect(server.headers()["access-control-allow-origin"]).toBe("*");
      const metadata = (await server.json()) as Record<string, unknown>;
      expect(metadata["issuer"]).toBe(origin);
      expect(metadata["code_challenge_methods_supported"]).toEqual(["S256"]);
      const resource = (await (
        await client.get("/.well-known/oauth-protected-resource")
      ).json()) as Record<string, unknown>;
      expect(resource["resource"]).toBe(`${origin}/mcp`);
      expect(resource["authorization_servers"]).toEqual([origin]);
      const preflight = await client.fetch("/api/v1/oauth/token", {
        method: "OPTIONS",
        headers: {
          Origin: "https://inspector.example",
          "Access-Control-Request-Method": "POST",
        },
      });
      expect(preflight.status()).toBe(204);

      // It registers itself.
      const registered = await client.post("/api/v1/oauth/register", {
        data: { client_name: `E2E client ${run}`, redirect_uris: [REDIRECT] },
      });
      expect(registered.status(), await registered.text()).toBe(201);
      const clientId = ((await registered.json()) as { client_id: string }).client_id;

      // The browser goes to the authorization endpoint signed out, signs in, and comes
      // back to the consent page, where the owner allows the client as Scout.
      const { verifier, challenge } = pkce();
      const authorize = new URLSearchParams({
        response_type: "code",
        client_id: clientId,
        redirect_uri: REDIRECT,
        code_challenge: challenge,
        code_challenge_method: "S256",
        scope: "read write",
        state: `state-${run}`,
      });
      const consent = await consentContext.newPage();
      const back = new Promise<URL>((resolve) => {
        void consent.route(`${REDIRECT}**`, async (route) => {
          resolve(new URL(route.request().url()));
          await route.fulfill({ status: 200, contentType: "text/plain", body: "connected" });
        });
      });
      await consent.goto(`/api/v1/oauth/authorize?${authorize.toString()}`);
      await expect(consent.getByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
      await signIn(consent, owner.email, PASSWORD);
      const card = consent.getByRole("region", { name: `Connect E2E client ${run}?` });
      await expect(card).toBeVisible();
      await expect(card.getByText("Then you go back to http://127.0.0.1:43110.")).toBeVisible();
      await card.getByRole("radio", { name: `Scout ${run}, my agent` }).check();
      await expect(card.getByRole("checkbox", { name: "Write" })).toBeChecked();
      await card.getByRole("button", { name: "Allow" }).click();
      const answer = await back;
      expect(answer.searchParams.get("state")).toBe(`state-${run}`);
      expect(answer.searchParams.get("iss")).toBe(origin);
      const code = answer.searchParams.get("code");
      expect(code, "a code").not.toBeNull();

      // It exchanges the code with its verifier, once.
      const exchange = {
        grant_type: "authorization_code",
        client_id: clientId,
        code: code ?? "",
        redirect_uri: REDIRECT,
        code_verifier: verifier,
      };
      const issued = await client.post("/api/v1/oauth/token", { form: exchange });
      expect(issued.status(), await issued.text()).toBe(200);
      const tokens = (await issued.json()) as Tokens;
      expect(tokens.access_token).toMatch(/^nwo_/);
      expect(tokens.refresh_token).toMatch(/^nwr_/);
      expect(tokens.scope).toBe("read write");

      // The token makes a page over REST, as the agent.
      const asScout = await playwright.request.newContext({
        baseURL: origin,
        extraHTTPHeaders: { ...ADDRESS, Authorization: `Bearer ${tokens.access_token}` },
      });
      try {
        const made = await asScout.post(`${base}/pages`, {
          data: { teamspace_id: general, title: `Notes ${run}` },
        });
        expect(made.status(), await made.text()).toBe(201);
        const page = (await made.json()) as { id: string; created_by: string };
        expect(page.created_by).toBe(scout.id);
        const bound = await asScout.get("/api/v1/account/me");
        expect(bound.status()).toBe(403);

        // The owner sees it among the connected apps, and disconnects it.
        await ownerPage.goto("/account/apps");
        const app = ownerPage.getByRole("listitem", { name: `E2E client ${run}` });
        await expect(app.getByText(`Scout ${run}`, { exact: true })).toBeVisible();
        await app
          .getByRole("button", { name: `Disconnect E2E client ${run} from Connected ${run}` })
          .click();
        await ownerPage
          .getByRole("alertdialog")
          .getByRole("button", { name: "Disconnect" })
          .click();
        await expect(ownerPage.getByText(`E2E client ${run} is disconnected.`)).toBeVisible();
        const refused = await asScout.get(`${base}/members`);
        expect(refused.status()).toBe(401);
      } finally {
        await asScout.dispose();
      }
      const reused = await client.post("/api/v1/oauth/token", { form: exchange });
      expect(reused.status()).toBe(400);
    } finally {
      await client.dispose();
    }
  } finally {
    await adminContext.close();
    await consentContext.close();
  }
});
