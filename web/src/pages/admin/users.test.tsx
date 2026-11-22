import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { User } from "@/api/account";
import type { Schemas } from "@/api/client";
import { ada, apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

function account(me: typeof ada): User {
  const { credential: _credential, scopes: _scopes, ...user } = me;
  return user;
}

const graceUser = account(grace);
const adaUser = account(ada);
const linus: User = {
  ...adaUser,
  id: "0193a5b2-0000-7000-8000-0000000000b3",
  email: "linus@example.com",
  display_name: "Linus",
  email_verified_at: null,
};

/** The accounts on the server, one page at a time, recording every change. */
function accounts(pages: User[][]) {
  const patches: (Schemas["UpdateUser"] & { id: string })[] = [];
  let all = pages.flat();
  server.use(
    signedInAs(grace),
    http.get("/api/v1/admin/users", ({ request }) => {
      const after = new URL(request.url).searchParams.get("after");
      const index = after === null ? 0 : pages.findIndex((page) => page.at(-1)?.id === after) + 1;
      const page = (pages[index] ?? []).map((user) => all.find((u) => u.id === user.id) ?? user);
      const next = index + 1 < pages.length ? page.at(-1)?.id : undefined;
      return HttpResponse.json({ users: page, ...(next ? { next } : {}) });
    }),
    http.patch("/api/v1/admin/users/:id", async ({ params, request }) => {
      const body = (await request.json()) as Schemas["UpdateUser"];
      patches.push({ id: String(params["id"]), ...body });
      all = all.map((user) =>
        user.id === params["id"]
          ? {
              ...user,
              ...(body.role ? { role: body.role } : {}),
              ...(body.disabled === undefined || body.disabled === null
                ? {}
                : { disabled_at: body.disabled ? "2026-11-21T12:00:00Z" : null }),
              ...(body.email_verified ? { email_verified_at: "2026-11-21T12:00:00Z" } : {}),
            }
          : user,
      );
      return HttpResponse.json(all.find((user) => user.id === params["id"]));
    }),
  );
  return patches;
}

function card(name: string) {
  const item = screen
    .getAllByRole("listitem")
    .find((element) => within(element).queryByText(name, { selector: "p" }) !== null);
  if (item === undefined) {
    throw new Error(`no card for ${name}`);
  }
  return within(item);
}

describe("users", () => {
  it("lists the accounts with what sets each apart", async () => {
    accounts([[graceUser, adaUser, linus]]);
    renderApp("/admin");

    await screen.findByText("Linus");
    expect(card("Grace").getByText("You")).toBeInTheDocument();
    expect(card("Grace").getByText("Admin")).toBeInTheDocument();
    expect(card("Linus").getByText("Not verified")).toBeInTheDocument();
    expect(card("Ada").queryByText("Not verified")).toBeNull();
    expect(screen.getByRole("link", { name: "Users" })).toHaveAttribute("aria-current", "page");
  });

  it("loads more accounts a page at a time", async () => {
    accounts([[graceUser, adaUser], [linus]]);
    const { user } = renderApp("/admin");

    await screen.findByText("Ada");
    expect(screen.queryByText("Linus")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Load more" }));
    expect(await screen.findByText("Linus")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
  });

  it("makes a member an admin once confirmed", async () => {
    const patches = accounts([[graceUser, adaUser]]);
    const { user } = renderApp("/admin");

    await user.click(await screen.findByRole("button", { name: "Make admin: ada@example.com" }));
    const dialog = await screen.findByRole("alertdialog", {
      name: "Make ada@example.com an admin?",
    });
    await user.click(within(dialog).getByRole("button", { name: "Make admin" }));

    expect(await screen.findByText("ada@example.com is updated.")).toBeInTheDocument();
    expect(patches).toEqual([{ id: adaUser.id, role: "admin" }]);
    await waitFor(() => expect(card("Ada").getByText("Admin")).toBeInTheDocument());
  });

  it("disables an account once confirmed, and enables it again", async () => {
    const patches = accounts([[graceUser, adaUser]]);
    const { user } = renderApp("/admin");

    await user.click(await screen.findByRole("button", { name: "Disable: ada@example.com" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("signed out everywhere");
    await user.click(within(dialog).getByRole("button", { name: "Disable" }));
    expect(await screen.findByText("Disabled")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Enable: ada@example.com" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Enable" }),
    );
    await waitFor(() => expect(screen.queryByText("Disabled")).toBeNull());
    expect(patches).toEqual([
      { id: adaUser.id, disabled: true },
      { id: adaUser.id, disabled: false },
    ]);
  });

  it("says clearly when the last admin would be demoted", async () => {
    accounts([[graceUser, adaUser]]);
    server.use(
      http.patch("/api/v1/admin/users/:id", () =>
        apiError(409, "last_admin", "the last active admin cannot be demoted or disabled"),
      ),
    );
    const { user } = renderApp("/admin");

    await user.click(await screen.findByRole("button", { name: "Make member: grace@example.com" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("you lose the admin pages at once");
    await user.click(within(dialog).getByRole("button", { name: "Make member" }));

    expect(await card("Grace").findByRole("alert")).toHaveTextContent(
      "Nothing changed: this is the last active admin.",
    );
  });

  it("marks an address verified, or mails a new link", async () => {
    const patches = accounts([[graceUser, linus]]);
    server.use(
      http.post("/api/v1/admin/users/:id/verification", () =>
        apiError(409, "mail_not_set_up", "mail is not set up"),
      ),
    );
    const { user } = renderApp("/admin");

    await user.click(
      await screen.findByRole("button", { name: "Send a verification link: linus@example.com" }),
    );
    expect(await card("Linus").findByRole("alert")).toHaveTextContent("Mail is not set up.");

    await user.click(screen.getByRole("button", { name: "Mark verified: linus@example.com" }));
    await waitFor(() => expect(card("Linus").queryByText("Not verified")).toBeNull());
    expect(patches).toEqual([{ id: linus.id, email_verified: true }]);
  });

  it("adds an account in any mode", async () => {
    accounts([[graceUser]]);
    let body: unknown;
    server.use(
      http.post("/api/v1/admin/users", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json(linus, { status: 201 });
      }),
    );
    const { user } = renderApp("/admin");

    await user.click(await screen.findByRole("button", { name: "Add an account" }));
    const form = within(screen.getByRole("form", { name: "Add an account" }));
    await user.type(form.getByLabelText("Email"), "linus@example.com");
    await user.type(form.getByLabelText("Password"), "a long enough password");
    await user.selectOptions(form.getByLabelText("Role"), "admin");
    await user.click(form.getByRole("button", { name: "Add account" }));

    expect(await screen.findByText("Added linus@example.com.")).toBeInTheDocument();
    expect(body).toEqual({
      email: "linus@example.com",
      password: "a long enough password",
      role: "admin",
    });
  });

  it("puts a taken address on the email field", async () => {
    accounts([[graceUser]]);
    server.use(
      http.post("/api/v1/admin/users", () =>
        apiError(409, "email_taken", "an account has this address already"),
      ),
    );
    const { user } = renderApp("/admin");

    await user.click(await screen.findByRole("button", { name: "Add an account" }));
    const form = within(screen.getByRole("form", { name: "Add an account" }));
    await user.type(form.getByLabelText("Email"), "ada@example.com");
    await user.type(form.getByLabelText("Password"), "a long enough password");
    await user.click(form.getByRole("button", { name: "Add account" }));

    expect(await form.findByText("An account has this address already.")).toBeInTheDocument();
    expect(form.getByLabelText("Email")).toHaveAttribute("aria-invalid", "true");
  });

  it("reports a failed load and retries", async () => {
    accounts([[graceUser]]);
    server.use(http.get("/api/v1/admin/users", () => HttpResponse.error(), { once: true }));
    const { user } = renderApp("/admin");

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not reach the server");
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("You")).toBeInTheDocument();
  });
});
