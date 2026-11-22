import { screen, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { CreatedInvite, Invite, JoinRequest } from "@/api/admin";
import { apiError, authConfig, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

const waiting: JoinRequest = {
  id: "0193a5b2-0000-7000-8000-0000000000e1",
  email: "linus@example.com",
  note: "From the hackathon team.\nI run the build.",
  status: "pending",
  created_at: "2026-11-20T09:00:00Z",
  confirmed_at: "2026-11-20T09:05:00Z",
};

const denied: JoinRequest = {
  ...waiting,
  id: "0193a5b2-0000-7000-8000-0000000000e2",
  email: "mallory@example.com",
  note: "",
  status: "denied",
  decided_at: "2026-11-20T12:00:00Z",
};

const invite: Invite = {
  id: "0193a5b2-0000-7000-8000-0000000000c5",
  email: "linus@example.com",
  created_at: "2026-11-21T09:00:00Z",
  expires_at: "2026-11-28T09:00:00Z",
  max_uses: 1,
  uses: 0,
  status: "open",
  code_kept: true,
  signups: [],
};

function requests(list: JoinRequest[]) {
  const decided: string[] = [];
  let current = list;
  const decide = (status: "approved" | "denied") => (id: string) => {
    decided.push(`${status} ${id}`);
    current = current.map((request) => (request.id === id ? { ...request, status } : request));
  };
  server.use(
    signedInAs(grace),
    authConfig({ requests: true }),
    http.get("/api/v1/admin/join-requests", () => HttpResponse.json({ requests: current })),
    http.get("/api/v1/admin/invites", () => HttpResponse.json({ invites: [], by_email: true })),
    http.post("/api/v1/admin/join-requests/:id/approve", ({ params }) => {
      decide("approved")(String(params["id"]));
      const created: CreatedInvite = { invite, code: "inv_approved", sent: true };
      return HttpResponse.json(created);
    }),
    http.post("/api/v1/admin/join-requests/:id/deny", ({ params }) => {
      decide("denied")(String(params["id"]));
      return new HttpResponse(null, { status: 204 });
    }),
  );
  return decided;
}

describe("requests to join", () => {
  it("lists the requests with their notes, waiting first", async () => {
    requests([waiting, denied]);
    renderApp("/admin/requests");

    expect(await screen.findByText("linus@example.com")).toBeInTheDocument();
    expect(screen.getByText(/I run the build/)).toBeInTheDocument();
    expect(screen.getByText("No note.")).toBeInTheDocument();
    expect(screen.getByText("Denied")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve: mallory@example.com" })).toBeNull();
  });

  it("approves a request, mailing an invite bound to the address", async () => {
    const decided = requests([waiting]);
    const { user } = renderApp("/admin/requests");

    await user.click(await screen.findByRole("button", { name: "Approve: linus@example.com" }));

    expect(await screen.findByText("Invite mailed to linus@example.com")).toBeInTheDocument();
    expect(screen.getByText(/Only linus@example.com can use it/)).toBeInTheDocument();
    expect(decided).toEqual([`approved ${waiting.id}`]);
    expect(await screen.findByText("Approved")).toBeInTheDocument();
  });

  it("denies a request once confirmed", async () => {
    const decided = requests([waiting]);
    const { user } = renderApp("/admin/requests");

    await user.click(await screen.findByRole("button", { name: "Deny: linus@example.com" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Deny linus@example.com?" });
    await user.click(within(dialog).getByRole("button", { name: "Deny" }));

    expect(await screen.findByText("Denied linus@example.com.")).toBeInTheDocument();
    expect(decided).toEqual([`denied ${waiting.id}`]);
  });

  it("says why an approval failed", async () => {
    requests([waiting]);
    server.use(
      http.post("/api/v1/admin/join-requests/:id/approve", () =>
        apiError(409, "email_taken", "an account has this address already"),
      ),
    );
    const { user } = renderApp("/admin/requests");

    await user.click(await screen.findByRole("button", { name: "Approve: linus@example.com" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "An account has this address already.",
    );
  });

  it("says when requests are off", async () => {
    requests([]);
    server.use(authConfig({ requests: false }));
    renderApp("/admin/requests");

    expect(await screen.findByText(/Requests to join are off/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open the settings" })).toHaveAttribute(
      "href",
      "/admin/settings",
    );
    expect(await screen.findByText("No requests to join.")).toBeInTheDocument();
  });
});
