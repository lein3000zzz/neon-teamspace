import { http, HttpResponse, ws } from "msw";
import { setupServer } from "msw/node";
import type { Schemas } from "@/api/client";
import { ada, defaultAuthConfig, defaultPreferences } from "./fixtures";

export const testMeta: Schemas["Meta"] = {
  product: "neon-teamspace",
  name: "Test instance",
  version: "0.1.0",
};

/**
 * The updates of a server that has not checked GitHub yet, with no updater set up: nothing
 * newer, so the Admin entry carries no mark.
 */
export const testUpdates: Schemas["Updates"] = {
  current: testMeta.version,
  development: false,
  check: true,
  prereleases: false,
  latest: null,
  newer: false,
  checked_at: null,
  error: null,
  updater: { set_up: false, status: null, error: null },
};

/** Every page's live connection, as the editor opens it; fakeSync() answers on it. */
export const syncLink = ws.link(/\/ws\/pages\/[^/?]+(\?[^/]*)?$/);

/** The inbox's live feed, as the signed-in shell opens it; fakeInboxFeed() answers on it. */
export const inboxLink = ws.link(/\/ws\/inbox$/);

/** Every database's live feed, as its views open it; fakeFeed() answers on it. */
export const feedLink = ws.link(/\/ws\/databases\/[^/?]+$/);

/**
 * The API as tests see it by default: Ada signed in, the server's default
 * auth settings, her look left at the defaults, and no workspace,
 * invitation, page or favorite yet, and to an admin no newer release. A
 * page's live connection, and a database's feed, are taken and never
 * answered. A test overrides a route with server.use().
 */
export const server = setupServer(
  http.get("/api/v1/meta", () => HttpResponse.json(testMeta)),
  http.get("/api/v1/account/me", () => HttpResponse.json(ada)),
  http.get("/api/v1/auth/config", () => HttpResponse.json(defaultAuthConfig)),
  http.get("/api/v1/workspaces", () => HttpResponse.json({ workspaces: [], can_create: true })),
  http.get("/api/v1/account/invitations", () => HttpResponse.json({ invitations: [] })),
  http.get("/api/v1/account/preferences", () => HttpResponse.json(defaultPreferences)),
  http.put("/api/v1/account/preferences", async ({ request }) =>
    HttpResponse.json(await request.json()),
  ),
  http.get("/api/v1/teamspaces/:id/pages", () => HttpResponse.json({ pages: [] })),
  http.get("/api/v1/workspaces/:id/private-pages", () => HttpResponse.json({ pages: [] })),
  http.get("/api/v1/workspaces/:id/shared-pages", () => HttpResponse.json({ pages: [] })),
  http.get("/api/v1/account/favorites", () => HttpResponse.json({ pages: [] })),
  http.get("/api/v1/inbox/count", () => HttpResponse.json({ unread: 0 })),
  http.get("/api/v1/admin/updates", () => HttpResponse.json(testUpdates)),
  http.get("/api/v1/pages/:id/comments", () =>
    HttpResponse.json({ threads: [], may_comment: true, following: false }),
  ),
  syncLink.addEventListener("connection", () => {}),
  feedLink.addEventListener("connection", () => {}),
  inboxLink.addEventListener("connection", () => {}),
);
