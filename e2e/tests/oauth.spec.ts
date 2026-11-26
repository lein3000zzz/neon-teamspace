import { account, callApi, clientAddress, makeAccounts, PASSWORD } from "./accounts";
import { ADMIN, expect, signIn, test } from "./fixtures";
import { allowAsAgent, pkce, REDIRECT, type Tokens } from "./oauth-client";

test.skip(ADMIN === undefined, "needs the first admin that make e2e creates");

const ADDRESS = clientAddress(17);

test.use({ extraHTTPHeaders: ADDRESS });

type Workspace = { id: string; slug: string };
type Agent = { id: string; name: string };
type Teamspaces = { teamspaces: { id: string }[] };

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
      const app = `E2E client ${run}`;
      const agent = `Scout ${run}`;
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
      const answer = await allowAsAgent(consent, authorize, app, agent, owner.email);
      expect(answer.searchParams.get("state")).toBe(`state-${run}`);
      expect(answer.searchParams.get("iss")).toBe(origin);
      const code = answer.searchParams.get("code");
      expect(code, "a code").not.toBeNull();

      // It exchanges the code with its verifier.
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

        // The owner sees it among the connected apps.
        await ownerPage.goto("/account/apps");
        const listed = ownerPage.getByRole("listitem", { name: app });
        await expect(listed.getByText(agent, { exact: true })).toBeVisible();

        // The code exchanged a second time is refused, and revokes everything issued from
        // it: the token stops, and the app is no longer connected.
        const reused = await client.post("/api/v1/oauth/token", { form: exchange });
        expect(reused.status()).toBe(400);
        expect(((await reused.json()) as { error: string }).error).toBe("invalid_grant");
        const revoked = await asScout.get(`${base}/members`);
        expect(revoked.status()).toBe(401);
        await ownerPage.reload();
        await expect(ownerPage.getByText("No apps are connected.")).toBeVisible();
      } finally {
        await asScout.dispose();
      }

      // Connected again, at once as the owner is signed in, then disconnected by the owner.
      const again = await allowAsAgent(consent, authorize, app, agent);
      const reissued = await client.post("/api/v1/oauth/token", {
        form: { ...exchange, code: again.searchParams.get("code") ?? "" },
      });
      expect(reissued.status(), await reissued.text()).toBe(200);
      const second = (await reissued.json()) as Tokens;
      const asScoutAgain = await playwright.request.newContext({
        baseURL: origin,
        extraHTTPHeaders: { ...ADDRESS, Authorization: `Bearer ${second.access_token}` },
      });
      try {
        expect((await asScoutAgain.get(`${base}/members`)).status()).toBe(200);
        await ownerPage.reload();
        const listed = ownerPage.getByRole("listitem", { name: app });
        await listed
          .getByRole("button", { name: `Disconnect ${app} from Connected ${run}` })
          .click();
        await ownerPage
          .getByRole("alertdialog")
          .getByRole("button", { name: "Disconnect" })
          .click();
        await expect(ownerPage.getByText(`${app} is disconnected.`)).toBeVisible();
        const refused = await asScoutAgain.get(`${base}/members`);
        expect(refused.status()).toBe(401);
      } finally {
        await asScoutAgain.dispose();
      }
    } finally {
      await client.dispose();
    }
  } finally {
    await adminContext.close();
    await consentContext.close();
  }
});
