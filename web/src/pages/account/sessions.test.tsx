import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { SessionInfo } from "@/api/account";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

const here: SessionInfo = {
  id: "0193a5b2-0000-7000-8000-000000000001",
  current: true,
  user_agent:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
  ip: "203.0.113.7",
  created_at: "2026-11-20T09:00:00Z",
  last_seen_at: "2026-11-21T09:00:00Z",
  expires_at: "2026-11-28T09:00:00Z",
  absolute_expires_at: "2026-12-20T09:00:00Z",
};

const phone: SessionInfo = {
  ...here,
  id: "0193a5b2-0000-7000-8000-000000000002",
  current: false,
  user_agent:
    "Mozilla/5.0 (Linux; Android 15) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Mobile Safari/537.36",
  ip: "198.51.100.4",
};

/** The account's sessions on the server, which revoking removes. */
function sessions(initial: SessionInfo[]) {
  let list = initial;
  const revoked: string[] = [];
  server.use(
    http.get("/api/v1/account/sessions", () => HttpResponse.json({ sessions: list })),
    http.delete("/api/v1/account/sessions/:id", ({ params }) => {
      revoked.push(String(params["id"]));
      list = list.filter((session) => session.id !== params["id"]);
      return new HttpResponse(null, { status: 204 });
    }),
    http.delete("/api/v1/account/sessions", () => {
      const ended = list.filter((session) => !session.current).length;
      list = list.filter((session) => session.current);
      return HttpResponse.json({ revoked: ended });
    }),
  );
  return revoked;
}

/** The cards of the sessions list. */
async function cards() {
  const region = within(await screen.findByRole("region", { name: "Sessions" }));
  return region.findAllByRole("listitem");
}

describe("sessions", () => {
  it("lists each browser, marking this one", async () => {
    sessions([here, phone]);
    renderApp("/account/sessions");

    const [first, second, ...rest] = await cards();
    expect(rest).toEqual([]);
    expect(within(first as HTMLElement).getByText("Safari on macOS")).toBeInTheDocument();
    expect(within(first as HTMLElement).getByText("This browser")).toBeInTheDocument();
    expect(within(second as HTMLElement).getByText("Chrome on Android")).toBeInTheDocument();
    expect(within(second as HTMLElement).getByText("198.51.100.4")).toBeInTheDocument();
  });

  it("signs another browser out", async () => {
    const revoked = sessions([here, phone]);
    const { user } = renderApp("/account/sessions");

    await user.click(await screen.findByRole("button", { name: /^Sign out Chrome on Android/ }));

    await waitFor(async () => expect(await cards()).toHaveLength(1));
    expect(revoked).toEqual([phone.id]);
  });

  it("signs out everywhere else and says how many ended", async () => {
    sessions([here, phone, { ...phone, id: "0193a5b2-0000-7000-8000-000000000003" }]);
    const { user } = renderApp("/account/sessions");

    await user.click(await screen.findByRole("button", { name: "Sign out everywhere else" }));

    expect(await screen.findByText("Signed out 2 other sessions.")).toBeInTheDocument();
    await waitFor(async () => expect(await cards()).toHaveLength(1));
    expect(screen.getByRole("button", { name: "Sign out everywhere else" })).toBeDisabled();
  });

  it("signs this browser out like the sidebar does", async () => {
    sessions([here]);
    server.use(http.post("/api/v1/auth/logout", () => new HttpResponse(null, { status: 204 })));
    const { user, router } = renderApp("/account/sessions");

    await user.click(await screen.findByRole("button", { name: /^Sign out This browser/ }));

    expect(await screen.findByRole("heading", { level: 1, name: "Sign in" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/login");
  });

  it("reports a failed load and retries", async () => {
    sessions([here]);
    server.use(
      http.get("/api/v1/account/sessions", () => new HttpResponse(null, { status: 500 }), {
        once: true,
      }),
    );
    const { user } = renderApp("/account/sessions");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The server answered with status 500.",
    );
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("This browser")).toBeInTheDocument();
  });
});
