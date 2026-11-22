import { APP_CSP, expect, test } from "./fixtures";

/** Headers every answer from the web origin carries. */
function expectSecurityHeaders(headers: Record<string, string>, label: string) {
  expect(headers["x-content-type-options"], label).toBe("nosniff");
  expect(headers["x-frame-options"], label).toBe("DENY");
  expect(headers["referrer-policy"], label).toBe("strict-origin-when-cross-origin");
  expect(headers["permissions-policy"], label).toContain("camera=()");
  expect(headers["permissions-policy"], label).toContain("microphone=()");
  expect(headers["permissions-policy"], label).toContain("geolocation=()");
  // Over plain HTTP: HSTS is only sent over HTTPS, when Caddy has a domain.
  expect(headers["strict-transport-security"], label).toBeUndefined();
  expect(headers["server"], label).toBeUndefined();
}

test("the app, the docs and the API carry the security headers", async ({ request }) => {
  for (const path of ["/", "/account/appearance", "/docs/", "/api/v1/meta", "/missing.js"]) {
    const response = await request.get(path);

    expectSecurityHeaders(response.headers(), path);
  }
  expect((await request.get("/")).headers()["content-security-policy"]).toBe(APP_CSP);
  expect((await request.get("/styleguide")).headers()["content-security-policy"]).toBe(APP_CSP);
});

test("index.html is checked on every load and hashed assets are cached for good", async ({
  request,
}) => {
  const index = await request.get("/");
  expect(index.headers()["cache-control"]).toBe("no-cache");

  const html = await index.text();
  const assets = [...html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/g)].map((match) => match[1]);
  expect(assets.length).toBeGreaterThan(0);
  for (const asset of assets) {
    const response = await request.get(asset ?? "");

    expect(response.status(), asset).toBe(200);
    expect(response.headers()["cache-control"], asset).toBe("public, max-age=31536000, immutable");
  }
  expect((await request.get("/theme.js")).headers()["cache-control"]).toBe("no-cache");
});

test("missing files are 404s, not the app, and never cached", async ({ request }) => {
  for (const path of [
    "/assets/missing.js",
    "/assets/missing",
    "/missing.png",
    "/fonts/missing",
    "/docs/_astro/missing.js",
    "/docs/missing/",
  ]) {
    const response = await request.get(path);

    expect(response.status(), path).toBe(404);
    expect(await response.text(), path).not.toContain('<div id="root">');
    // A 404 cached under a hashed asset's path would outlive the asset's next deployment.
    expect(response.headers()["cache-control"], path).toBe("no-store");
  }
});

test("the admin listener's paths are not reachable through the web origin", async ({ request }) => {
  const app = await (await request.get("/")).text();

  // Caddy never passes them to the server, so they are routes of the app like any other
  // unknown path, answered with its index.html.
  for (const path of [
    "/metrics",
    "/readyz",
    "/livez",
    "/debug/pprof/heap",
    "/debug/pprof/profile?seconds=1",
  ]) {
    const response = await request.get(path);

    expect(response.headers()["content-type"], path).toContain("text/html");
    expect(await response.text(), path).toBe(app);
  }
});

test("the third-party notices and the font licenses are served", async ({ request }) => {
  const notices = await request.get("/THIRD_PARTY_NOTICES.txt");
  expect(notices.status()).toBe(200);
  expect(notices.headers()["content-type"]).toContain("text/plain");
  const text = await notices.text();
  expect(text).toContain("react");
  expect(text).toContain("License: MIT");

  const font = await request.get("/fonts/ia-writer-mono/LICENSE.md");
  expect(font.status()).toBe(200);
  expect(await font.text()).toContain("SIL Open Font License");
});
