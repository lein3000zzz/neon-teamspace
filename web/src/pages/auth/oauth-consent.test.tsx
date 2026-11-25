import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConsentInfo, ConsentOption } from "@/api/oauth";
import { leaveFor } from "@/lib/leave";
import { ada, apiError, fakeSession, signedOut } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { design, scout } from "@/test/workspaces";

vi.mock("@/lib/leave", () => ({ leaveFor: vi.fn<(address: string) => void>() }));

const CLIENT = "0193a5b2-0000-7000-8000-00000000d001";
const REDIRECT = "http://127.0.0.1:43110/callback";

/** The request a client sent the browser with, as the authorization endpoint passes it on. */
const request = {
  response_type: "code",
  client_id: CLIENT,
  redirect_uri: REDIRECT,
  code_challenge: "c".repeat(43),
  code_challenge_method: "S256",
  scope: "read write",
  state: "opaque-state",
};
const consentPath = `/oauth/consent?${new URLSearchParams(request).toString()}`;

/** Design as the consent page offers it to Ada, a member there with her agent Scout. */
function option(patch: Partial<ConsentOption> = {}): ConsentOption {
  const workspace = design("member");
  return {
    id: workspace.id,
    name: workspace.name,
    slug: workspace.slug,
    icon: workspace.icon ?? null,
    hue: workspace.hue,
    role: "member",
    person_scopes: ["read", "write"],
    agents_on: true,
    agents: [scout],
    agent_scopes: ["read", "write"],
    ...patch,
  };
}

function consentOf(workspaces: ConsentOption[], patch: Partial<ConsentInfo> = {}): ConsentInfo {
  return {
    client: { id: CLIENT, name: "Claude Code" },
    redirect_uri: REDIRECT,
    redirect_host: "http://127.0.0.1:43110",
    requested_scopes: ["read", "write"],
    resource: "mcp",
    workspaces,
    ...patch,
  };
}

/** The server's consent routes, answering with `consent`, recording what the page sent. */
function fakeConsent(consent: ConsentInfo) {
  const sent = { consent: [] as unknown[], approve: [] as unknown[], deny: [] as unknown[] };
  server.use(
    http.post("/api/v1/oauth/consent", async ({ request: asked }) => {
      sent.consent.push(await asked.json());
      return HttpResponse.json(consent);
    }),
    http.post("/api/v1/oauth/consent/approve", async ({ request: asked }) => {
      sent.approve.push(await asked.json());
      return HttpResponse.json({ redirect_to: `${REDIRECT}?code=c0de&state=opaque-state` });
    }),
    http.post("/api/v1/oauth/consent/deny", async ({ request: asked }) => {
      sent.deny.push(await asked.json());
      return HttpResponse.json({ redirect_to: `${REDIRECT}?error=access_denied` });
    }),
  );
  return sent;
}

/** The card of the consent page. */
async function card(name = "Connect Claude Code?") {
  return within(await screen.findByRole("region", { name }));
}

beforeEach(() => {
  vi.mocked(leaveFor).mockClear();
});

describe("the consent page", () => {
  it("names the app and where the browser goes back to, acting as the person to start", async () => {
    const sent = fakeConsent(consentOf([option()]));
    renderApp(consentPath);

    const page = await card();
    expect(page.getByText(/what the app calls itself: nobody checked it/)).toBeInTheDocument();
    expect(page.getByText("Then you go back to http://127.0.0.1:43110.")).toBeInTheDocument();
    expect(page.getByText("Signed in as ada@example.com.")).toBeInTheDocument();
    expect(page.getByLabelText("Workspace")).toHaveDisplayValue("Design");
    expect(page.getByRole("radio", { name: "Me, Ada" })).toBeChecked();
    expect(page.getByRole("radio", { name: "Scout, my agent" })).not.toBeChecked();
    const read = page.getByRole("checkbox", { name: "Read" });
    expect(read).toBeChecked();
    expect(read).toBeDisabled();
    expect(page.getByRole("checkbox", { name: "Write" })).toBeChecked();
    expect(page.queryByRole("checkbox", { name: "Admin" })).toBeNull();
    expect(sent.consent).toEqual([request]);
  });

  it("approves as an agent with the scopes chosen, then sends the browser back", async () => {
    const sent = fakeConsent(consentOf([option()]));
    const { user } = renderApp(consentPath);

    const page = await card();
    await user.click(page.getByRole("radio", { name: "Scout, my agent" }));
    await user.click(page.getByRole("checkbox", { name: "Write" }));
    await user.click(page.getByRole("button", { name: "Allow" }));

    await waitFor(() =>
      expect(leaveFor).toHaveBeenCalledWith(`${REDIRECT}?code=c0de&state=opaque-state`),
    );
    expect(sent.approve).toEqual([
      { request, workspace_id: design().id, agent_id: scout.id, scopes: ["read"] },
    ]);
    expect(await screen.findByRole("heading", { name: "Back to the app" })).toBeVisible();
  });

  it("approves as the person, with what the client asked for ticked", async () => {
    const sent = fakeConsent(consentOf([option()], { requested_scopes: ["read"] }));
    const { user } = renderApp(consentPath);

    const page = await card();
    expect(page.getByRole("checkbox", { name: "Write" })).not.toBeChecked();
    await user.click(page.getByRole("button", { name: "Allow" }));

    await waitFor(() => expect(sent.approve).toHaveLength(1));
    expect(sent.approve[0]).toEqual({ request, workspace_id: design().id, scopes: ["read"] });
  });

  it("offers admin to owners and admins acting as themselves, and to no agent", async () => {
    fakeConsent(
      consentOf([option({ role: "owner", person_scopes: ["read", "write", "admin"] })], {
        requested_scopes: ["read", "write", "admin"],
      }),
    );
    const { user } = renderApp(consentPath);

    const page = await card();
    expect(page.getByRole("checkbox", { name: "Admin" })).toBeChecked();
    await user.click(page.getByRole("radio", { name: "Scout, my agent" }));
    expect(page.queryByRole("checkbox", { name: "Admin" })).toBeNull();
    expect(page.getByRole("checkbox", { name: "Write" })).toBeChecked();
  });

  it("offers only the identities the settings allow", async () => {
    fakeConsent(
      consentOf([
        option({ person_scopes: null }),
        option({
          id: "0193a5b2-0000-7000-8000-00000000a002",
          name: "Studio",
          agents_on: false,
          agents: [],
        }),
      ]),
    );
    const { user } = renderApp(consentPath);

    const page = await card();
    expect(page.queryByRole("radio", { name: "Me, Ada" })).toBeNull();
    expect(page.getByRole("radio", { name: "Scout, my agent" })).toBeChecked();
    await user.selectOptions(page.getByLabelText("Workspace"), "Studio");
    expect(page.getByRole("radio", { name: "Me, Ada" })).toBeChecked();
    expect(page.queryByRole("radio", { name: "Scout, my agent" })).toBeNull();
    expect(
      page.getByText("Acting as an agent is turned off in this workspace."),
    ).toBeInTheDocument();
  });

  it("says when the person has no agent that works", async () => {
    fakeConsent(consentOf([option({ agents: [] })]));
    renderApp(consentPath);

    const page = await card();
    expect(
      page.getByText("You have no agent here that works. Make one in the workspace's settings."),
    ).toBeInTheDocument();
  });

  it("says when no workspace offers anything, and denies from there", async () => {
    const sent = fakeConsent(consentOf([option({ person_scopes: null, agents_on: false })]));
    const { user } = renderApp(consentPath);

    const page = await card("Nowhere to connect it");
    await user.click(page.getByRole("button", { name: "Deny" }));

    await waitFor(() => expect(leaveFor).toHaveBeenCalledWith(`${REDIRECT}?error=access_denied`));
    expect(sent.deny).toEqual([{ request }]);
  });

  it("denies, sending the browser back with access_denied", async () => {
    const sent = fakeConsent(consentOf([option()]));
    const { user } = renderApp(consentPath);

    const page = await card();
    await user.click(page.getByRole("button", { name: "Deny" }));

    await waitFor(() => expect(leaveFor).toHaveBeenCalledWith(`${REDIRECT}?error=access_denied`));
    expect(sent.deny).toEqual([{ request }]);
    expect(sent.approve).toEqual([]);
  });

  it("reports a refused approval and stays", async () => {
    fakeConsent(consentOf([option()]));
    server.use(
      http.post("/api/v1/oauth/consent/approve", () =>
        apiError(403, "identity_unavailable", "clients do not act as agents in this workspace"),
      ),
    );
    const { user } = renderApp(consentPath);

    const page = await card();
    await user.click(page.getByRole("radio", { name: "Scout, my agent" }));
    await user.click(page.getByRole("button", { name: "Allow" }));

    expect(await page.findByRole("alert")).toHaveTextContent(
      "Clients do not act as agents in this workspace.",
    );
    expect(leaveFor).not.toHaveBeenCalled();
  });

  it("sends someone signed out to sign in, and back to the request", async () => {
    const session = fakeSession(ada, "correct horse battery");
    server.use(...session.handlers);
    fakeConsent(consentOf([option()]));
    const { user, router } = renderApp(consentPath);

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/login");
    await user.type(screen.getByLabelText("Email"), "ada@example.com");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(await card()).toBeTruthy();
    expect(router.state.location.pathname).toBe("/oauth/consent");
    expect(router.state.location.search).toMatchObject(request);
  });

  it("shows the problem the authorization endpoint found, to anyone, asking nothing", async () => {
    server.use(signedOut);
    const sent = fakeConsent(consentOf([option()]));
    renderApp("/oauth/consent?error=redirect_uri_mismatch");

    const page = await card("This connection request does not work");
    expect(
      page.getByText(
        "The request would go back to an address the app did not register, so it goes nowhere.",
      ),
    ).toBeInTheDocument();
    expect(sent.consent).toEqual([]);
  });

  it("shows the problem the server finds in the request", async () => {
    server.use(
      http.post("/api/v1/oauth/consent", () =>
        apiError(400, "pkce_required", "PKCE is required, with the S256 method"),
      ),
    );
    renderApp(consentPath);

    const page = await card("This connection request does not work");
    expect(
      page.getByText("The request lacks PKCE with the S256 method, which this server requires."),
    ).toBeInTheDocument();
  });
});
