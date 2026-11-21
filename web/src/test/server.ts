import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import type { Schemas } from "@/api/client";

export const testMeta: Schemas["Meta"] = { name: "Test instance", version: "0.1.0" };

/** The API as tests see it by default; a test overrides a route with server.use(). */
export const server = setupServer(http.get("/api/v1/meta", () => HttpResponse.json(testMeta)));
