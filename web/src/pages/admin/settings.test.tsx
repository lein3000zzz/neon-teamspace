import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { Setting } from "@/api/admin";
import { apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";

const registration: Setting = {
  key: "auth.registration",
  kind: "enum",
  options: ["open", "invite", "closed"],
  description: "Who may create an account.",
  value: "invite",
  default: "invite",
  source: "default",
  pinned: false,
  browser_only: false,
  env_var: "NEON_SETTINGS__AUTH__REGISTRATION",
};

const requestsSetting: Setting = {
  key: "auth.requests",
  kind: "bool",
  description: "People without an invite can ask for one.",
  value: false,
  default: false,
  source: "default",
  pinned: false,
  browser_only: false,
  env_var: "NEON_SETTINGS__AUTH__REQUESTS",
};

const minLength: Setting = {
  key: "auth.min_password_length",
  kind: "integer",
  description: "The fewest characters a new password has.",
  value: 14,
  default: 12,
  source: "database",
  pinned: false,
  browser_only: false,
  env_var: "NEON_SETTINGS__AUTH__MIN_PASSWORD_LENGTH",
};

const idle: Setting = {
  key: "auth.session_idle_ttl",
  kind: "duration",
  description: "A browser session unused this long signs out.",
  value: "7d",
  default: "7d",
  source: "default",
  pinned: false,
  browser_only: false,
  env_var: "NEON_SETTINGS__AUTH__SESSION_IDLE_TTL",
};

const name: Setting = {
  key: "instance.name",
  kind: "string",
  description: "The instance's name.",
  value: "Neon E2E",
  default: "Neon Workspace",
  source: "env",
  pinned: true,
  browser_only: false,
  env_var: "NEON_SETTINGS__INSTANCE__NAME",
};

const smtpPassword: Setting = {
  key: "mail.password",
  kind: "secret",
  description: "The password of that account.",
  value: "[redacted]",
  default: "",
  source: "database",
  pinned: false,
  browser_only: true,
  env_var: "NEON_SETTINGS__MAIL__PASSWORD",
};

const ALL = [minLength, registration, requestsSetting, idle, name, smtpPassword];

function settings(list: Setting[] = ALL) {
  const patches: Record<string, unknown>[] = [];
  server.use(
    signedInAs(grace),
    http.get("/api/v1/admin/settings", () => HttpResponse.json({ settings: list })),
    http.patch("/api/v1/admin/settings", async ({ request }) => {
      const { changes } = (await request.json()) as { changes: Record<string, unknown> };
      patches.push(changes);
      return HttpResponse.json({
        settings: list.map((setting) =>
          setting.key in changes
            ? {
                ...setting,
                value: changes[setting.key] ?? setting.default,
                source: changes[setting.key] === null ? "default" : "database",
              }
            : setting,
        ),
      });
    }),
  );
  return patches;
}

async function renderSettings() {
  const app = renderApp("/admin/settings");
  await screen.findByRole("region", { name: "Accounts" });
  return app;
}

describe("settings", () => {
  it("groups the settings by their key's prefix, with where each value comes from", async () => {
    settings();
    await renderSettings();

    const accounts = within(screen.getByRole("region", { name: "Accounts" }));
    expect(accounts.getByLabelText("auth.registration")).toHaveValue("invite");
    expect(accounts.getByLabelText("auth.min_password_length")).toHaveValue("14");
    expect(accounts.getAllByText("Default")).toHaveLength(3);
    expect(accounts.getByText("Saved here")).toBeInTheDocument();

    const instance = within(screen.getByRole("region", { name: "Instance" }));
    expect(instance.getByText("Environment")).toBeInTheDocument();
    expect(instance.getByText("Pinned")).toBeInTheDocument();
    expect(instance.getByLabelText("instance.name")).toBeDisabled();
    expect(instance.getByText("Change it with NEON_SETTINGS__INSTANCE__NAME.")).toBeInTheDocument();
  });

  it("never shows a secret's value", async () => {
    settings();
    await renderSettings();

    const secret = screen.getByLabelText("mail.password");
    expect(secret).toHaveAttribute("type", "password");
    expect(secret).toHaveValue("");
    expect(secret).toHaveAttribute("placeholder", "Set. Type a new value to replace it.");
    expect(screen.queryByDisplayValue("[redacted]")).toBeNull();
  });

  it("saves the changes together", async () => {
    const patches = settings();
    const { user } = await renderSettings();

    await user.selectOptions(screen.getByLabelText("auth.registration"), "open");
    await user.click(
      within(screen.getByRole("radiogroup", { name: "auth.requests" })).getByRole("radio", {
        name: "On",
      }),
    );
    const ttl = screen.getByLabelText("auth.session_idle_ttl");
    await user.clear(ttl);
    await user.type(ttl, "12h");
    await user.type(screen.getByLabelText("mail.password"), "s3cret");
    expect(screen.getByText("4 unsaved changes")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("Saved.")).toBeInTheDocument();
    expect(patches).toEqual([
      {
        "auth.registration": "open",
        "auth.requests": true,
        "auth.session_idle_ttl": "12h",
        "mail.password": "s3cret",
      },
    ]);
  });

  it("resets a saved setting to its default", async () => {
    const patches = settings();
    const { user } = await renderSettings();

    await user.click(
      screen.getByRole("button", { name: "Reset auth.min_password_length to its default" }),
    );
    expect(screen.getByText(/Goes back to the default when saved/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(patches).toEqual([{ "auth.min_password_length": null }]));
  });

  it("sends whole numbers as numbers, and refuses anything else before sending", async () => {
    const patches = settings();
    const { user } = await renderSettings();

    const field = screen.getByLabelText("auth.min_password_length");
    await user.clear(field);
    await user.type(field, "twelve");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    expect(field).toHaveAttribute("aria-invalid", "true");
    expect(patches).toEqual([]);

    await user.clear(field);
    await user.type(field, "16");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(patches).toEqual([{ "auth.min_password_length": 16 }]));
  });

  it("shows the server's problem beside each setting it refused, and saves nothing", async () => {
    settings();
    server.use(
      http.patch("/api/v1/admin/settings", () =>
        apiError(400, "invalid_settings", "nothing was saved: some settings are invalid", {
          problems: [
            {
              key: "auth.requests",
              message: "needs mail set up first: mail.enabled on, with mail.host and mail.from",
            },
            { key: "auth.session_idle_ttl", message: "must be between 5m and 365d" },
          ],
        }),
      ),
    );
    const { user } = await renderSettings();

    await user.click(
      within(screen.getByRole("radiogroup", { name: "auth.requests" })).getByRole("radio", {
        name: "On",
      }),
    );
    const ttl = screen.getByLabelText("auth.session_idle_ttl");
    await user.clear(ttl);
    await user.type(ttl, "1s");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Nothing was saved: fix the settings marked below.",
    );
    expect(screen.getByText("must be between 5m and 365d")).toBeInTheDocument();
    expect(ttl).toHaveAttribute("aria-invalid", "true");
    expect(
      screen.getByText("needs mail set up first: mail.enabled on, with mail.host and mail.from"),
    ).toBeInTheDocument();
    expect(screen.getByText("2 unsaved changes")).toBeInTheDocument();
  });

  it("discards the changes", async () => {
    const patches = settings();
    const { user } = await renderSettings();

    await user.selectOptions(screen.getByLabelText("auth.registration"), "closed");
    await user.click(screen.getByRole("button", { name: "Discard" }));

    expect(screen.getByLabelText("auth.registration")).toHaveValue("invite");
    expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
    expect(patches).toEqual([]);
  });

  it("drops a change typed back to the saved value", async () => {
    settings();
    const { user } = await renderSettings();

    await user.selectOptions(screen.getByLabelText("auth.registration"), "open");
    expect(screen.getByText("1 unsaved change")).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("auth.registration"), "invite");
    expect(screen.queryByText(/unsaved change/)).toBeNull();
  });
});
