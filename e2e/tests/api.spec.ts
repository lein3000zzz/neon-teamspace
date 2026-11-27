import { type ApiErrorBody, SERVER_VERSION, expect, test } from "./fixtures";

test("GET /api/v1/meta answers the product, the instance's name and the server's version", async ({
  request,
}) => {
  const response = await request.get("/api/v1/meta");

  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toContain("application/json");
  const body: unknown = await response.json();
  expect(body).toEqual({
    product: "neon-teamspace",
    name: expect.any(String),
    version: SERVER_VERSION,
  });
});

test("an unknown API route gets the error shape with a 404, not the app", async ({ request }) => {
  for (const path of ["/api/v1/no-such-route", "/api/no-such-version", "/api"]) {
    const response = await request.get(path);

    expect(response.status(), path).toBe(404);
    expect(response.headers()["content-type"], path).toContain("application/json");
    const body = (await response.json()) as ApiErrorBody;
    expect(body, path).toEqual({
      error: { code: "not_found", message: expect.any(String) },
    });
  }
});

test("the API description is served beside the API", async ({ request }) => {
  const response = await request.get("/api/v1/openapi.json");

  expect(response.status()).toBe(200);
  const spec = (await response.json()) as { openapi: string; paths: Record<string, unknown> };
  expect(spec.openapi).toMatch(/^3\./);
  expect(Object.keys(spec.paths)).toContain("/api/v1/meta");
});
