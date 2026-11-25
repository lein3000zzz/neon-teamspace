import { screen, waitFor, within } from "@testing-library/react";
import { http } from "msw";
import { describe, expect, it } from "vitest";
import { apiError } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import {
  adaHook,
  bobHook,
  ciHook,
  delivered,
  fakeWebhooks,
  generalPages,
  HOOK_PATH,
  loggedRequest,
  NEW_HOOK_PATH,
  retried,
  ROTATED_SECRET,
  scoutHook,
  SIGNING_SECRET,
} from "@/test/webhooks";
import { design, fakeWorkspaces, people, scout } from "@/test/workspaces";

const ID = design().id;
const base = `/api/v1/workspaces/${ID}`;

async function section(name: string) {
  return within(await screen.findByRole("region", { name }));
}

async function webhookCard(name: string, url: string) {
  const outgoing = await section("Outgoing webhooks");
  return within(await outgoing.findByRole("listitem", { name: `Webhook for ${name} to ${url}` }));
}

async function incomingCard(name: string) {
  const incoming = await section("Incoming webhooks");
  return within(await incoming.findByRole("listitem", { name }));
}

describe("outgoing webhooks", () => {
  it("lists each with whose it is, where it goes and what it asks for, and why one is off", async () => {
    fakeWorkspaces({ agents: [scout] });
    fakeWebhooks({ webhooks: [adaHook, scoutHook] });
    renderApp("/w/design/settings/webhooks");

    const mine = await webhookCard("Ada", adaHook.url);
    expect(mine.getByText("On")).toBeInTheDocument();
    expect(mine.getByText("Assigned to a row")).toBeInTheDocument();
    expect(mine.getByText("Status changed on a row they are assigned to")).toBeInTheDocument();
    expect(mine.queryByText("Turned off")).toBeNull();

    const off = await webhookCard("Scout", scoutHook.url);
    expect(off.getByText("Off")).toBeInTheDocument();
    expect(off.getByText("· via Ada")).toBeInTheDocument();
    expect(off.getByText("Turned off")).toBeInTheDocument();
    expect(off.getByText(scoutHook.disabled_reason ?? "")).toBeInTheDocument();
    expect(off.getByRole("button", { name: `Turn on: ${scoutHook.url}` })).toBeInTheDocument();
  });

  it("registers one, showing its secret once", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks();
    const { user } = renderApp("/w/design/settings/webhooks");

    expect(await screen.findByText("No outgoing webhooks yet.")).toBeInTheDocument();
    const form = within(await screen.findByRole("form", { name: "Register a webhook" }));
    await user.click(form.getByRole("checkbox", { name: "Assigned to a row" }));
    await user.click(
      form.getByRole("checkbox", { name: "Status changed on a row they are assigned to" }),
    );
    await user.click(form.getByRole("button", { name: "Register webhook" }));
    expect(form.getByLabelText("For")).toHaveAccessibleDescription(/Fill this in./);
    expect(form.getByText("Choose at least one event.")).toBeInTheDocument();
    expect(api.changes).toEqual([]);

    await form.findByRole("option", { name: "Ada" });
    await user.selectOptions(form.getByLabelText("For"), "Ada");
    await user.type(form.getByLabelText("URL"), "https://hooks.example.com/new");
    await user.click(form.getByRole("checkbox", { name: "Assigned to a row" }));
    await user.click(form.getByRole("checkbox", { name: "Also for their own changes" }));
    await user.click(form.getByRole("button", { name: "Register webhook" }));

    const shown = await screen.findByText("Webhook registered");
    expect(shown).toBeInTheDocument();
    expect(screen.getByDisplayValue(SIGNING_SECRET)).toHaveAttribute("readonly");
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `${base}/webhooks`,
        body: {
          user_id: people.ada.id,
          url: "https://hooks.example.com/new",
          events: ["row.assigned"],
          include_own: true,
        },
      },
    ]);
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByDisplayValue(SIGNING_SECRET)).toBeNull();
    expect(await webhookCard("Ada", "https://hooks.example.com/new")).toBeTruthy();
  });

  it("offers a member themselves and their own agents alone, and no incoming webhooks", async () => {
    fakeWorkspaces({ role: "member", agents: [scout] });
    fakeWebhooks();
    renderApp("/w/design/settings/webhooks");

    const form = within(await screen.findByRole("form", { name: "Register a webhook" }));
    await waitFor(() =>
      expect(
        within(form.getByLabelText("For"))
          .getAllByRole("option")
          .map((option) => option.textContent),
      ).toEqual(["Choose who", "Ada", "Scout · via Ada"]),
    );
    const incoming = await section("Incoming webhooks");
    expect(incoming.getByText("Owners and admins make incoming webhooks.")).toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Make an incoming webhook" })).toBeNull();
  });

  it("says when the server keeps no secrets", async () => {
    fakeWorkspaces();
    fakeWebhooks();
    server.use(
      http.post(`${base}/webhooks`, () =>
        apiError(409, "no_secret_key", "secrets.keys is not set"),
      ),
    );
    const { user } = renderApp("/w/design/settings/webhooks");

    const form = within(await screen.findByRole("form", { name: "Register a webhook" }));
    await form.findByRole("option", { name: "Ada" });
    await user.selectOptions(form.getByLabelText("For"), "Ada");
    await user.type(form.getByLabelText("URL"), "https://hooks.example.com/new");
    await user.click(form.getByRole("button", { name: "Register webhook" }));

    expect(await form.findByRole("alert")).toHaveTextContent(/an instance admin sets secrets.keys/);
  });

  it("shows the server's refusal of an address", async () => {
    fakeWorkspaces();
    fakeWebhooks();
    server.use(
      http.post(`${base}/webhooks`, () =>
        apiError(
          400,
          "address_refused",
          "the URL's host is a private address, which webhooks may not reach: see webhooks.allow_private",
        ),
      ),
    );
    const { user } = renderApp("/w/design/settings/webhooks");

    const form = within(await screen.findByRole("form", { name: "Register a webhook" }));
    await form.findByRole("option", { name: "Ada" });
    await user.selectOptions(form.getByLabelText("For"), "Ada");
    await user.type(form.getByLabelText("URL"), "http://10.0.0.5/hook");
    await user.click(form.getByRole("button", { name: "Register webhook" }));

    expect(await form.findByText(/a private address, which webhooks may not reach/)).toBeTruthy();
  });

  it("sends a test event and says how it went", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks({ webhooks: [adaHook] });
    const { user } = renderApp("/w/design/settings/webhooks");

    const card = await webhookCard("Ada", adaHook.url);
    await user.click(card.getByRole("button", { name: `Send a test event to ${adaHook.url}` }));

    expect(await card.findByText("Test event delivered: 200 in 12 ms.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "POST", path: `${base}/webhooks/${adaHook.id}/test`, body: undefined },
    ]);

    server.use(
      http.post(`${base}/webhooks/:webhook_id/test`, () =>
        Response.json({
          ...delivered,
          delivery_id: null,
          outcome: "failed",
          status_code: 500,
          error_kind: "http_status",
        }),
      ),
    );
    await user.click(card.getByRole("button", { name: `Send a test event to ${adaHook.url}` }));
    expect(await card.findByText("Test event failed: an error status, 500.")).toBeInTheDocument();
  });

  it("rotates its secret once confirmed, showing the new one once", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks({ webhooks: [adaHook] });
    const { user } = renderApp("/w/design/settings/webhooks");

    const card = await webhookCard("Ada", adaHook.url);
    await user.click(
      card.getByRole("button", { name: `Rotate the secret of the webhook to ${adaHook.url}` }),
    );
    const dialog = within(
      await screen.findByRole("alertdialog", { name: "Rotate the signing secret?" }),
    );
    expect(dialog.getByText(/keeps signing deliveries beside it for a day/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Rotate secret" }));

    expect(await card.findByDisplayValue(ROTATED_SECRET)).toHaveAttribute("readonly");
    expect(card.getByText(/The previous secret signs too until/)).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "POST", path: `${base}/webhooks/${adaHook.id}/secret`, body: undefined },
    ]);
    await user.click(card.getByRole("button", { name: "Done" }));
    expect(card.queryByDisplayValue(ROTATED_SECRET)).toBeNull();
  });

  it("turns one off and on, changes it, and deletes it once confirmed", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks({ webhooks: [adaHook] });
    const { user } = renderApp("/w/design/settings/webhooks");

    let card = await webhookCard("Ada", adaHook.url);
    await user.click(card.getByRole("button", { name: `Turn off: ${adaHook.url}` }));
    card = await webhookCard("Ada", adaHook.url);
    expect(await card.findByText("turned off by Ada")).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: `Turn on: ${adaHook.url}` }));
    expect(await card.findByRole("button", { name: `Turn off: ${adaHook.url}` })).toBeTruthy();

    await user.click(card.getByRole("button", { name: `Edit the webhook to ${adaHook.url}` }));
    const form = within(card.getByRole("form", { name: `Edit the webhook to ${adaHook.url}` }));
    await user.clear(form.getByLabelText("URL"));
    await user.type(form.getByLabelText("URL"), "https://hooks.example.com/moved");
    await user.click(
      form.getByRole("checkbox", { name: "Status changed on a row they are assigned to" }),
    );
    await user.click(form.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Webhook saved.")).toBeInTheDocument();

    const moved = await webhookCard("Ada", "https://hooks.example.com/moved");
    await user.click(
      moved.getByRole("button", { name: "Delete the webhook to https://hooks.example.com/moved" }),
    );
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete this webhook?" }));
    await user.click(dialog.getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("Webhook deleted.")).toBeInTheDocument();
    expect(await screen.findByText("No outgoing webhooks yet.")).toBeInTheDocument();

    expect(api.changes).toEqual([
      { method: "PATCH", path: `${base}/webhooks/${adaHook.id}`, body: { enabled: false } },
      { method: "PATCH", path: `${base}/webhooks/${adaHook.id}`, body: { enabled: true } },
      {
        method: "PATCH",
        path: `${base}/webhooks/${adaHook.id}`,
        body: {
          url: "https://hooks.example.com/moved",
          events: ["row.assigned"],
          include_own: false,
        },
      },
      { method: "DELETE", path: `${base}/webhooks/${adaHook.id}`, body: undefined },
    ]);
  });

  it("shows its delivery log, each attempt's outcome, status and the start of the answer", async () => {
    fakeWorkspaces();
    fakeWebhooks({ webhooks: [adaHook], attempts: [delivered, retried] });
    const { user } = renderApp("/w/design/settings/webhooks");

    const card = await webhookCard("Ada", adaHook.url);
    await user.click(
      card.getByRole("button", { name: `Delivery log of the webhook to ${adaHook.url}` }),
    );
    const log = within(
      await card.findByRole("region", { name: `Delivery log of the webhook to ${adaHook.url}` }),
    );
    const entries = log.getAllByRole("listitem");
    expect(entries).toHaveLength(2);
    expect(within(entries[0] as HTMLElement).getByText("Delivered")).toBeInTheDocument();
    expect(within(entries[0] as HTMLElement).getByText("42 ms")).toBeInTheDocument();
    const second = within(entries[1] as HTMLElement);
    expect(second.getByText("Will retry")).toBeInTheDocument();
    expect(second.getByText("503")).toBeInTheDocument();
    expect(second.getByText("an error status")).toBeInTheDocument();
    expect(second.getByText("down for maintenance")).toBeInTheDocument();
  });

  it("opens on an agent's webhooks from its card, ready to register one for it", async () => {
    fakeWorkspaces({ agents: [scout] });
    fakeWebhooks({ webhooks: [adaHook, scoutHook] });
    const { user } = renderApp("/w/design/settings/agents");

    const agents = within(await screen.findByRole("region", { name: "Agents" }));
    const card = within(await agents.findByRole("listitem", { name: "Scout" }));
    await user.click(card.getByRole("link", { name: "Webhooks of Scout" }));

    expect(await screen.findByText("The webhooks of Scout.")).toBeInTheDocument();
    const outgoing = await section("Outgoing webhooks");
    expect(await webhookCard("Scout", scoutHook.url)).toBeTruthy();
    expect(
      outgoing.queryByRole("listitem", { name: `Webhook for Ada to ${adaHook.url}` }),
    ).toBeNull();
    const form = within(screen.getByRole("form", { name: "Register a webhook" }));
    expect(form.getByLabelText("For")).toHaveValue(scout.id);
    await user.click(screen.getByRole("link", { name: "Show every webhook" }));
    expect(await webhookCard("Ada", adaHook.url)).toBeTruthy();
  });

  it("lists everyone's for an owner", async () => {
    fakeWorkspaces();
    fakeWebhooks({ webhooks: [adaHook, bobHook] });
    renderApp("/w/design/settings/webhooks");

    expect(await webhookCard("Bob", bobHook.url)).toBeTruthy();
    expect((await webhookCard("Bob", bobHook.url)).getByText("Registered by Bob")).toBeTruthy();
  });
});

describe("incoming webhooks", () => {
  it("makes one on a database or page the user may edit, showing its URL once", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks();
    const { user } = renderApp("/w/design/settings/webhooks");

    expect(await screen.findByText("No incoming webhooks yet.")).toBeInTheDocument();
    const form = within(await screen.findByRole("form", { name: "Make an incoming webhook" }));
    const target = await form.findByLabelText("Target");
    await waitFor(() =>
      expect(
        within(target)
          .getAllByRole("option")
          .map((option) => option.textContent),
      ).toEqual([
        "Choose a database or page",
        "Tasks · Database · General",
        "Changelog · Page · General",
      ]),
    );
    await user.click(form.getByRole("button", { name: "Make incoming webhook" }));
    expect(form.getByLabelText("Name")).toHaveAccessibleDescription(/Fill this in./);

    await user.type(form.getByLabelText("Name"), " Deploys ");
    await user.selectOptions(target, "Changelog · Page · General");
    await user.click(form.getByRole("checkbox", { name: "Take only signed requests" }));
    await user.click(form.getByRole("button", { name: "Make incoming webhook" }));

    expect(await screen.findByText("Deploys is made")).toBeInTheDocument();
    expect(screen.getByDisplayValue(`${window.location.origin}${HOOK_PATH}`)).toHaveAttribute(
      "readonly",
    );
    expect(screen.getByDisplayValue(SIGNING_SECRET)).toHaveAttribute("readonly");
    expect(api.changes).toEqual([
      {
        method: "POST",
        path: `${base}/incoming-webhooks`,
        body: { name: "Deploys", target_id: generalPages[1]?.id, signed: true },
      },
    ]);
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByDisplayValue(SIGNING_SECRET)).toBeNull();
    const card = await incomingCard("Deploys");
    expect(card.getByText("Page: Changelog")).toBeInTheDocument();
    expect(card.getByText("Signed")).toBeInTheDocument();
  });

  it("says when the workspace is at its agent limit", async () => {
    fakeWorkspaces();
    fakeWebhooks();
    server.use(
      http.post(`${base}/incoming-webhooks`, () =>
        apiError(409, "agent_limit", "this workspace has reached its limit of 1 agents"),
      ),
    );
    const { user } = renderApp("/w/design/settings/webhooks");

    const form = within(await screen.findByRole("form", { name: "Make an incoming webhook" }));
    await user.type(form.getByLabelText("Name"), "CI");
    await waitFor(() =>
      expect(within(form.getByLabelText("Target")).getAllByRole("option")).toHaveLength(3),
    );
    await user.selectOptions(form.getByLabelText("Target"), "Tasks · Database · General");
    await user.click(form.getByRole("button", { name: "Make incoming webhook" }));

    expect(await form.findByRole("alert")).toHaveTextContent(
      "This workspace has as many agents and integrations as its limit allows.",
    );
  });

  it("gives one a new URL and a signing secret, each shown once, and stops checking signatures", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks({ incoming: [ciHook] });
    const { user } = renderApp("/w/design/settings/webhooks");

    let card = await incomingCard("CI");
    expect(card.getByText("Database: Tasks")).toBeInTheDocument();
    expect(card.getByText("Unsigned")).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: "Give CI a new URL" }));
    let dialog = within(await screen.findByRole("alertdialog", { name: "Give CI a new URL?" }));
    expect(dialog.getByText(/The old URL stops working at once/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "New URL" }));
    expect(
      await card.findByDisplayValue(`${window.location.origin}${NEW_HOOK_PATH}`),
    ).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: "Done" }));

    await user.click(card.getByRole("button", { name: "Make a signing secret for CI" }));
    dialog = within(
      await screen.findByRole("alertdialog", { name: "Make CI a new signing secret?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Require signatures" }));
    expect(await card.findByDisplayValue(ROTATED_SECRET)).toBeInTheDocument();
    card = await incomingCard("CI");
    expect(await card.findByText("Signed")).toBeInTheDocument();

    await user.click(card.getByRole("button", { name: "Stop checking the signatures of CI" }));
    dialog = within(
      await screen.findByRole("alertdialog", { name: "Stop checking the signatures of CI?" }),
    );
    await user.click(dialog.getByRole("button", { name: "Stop checking signatures" }));
    expect(await card.findByText("Unsigned")).toBeInTheDocument();

    expect(api.changes.map((change) => `${change.method} ${change.path}`)).toEqual([
      `POST ${base}/incoming-webhooks/${ciHook.id}/token`,
      `POST ${base}/incoming-webhooks/${ciHook.id}/secret`,
      `DELETE ${base}/incoming-webhooks/${ciHook.id}/secret`,
    ]);
  });

  it("shows its last requests, and why a suspended one refuses", async () => {
    fakeWorkspaces();
    fakeWebhooks({
      incoming: [{ ...ciHook, integration: { ...ciHook.integration, status: "suspended" } }],
      requests: [loggedRequest],
    });
    const { user } = renderApp("/w/design/settings/webhooks");

    const card = await incomingCard("CI");
    expect(card.getByText(/it refuses what is sent until that ends/)).toBeInTheDocument();
    await user.click(card.getByRole("button", { name: "Requests of CI" }));
    const log = within(await card.findByRole("region", { name: "Requests of CI" }));
    expect(log.getByText("ok")).toBeInTheDocument();
    expect(log.getByText("create_row")).toBeInTheDocument();
    expect(log.getByText("203.0.113.7")).toBeInTheDocument();
  });

  it("renames one, turns it off, and deletes it once confirmed", async () => {
    fakeWorkspaces();
    const api = fakeWebhooks({ incoming: [ciHook] });
    const { user } = renderApp("/w/design/settings/webhooks");

    let card = await incomingCard("CI");
    await user.click(card.getByRole("button", { name: "Rename CI" }));
    const form = within(card.getByRole("form", { name: "Rename CI" }));
    await user.clear(form.getByLabelText("Name"));
    await user.type(form.getByLabelText("Name"), "Builds");
    await user.click(form.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Builds is saved.")).toBeInTheDocument();

    card = await incomingCard("Builds");
    await user.click(card.getByRole("button", { name: "Turn off: Builds" }));
    card = await incomingCard("Builds");
    expect(await card.findByText("Turned off")).toBeInTheDocument();

    await user.click(card.getByRole("button", { name: "Delete Builds" }));
    const dialog = within(await screen.findByRole("alertdialog", { name: "Delete Builds?" }));
    await user.click(dialog.getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("Builds is deleted.")).toBeInTheDocument();
    expect(api.changes).toEqual([
      { method: "PATCH", path: `${base}/incoming-webhooks/${ciHook.id}`, body: { name: "Builds" } },
      { method: "PATCH", path: `${base}/incoming-webhooks/${ciHook.id}`, body: { enabled: false } },
      { method: "DELETE", path: `${base}/incoming-webhooks/${ciHook.id}`, body: undefined },
    ]);
  });
});
