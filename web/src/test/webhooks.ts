import { http, HttpResponse, type HttpResponseResolver } from "msw";
import type { Schemas } from "@/api/client";
import type { DeliveryAttempt, IncomingRequest, IncomingWebhook, Webhook } from "@/api/webhooks";
import type { TreePage } from "@/api/pages";
import { ada } from "./fixtures";
import { server } from "./server";
import { people, scout, TEAMSPACE_IDS, type Recorded } from "./workspaces";

const WORKSPACE_ID = "0193a5b2-0000-7000-8000-00000000a001";
const CREATED = "2026-11-20T09:00:00Z";

/** Stand-ins for what the server makes: real ones are 43 random base64url characters long. */
export const SIGNING_SECRET = `whsec_${"s".repeat(43)}`;
export const ROTATED_SECRET = `whsec_${"r".repeat(43)}`;
export const HOOK_PATH = `/hooks/in/nwh_${"t".repeat(43)}`;
export const NEW_HOOK_PATH = `/hooks/in/nwh_${"n".repeat(43)}`;

/** Ada's webhook for herself, on and asking for both events. */
export const adaHook: Webhook = {
  id: "0193a5b2-0000-7000-8000-00000000f001",
  workspace_id: WORKSPACE_ID,
  identity: people.ada,
  url: "https://hooks.example.com/ada",
  events: ["row.assigned", "row.status_changed"],
  include_own: false,
  enabled: true,
  failures: 0,
  created_by: { id: ada.id, display_name: "Ada" },
  created_at: CREATED,
  updated_at: CREATED,
};

/** Scout's webhook, which turned itself off. */
export const scoutHook: Webhook = {
  ...adaHook,
  id: "0193a5b2-0000-7000-8000-00000000f002",
  identity: {
    id: scout.id,
    kind: "agent",
    display_name: "Scout",
    hue: scout.hue,
    owner: { id: ada.id, display_name: "Ada", hue: ada.hue },
  },
  url: "https://agents.example.com/scout",
  events: ["row.assigned"],
  enabled: false,
  disabled_at: CREATED,
  disabled_reason:
    "turned off after 5 deliveries in a row failed; the last: the receiver answered 500",
  failures: 5,
};

/** Bob's webhook, which an owner or admin sees. */
export const bobHook: Webhook = {
  ...adaHook,
  id: "0193a5b2-0000-7000-8000-00000000f003",
  identity: people.bob,
  url: "https://hooks.example.com/bob",
  created_by: { id: people.bob.id, display_name: "Bob" },
};

/** An incoming webhook on Tasks. */
export const ciHook: IncomingWebhook = {
  id: "0193a5b2-0000-7000-8000-00000000f101",
  workspace_id: WORKSPACE_ID,
  name: "CI",
  integration: {
    id: "0193a5b2-0000-7000-8000-00000000c0c1",
    status: "active",
    owner: { id: ada.id, display_name: "Ada", hue: ada.hue },
  },
  target: { id: "0193a5b2-0000-7000-8000-000000009001", kind: "database", title: "Tasks" },
  signed: false,
  enabled: true,
  created_by: { id: ada.id, display_name: "Ada" },
  created_at: CREATED,
  updated_at: CREATED,
};

/** The pages of General a target may be picked from: a database, and a page under it. */
export const generalPages: TreePage[] = [
  {
    id: "0193a5b2-0000-7000-8000-000000009001",
    kind: "database",
    title: "Tasks",
    access: "edit",
    child_count: 0,
    teamspace_id: TEAMSPACE_IDS.general,
  },
  {
    id: "0193a5b2-0000-7000-8000-000000009002",
    kind: "page",
    title: "Changelog",
    access: "edit",
    child_count: 0,
    teamspace_id: TEAMSPACE_IDS.general,
  },
  {
    id: "0193a5b2-0000-7000-8000-000000009003",
    kind: "page",
    title: "Read only",
    access: "view",
    child_count: 0,
    teamspace_id: TEAMSPACE_IDS.general,
  },
];

export const delivered: DeliveryAttempt = {
  id: "0193a5b2-0000-7000-8000-00000000f201",
  delivery_id: "0193a5b2-0000-7000-8000-00000000f301",
  event_id: "0193a5b2-0000-7000-8000-00000000f401",
  event_type: "row.assigned",
  attempt: 2,
  outcome: "delivered",
  status_code: 200,
  latency_ms: 42,
  at: "2026-11-21T10:00:00Z",
};

export const retried: DeliveryAttempt = {
  ...delivered,
  id: "0193a5b2-0000-7000-8000-00000000f202",
  attempt: 1,
  outcome: "retrying",
  status_code: 503,
  error_kind: "http_status",
  excerpt: "down for maintenance",
  at: "2026-11-21T09:59:00Z",
};

export const loggedRequest: IncomingRequest = {
  id: "0193a5b2-0000-7000-8000-00000000f501",
  at: "2026-11-21T10:00:00Z",
  status_code: 200,
  outcome: "ok",
  action: "create_row",
  client_ip: "203.0.113.7",
  latency_ms: 12,
};

type Options = {
  webhooks?: Webhook[];
  incoming?: IncomingWebhook[];
  attempts?: DeliveryAttempt[];
  requests?: IncomingRequest[];
};

/**
 * The webhook routes of the server for Design, holding what changes so pages
 * read back what they did, every change recorded. Run fakeWorkspaces() first,
 * for the workspace and its members.
 */
export function fakeWebhooks(options: Options = {}) {
  const state = {
    webhooks: options.webhooks ?? [],
    incoming: options.incoming ?? [],
  };
  const changes: Recorded[] = [];
  const record = (
    resolver: (body: unknown, params: Record<string, string>) => Response | Promise<Response>,
  ) =>
    (async ({ request, params }) => {
      const text = await request.text();
      const body: unknown = text === "" ? undefined : JSON.parse(text);
      changes.push({ method: request.method, path: new URL(request.url).pathname, body });
      return resolver(body, params as Record<string, string>);
    }) satisfies HttpResponseResolver;
  const base = "/api/v1/workspaces/:id";
  const findWebhook = (id: string | undefined) =>
    state.webhooks.find((webhook) => webhook.id === id);
  const findHook = (id: string | undefined) => state.incoming.find((hook) => hook.id === id);
  const replaceHook = (next: IncomingWebhook) => {
    state.incoming = state.incoming.map((hook) => (hook.id === next.id ? next : hook));
    return next;
  };
  const replaceWebhook = (next: Webhook) => {
    state.webhooks = state.webhooks.map((webhook) => (webhook.id === next.id ? next : webhook));
    return next;
  };

  server.use(
    http.get(`/api/v1/teamspaces/${TEAMSPACE_IDS.general}/pages`, () =>
      HttpResponse.json({ pages: generalPages }),
    ),
    http.get(`${base}/webhooks`, () => HttpResponse.json({ webhooks: state.webhooks })),
    http.post(
      `${base}/webhooks`,
      record((body) => {
        const input = body as Schemas["CreateWebhook"];
        const identity =
          Object.values(people).find((person) => person.id === input.user_id) ?? scoutHook.identity;
        const made: Webhook = {
          ...adaHook,
          id: "0193a5b2-0000-7000-8000-00000000f009",
          identity,
          url: input.url,
          events: input.events,
          include_own: input.include_own ?? false,
        };
        state.webhooks = [...state.webhooks, made];
        return HttpResponse.json({ webhook: made, secret: SIGNING_SECRET }, { status: 201 });
      }),
    ),
    http.patch(
      `${base}/webhooks/:webhook_id`,
      record((body, params) => {
        const found = findWebhook(params["webhook_id"]);
        if (found === undefined) {
          return HttpResponse.json({}, { status: 404 });
        }
        const patch = body as Schemas["UpdateWebhook"];
        const enabled = patch.enabled ?? found.enabled;
        return HttpResponse.json(
          replaceWebhook({
            ...found,
            ...(patch.url ? { url: patch.url } : {}),
            ...(patch.events ? { events: patch.events } : {}),
            ...(patch.include_own === undefined || patch.include_own === null
              ? {}
              : { include_own: patch.include_own }),
            enabled,
            ...(enabled
              ? { disabled_reason: null, disabled_at: null, failures: 0 }
              : { disabled_reason: "turned off by Ada", disabled_at: CREATED }),
          }),
        );
      }),
    ),
    http.delete(
      `${base}/webhooks/:webhook_id`,
      record((_, params) => {
        state.webhooks = state.webhooks.filter((webhook) => webhook.id !== params["webhook_id"]);
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.post(
      `${base}/webhooks/:webhook_id/secret`,
      record((_, params) => {
        const found = findWebhook(params["webhook_id"]);
        const webhook = replaceWebhook({
          ...(found ?? adaHook),
          previous_secret_expires_at: "2026-11-22T09:00:00Z",
        });
        return HttpResponse.json({ webhook, secret: ROTATED_SECRET });
      }),
    ),
    http.post(
      `${base}/webhooks/:webhook_id/test`,
      record(() =>
        HttpResponse.json({
          ...delivered,
          id: "0193a5b2-0000-7000-8000-00000000f209",
          delivery_id: null,
          event_type: "webhook.test",
          attempt: 1,
          latency_ms: 12,
        }),
      ),
    ),
    http.get(`${base}/webhooks/:webhook_id/deliveries`, () =>
      HttpResponse.json({ attempts: options.attempts ?? [] }),
    ),
    http.get(`${base}/incoming-webhooks`, () => HttpResponse.json({ hooks: state.incoming })),
    http.post(
      `${base}/incoming-webhooks`,
      record((body) => {
        const input = body as Schemas["CreateIncomingWebhook"];
        const target = generalPages.find((page) => page.id === input.target_id);
        const hook: IncomingWebhook = {
          ...ciHook,
          id: "0193a5b2-0000-7000-8000-00000000f109",
          name: input.name,
          signed: input.signed ?? false,
          target: {
            id: input.target_id,
            kind: target?.kind === "database" ? "database" : "page",
            title: target?.title ?? null,
          },
        };
        state.incoming = [...state.incoming, hook];
        return HttpResponse.json(
          {
            hook,
            url: HOOK_PATH,
            ...(input.signed ? { signing_secret: SIGNING_SECRET } : {}),
          },
          { status: 201 },
        );
      }),
    ),
    http.patch(
      `${base}/incoming-webhooks/:hook_id`,
      record((body, params) => {
        const found = findHook(params["hook_id"]) ?? ciHook;
        const patch = body as Schemas["UpdateIncomingWebhook"];
        return HttpResponse.json(
          replaceHook({
            ...found,
            ...(patch.name ? { name: patch.name } : {}),
            ...(patch.enabled === undefined || patch.enabled === null
              ? {}
              : { enabled: patch.enabled }),
          }),
        );
      }),
    ),
    http.delete(
      `${base}/incoming-webhooks/:hook_id`,
      record((_, params) => {
        state.incoming = state.incoming.filter((hook) => hook.id !== params["hook_id"]);
        return new HttpResponse(null, { status: 204 });
      }),
    ),
    http.post(
      `${base}/incoming-webhooks/:hook_id/token`,
      record((_, params) =>
        HttpResponse.json({ hook: findHook(params["hook_id"]) ?? ciHook, url: NEW_HOOK_PATH }),
      ),
    ),
    http.post(
      `${base}/incoming-webhooks/:hook_id/secret`,
      record((_, params) => {
        const hook = replaceHook({ ...(findHook(params["hook_id"]) ?? ciHook), signed: true });
        return HttpResponse.json({ hook, signing_secret: ROTATED_SECRET });
      }),
    ),
    http.delete(
      `${base}/incoming-webhooks/:hook_id/secret`,
      record((_, params) =>
        HttpResponse.json(
          replaceHook({ ...(findHook(params["hook_id"]) ?? ciHook), signed: false }),
        ),
      ),
    ),
    http.get(`${base}/incoming-webhooks/:hook_id/requests`, () =>
      HttpResponse.json({ requests: options.requests ?? [] }),
    ),
  );
  return { changes };
}
