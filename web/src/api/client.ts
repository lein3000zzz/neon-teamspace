import createClient from "openapi-fetch";
import type { components, paths } from "./schema";

export type Schemas = components["schemas"];

/**
 * The typed client for the `neon` HTTP API. The SPA is served from the same
 * origin as the API (Caddy in production, the Vite proxy in development), so
 * the base URL is this page's origin and the session cookie goes with every
 * request. Being same-origin is also what the server's origin check on
 * requests that change something wants: the browser sends this page's Origin.
 */
export const api = createClient<paths>({
  baseUrl: window.location.origin,
  credentials: "same-origin",
  // Looked up per request rather than captured once, so anything that wraps
  // fetch after this module loads (request mocking in tests) is honoured.
  fetch: (request) => globalThis.fetch(request),
});
