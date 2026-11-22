import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import type { Schemas } from "@/api/client";
import { ada, defaultAuthConfig, defaultPreferences } from "./fixtures";

export const testMeta: Schemas["Meta"] = { name: "Test instance", version: "0.1.0" };

/**
 * The API as tests see it by default: Ada signed in, the server's default
 * auth settings, and her look left at the defaults. A test overrides a route
 * with server.use().
 */
export const server = setupServer(
  http.get("/api/v1/meta", () => HttpResponse.json(testMeta)),
  http.get("/api/v1/account/me", () => HttpResponse.json(ada)),
  http.get("/api/v1/auth/config", () => HttpResponse.json(defaultAuthConfig)),
  http.get("/api/v1/account/preferences", () => HttpResponse.json(defaultPreferences)),
  http.put("/api/v1/account/preferences", async ({ request }) =>
    HttpResponse.json(await request.json()),
  ),
);
