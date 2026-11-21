import { screen, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { META_TIMEOUT_MS } from "@/api/meta";
import { createQueryClient } from "@/api/query-client";
import { renderApp } from "@/test/render-app";
import { server, testMeta } from "@/test/server";

/** The desktop sidebar; the drawer's copy only mounts while it is open. */
async function sidebar() {
  return within(await screen.findByRole("complementary"));
}

/**
 * Fakes setTimeout so a test can jump past the request deadline or the retry
 * delay. Fake time still follows the real clock, which Testing Library's own
 * short timeouts rely on.
 */
function useFakeTimeouts() {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true });
}

describe("sidebar footer", () => {
  it("shows the instance name and server version from /api/v1/meta", async () => {
    renderApp();
    const aside = await sidebar();
    expect(await aside.findByText(testMeta.name)).toBeInTheDocument();
    expect(aside.getByText(`v${testMeta.version}`)).toBeInTheDocument();
  });

  it("reports a server error and recovers on retry", async () => {
    server.use(
      http.get(
        "/api/v1/meta",
        () =>
          HttpResponse.json(
            { error: { code: "internal", message: "Database unavailable" } },
            { status: 503 },
          ),
        { once: true },
      ),
    );
    const { user } = renderApp();
    const aside = await sidebar();

    expect(await aside.findByText("Server error 503")).toBeInTheDocument();
    await user.click(aside.getByRole("button", { name: "Retry" }));
    expect(await aside.findByText(testMeta.name)).toBeInTheDocument();
  });

  it("reports an unreachable server", async () => {
    server.use(http.get("/api/v1/meta", () => HttpResponse.error()));
    renderApp();
    expect(await (await sidebar()).findByText("Server unreachable")).toBeInTheDocument();
  });

  it("reports an unreachable server when only the proxy in front answers", async () => {
    server.use(http.get("/api/v1/meta", () => new HttpResponse("Bad Gateway", { status: 502 })));
    renderApp();
    expect(await (await sidebar()).findByText("Server unreachable")).toBeInTheDocument();
  });

  it("reports an unreachable server when the request gets no answer in time", async () => {
    useFakeTimeouts();
    server.use(http.get("/api/v1/meta", () => new Promise<never>(() => {})));
    renderApp();
    const aside = await sidebar();
    expect(aside.getByText("Connecting")).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(META_TIMEOUT_MS);
    expect(await aside.findByText("Server unreachable")).toBeInTheDocument();
  });

  it("retries once, a second later, before reporting the server unreachable", async () => {
    useFakeTimeouts();
    let requests = 0;
    server.use(
      http.get("/api/v1/meta", () => {
        requests += 1;
        return HttpResponse.error();
      }),
    );
    renderApp("/", createQueryClient());
    const aside = await sidebar();
    await vi.waitFor(() => expect(requests).toBe(1));
    expect(aside.getByText("Connecting")).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(1000);
    expect(await aside.findByText("Server unreachable")).toBeInTheDocument();
    expect(requests).toBe(2);
  });

  it("keeps the navigation usable while the API is down", async () => {
    server.use(http.get("/api/v1/meta", () => HttpResponse.error()));
    const { user } = renderApp();
    const aside = await sidebar();
    await aside.findByText("Server unreachable");

    await user.click(aside.getByRole("link", { name: "Styleguide" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Styleguide" }),
    ).toBeInTheDocument();
  });
});
