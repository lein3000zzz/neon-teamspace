import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Release, Updates, UpdaterStatus } from "@/api/updates";
import { formatDateTime } from "@/lib/format";
import { reloadPage } from "@/lib/reload";
import { apiError, grace, signedInAs } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server, testMeta, testUpdates } from "@/test/server";

vi.mock("@/lib/reload", () => ({ reloadPage: vi.fn<() => void>() }));

const CHECKED_AT = "2026-11-28T09:00:00Z";
const PUBLISHED_AT = "2026-11-27T18:30:00Z";
const NOTES = "https://github.com/lein3000zzz/neon-teamspace/releases/tag/v0.2.0";

/** The release the tests' GitHub lists. */
const RELEASE: Release = {
  version: "0.2.0",
  prerelease: false,
  url: NOTES,
  published_at: PUBLISHED_AT,
};

/** A server on 0.1.0 whose last check found 0.2.0 out, with no updater set up. */
const NEWER: Updates = {
  ...testUpdates,
  latest: RELEASE,
  newer: true,
  checked_at: CHECKED_AT,
};

function updaterStatus(patch: Partial<UpdaterStatus> = {}): UpdaterStatus {
  return { state: "idle", updater_version: "0.2.0", ...patch };
}

/** As NEWER, with an updater set up that answered `status`. */
function withUpdater(status: UpdaterStatus = updaterStatus()): Updates {
  return { ...NEWER, updater: { set_up: true, status, error: null } };
}

/** The settings page, signed in as an admin, with `updates` as the server's answer. */
async function renderPanel(updates: Updates) {
  server.use(
    signedInAs(grace),
    http.get("/api/v1/admin/settings", () =>
      HttpResponse.json({ settings: [], mail: { configured: false } }),
    ),
    http.get("/api/v1/admin/updates", () => HttpResponse.json(updates)),
  );
  const app = renderApp("/admin/settings");
  const region = await screen.findByRole("region", { name: "Version and updates" });
  await within(region).findByText("Running");
  return { ...app, panel: within(region) };
}

/** Follows the fake clock past the next look at an update's progress. */
async function nextPoll() {
  await vi.advanceTimersByTimeAsync(2_000);
}

function useFakeClock() {
  vi.useFakeTimers({
    toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"],
    shouldAdvanceTime: true,
  });
}

beforeEach(() => {
  vi.mocked(reloadPage).mockClear();
});

describe("the version and updates", () => {
  it("shows the release running, the newest with its notes, and when the server checked", async () => {
    const latest = { ...RELEASE, version: "0.1.0", url: NOTES.replace("0.2.0", "0.1.0") };
    const { panel } = await renderPanel({ ...NEWER, latest, newer: false });

    expect(panel.getByText("0.1.0", { selector: "span" })).toBeInTheDocument();
    const notes = panel.getByRole("link", { name: "Release notes of 0.1.0, on GitHub" });
    expect(notes).toHaveAttribute("href", latest.url);
    expect(notes).toHaveAttribute("target", "_blank");
    expect(panel.getByText(`published ${formatDateTime(PUBLISHED_AT)}`)).toBeInTheDocument();
    expect(panel.getByText(formatDateTime(CHECKED_AT))).toBeInTheDocument();
    expect(panel.getByText("This is the newest release.")).toBeInTheDocument();
    expect(panel.queryByRole("button", { name: /Update to/ })).toBeNull();
  });

  it("says the server has not checked yet", async () => {
    const { panel } = await renderPanel(testUpdates);

    expect(panel.getByText("None found yet")).toBeInTheDocument();
    expect(panel.getByText("Not yet")).toBeInTheDocument();
    expect(panel.getByRole("button", { name: "Check now" })).toBeEnabled();
  });

  it("says the release check is off, and offers no check", async () => {
    const { panel } = await renderPanel({ ...testUpdates, check: false });

    expect(panel.getByText(/The release check is off/)).toHaveTextContent(
      "Turn on updates.check under Updates below",
    );
    expect(panel.queryByText("Newest release")).toBeNull();
    expect(panel.queryByRole("button", { name: "Check now" })).toBeNull();
  });

  it("shows why the last check failed, beside what the check before found", async () => {
    const error =
      "GitHub answered 403: it limits how often one address asks, so the next check tries again";
    const { panel } = await renderPanel({ ...NEWER, error });

    expect(panel.getByText("Last check failed")).toBeInTheDocument();
    expect(panel.getByText(error)).toBeInTheDocument();
    expect(panel.getByRole("link", { name: "Release notes of 0.2.0, on GitHub" })).toBeVisible();
  });

  it("never offers a development build an update", async () => {
    const { panel } = await renderPanel({ ...withUpdater(), development: true, newer: false });

    expect(panel.getByText("0.1.0, a development build")).toBeInTheDocument();
    expect(panel.getByText(/A development build is never behind a release/)).toBeInTheDocument();
    expect(panel.queryByRole("button", { name: /Update to/ })).toBeNull();
  });

  it("checks now when asked, and shows what the check found", async () => {
    let checks = 0;
    const { user, panel } = await renderPanel(testUpdates);
    server.use(
      http.post("/api/v1/admin/updates/check", () => {
        checks += 1;
        return HttpResponse.json(NEWER);
      }),
    );

    await user.click(panel.getByRole("button", { name: "Check now" }));

    expect(await panel.findByText("Neon Teamspace 0.2.0 is out")).toBeInTheDocument();
    expect(checks).toBe(1);
  });

  it("says why a check was refused", async () => {
    const { user, panel } = await renderPanel(testUpdates);
    server.use(
      http.post("/api/v1/admin/updates/check", () =>
        apiError(500, "internal", "something went wrong"),
      ),
    );

    await user.click(panel.getByRole("button", { name: "Check now" }));

    expect(await panel.findByRole("alert")).toHaveTextContent("Something went wrong.");
  });

  it("says why the status could not be read, with a retry", async () => {
    server.use(
      signedInAs(grace),
      http.get("/api/v1/admin/settings", () =>
        HttpResponse.json({ settings: [], mail: { configured: false } }),
      ),
      http.get("/api/v1/admin/updates", () => apiError(500, "internal", "the database is down")),
    );
    renderApp("/admin/settings");
    const panel = within(await screen.findByRole("region", { name: "Version and updates" }));

    expect(await panel.findByRole("alert")).toHaveTextContent("The database is down.");
    expect(panel.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("without an updater, gives the commands that update by hand, and the docs", async () => {
    const latest = { ...RELEASE, version: "0.3.0-rc.1", prerelease: true };
    const { user, panel } = await renderPanel({ ...NEWER, latest });

    expect(panel.getByText("Neon Teamspace 0.3.0-rc.1 is out")).toBeInTheDocument();
    expect(panel.getByText("Pre-release")).toBeInTheDocument();
    expect(panel.getByRole("link", { name: "Read its release notes" })).toHaveAttribute(
      "href",
      NOTES,
    );
    expect(panel.getByText(/set NEON_VERSION to 0.3.0-rc.1/)).toBeInTheDocument();
    expect(panel.getByLabelText("deployments/.env")).toHaveTextContent("NEON_VERSION=0.3.0-rc.1");
    expect(panel.getByLabelText("Commands").textContent).toBe(
      "docker compose -f deployments/docker-compose.yml pull\n" +
        "docker compose -f deployments/docker-compose.yml up -d --wait",
    );
    expect(panel.getByRole("link", { name: "Upgrading" })).toHaveAttribute(
      "href",
      "/docs/self-hosting/upgrading/",
    );
    expect(panel.getByRole("link", { name: "Set up the updater" })).toHaveAttribute(
      "href",
      "/docs/self-hosting/upgrading/#one-click-updates",
    );
    expect(panel.queryByRole("button", { name: /Update to/ })).toBeNull();

    await user.click(panel.getByRole("button", { name: "Copy Commands" }));
    expect(await navigator.clipboard.readText()).toContain("up -d --wait");
    expect(panel.getByRole("button", { name: "Copied Commands" })).toBeInTheDocument();
  });

  it("asks before updating, and asks nothing when cancelled", async () => {
    let applied = 0;
    const { user, panel } = await renderPanel(withUpdater());
    server.use(
      http.post("/api/v1/admin/updates/apply", () => {
        applied += 1;
        return HttpResponse.json(withUpdater(updaterStatus({ state: "updating" })), {
          status: 202,
        });
      }),
    );

    await user.click(panel.getByRole("button", { name: "Update to 0.2.0" }));
    const dialog = within(await screen.findByRole("alertdialog"));
    expect(dialog.getByRole("heading", { name: "Update to 0.2.0?" })).toBeInTheDocument();
    expect(dialog.getByText(/The server restarts on the new release/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(applied).toBe(0);
    expect(panel.getByRole("button", { name: "Update to 0.2.0" })).toBeEnabled();
    expect(panel.queryByText("Updating to 0.2.0")).toBeNull();
  });

  it("follows an update through the restart, then reloads on the new release", async () => {
    useFakeClock();
    const { user, panel } = await renderPanel(withUpdater());
    const started = updaterStatus({
      state: "updating",
      version: "0.2.0",
      log: "Pulling server, web at 0.2.0\n",
    });
    let stage: "old" | "down" | "new" = "old";
    let applied: unknown;
    server.use(
      http.post("/api/v1/admin/updates/apply", async ({ request }) => {
        applied = await request.json();
        return HttpResponse.json(withUpdater(started), { status: 202 });
      }),
      http.get("/api/v1/meta", () =>
        stage === "down"
          ? HttpResponse.error()
          : HttpResponse.json({ ...testMeta, version: stage === "new" ? "0.2.0" : "0.1.0" }),
      ),
      http.get("/api/v1/admin/updates", () =>
        stage === "down"
          ? new HttpResponse("Bad Gateway", { status: 502 })
          : HttpResponse.json(
              stage === "new"
                ? {
                    ...withUpdater(updaterStatus({ state: "done", version: "0.2.0" })),
                    current: "0.2.0",
                    newer: false,
                  }
                : withUpdater(started),
            ),
      ),
    );

    await user.click(panel.getByRole("button", { name: "Update to 0.2.0" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Update" }),
    );

    expect(await panel.findByText("Updating to 0.2.0")).toBeInTheDocument();
    expect(applied).toEqual({ version: "0.2.0" });
    await nextPoll();
    expect(await panel.findByText("The updater is at work.")).toBeInTheDocument();
    expect(panel.getByLabelText("What the updater did")).toHaveTextContent(
      "Pulling server, web at 0.2.0",
    );

    stage = "down";
    await nextPoll();
    expect(await panel.findByText(/The server is restarting/)).toBeInTheDocument();
    // What the updater did stays in sight while nothing answers.
    expect(panel.getByLabelText("What the updater did")).toHaveTextContent("Pulling server");

    stage = "new";
    await nextPoll();
    expect(await panel.findByText("Updated to 0.2.0. Reloading the page…")).toBeInTheDocument();
    expect(reloadPage).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(reloadPage).toHaveBeenCalledTimes(1);
  });

  it("follows an update under way when the page opens", async () => {
    useFakeClock();
    const { panel } = await renderPanel(
      withUpdater(
        updaterStatus({ state: "updating", version: "0.2.0", log: "Recreating server\n" }),
      ),
    );

    expect(panel.getByText("Updating to 0.2.0")).toBeInTheDocument();
    expect(panel.queryByRole("button", { name: "Update to 0.2.0" })).toBeNull();
    await nextPoll();
    expect(await panel.findByLabelText("What the updater did")).toHaveTextContent(
      "Recreating server",
    );
  });

  it("shows the updater's account of an update that failed, and offers it again", async () => {
    useFakeClock();
    const { user, panel } = await renderPanel(withUpdater());
    const failed = updaterStatus({
      state: "failed",
      version: "0.2.0",
      error: "pulling the images: manifest unknown",
      log: "Pulling server, web at 0.2.0\nmanifest unknown\n",
    });
    let stage: "old" | "failed" = "old";
    server.use(
      http.post("/api/v1/admin/updates/apply", () =>
        HttpResponse.json(withUpdater(updaterStatus({ state: "updating", version: "0.2.0" })), {
          status: 202,
        }),
      ),
      http.get("/api/v1/admin/updates", () =>
        HttpResponse.json(
          withUpdater(
            stage === "failed" ? failed : updaterStatus({ state: "updating", version: "0.2.0" }),
          ),
        ),
      ),
    );

    await user.click(panel.getByRole("button", { name: "Update to 0.2.0" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Update" }),
    );
    expect(await panel.findByText("Updating to 0.2.0")).toBeInTheDocument();
    stage = "failed";
    await nextPoll();

    expect(
      await panel.findByText(
        "The update to 0.2.0 failed: pulling the images: manifest unknown. The server runs what it ran before.",
      ),
    ).toBeInTheDocument();
    expect(panel.getByText("What the updater did")).toBeInTheDocument();
    expect(panel.getByLabelText("What the updater did")).toHaveTextContent("manifest unknown");
    expect(panel.getByRole("button", { name: "Update to 0.2.0" })).toBeEnabled();
    expect(reloadPage).not.toHaveBeenCalled();
  });

  it("says why the server refused an update", async () => {
    const { user, panel } = await renderPanel(withUpdater());
    server.use(
      http.post("/api/v1/admin/updates/apply", () =>
        apiError(409, "update_in_progress", "an update is under way already"),
      ),
    );

    await user.click(panel.getByRole("button", { name: "Update to 0.2.0" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Update" }),
    );

    expect(await panel.findByRole("alert")).toHaveTextContent("An update is under way already.");
    expect(panel.queryByText("Updating to 0.2.0")).toBeNull();
  });

  it("follows an update another admin started when the server refuses this one", async () => {
    useFakeClock();
    const { user, panel } = await renderPanel(withUpdater());
    const theirs = updaterStatus({
      state: "updating",
      version: "0.2.0",
      log: "Pulling server, web at 0.2.0\n",
    });
    let refused = false;
    server.use(
      http.post("/api/v1/admin/updates/apply", () => {
        refused = true;
        return apiError(409, "update_in_progress", "an update is under way already");
      }),
      http.get("/api/v1/admin/updates", () =>
        HttpResponse.json(withUpdater(refused ? theirs : updaterStatus())),
      ),
    );

    await user.click(panel.getByRole("button", { name: "Update to 0.2.0" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Update" }),
    );

    expect(await panel.findByText("Updating to 0.2.0")).toBeInTheDocument();
    expect(panel.queryByRole("button", { name: "Update to 0.2.0" })).toBeNull();
    await nextPoll();
    expect(await panel.findByLabelText("What the updater did")).toHaveTextContent(
      "Pulling server, web at 0.2.0",
    );
    expect(reloadPage).not.toHaveBeenCalled();
  });

  it("says when the updater cannot be asked", async () => {
    const { panel } = await renderPanel({
      ...NEWER,
      updater: { set_up: true, status: null, error: "the updater did not answer within 3s" },
    });

    expect(panel.getByText("The updater did not answer within 3s.")).toBeInTheDocument();
    expect(panel.getByRole("button", { name: "Update to 0.2.0" })).toBeEnabled();
  });

  it("gives up waiting after ten minutes, with what the updater did last", async () => {
    useFakeClock();
    const { panel } = await renderPanel(
      withUpdater(updaterStatus({ state: "updating", version: "0.2.0", log: "Pulling server\n" })),
    );
    await nextPoll();
    expect(await panel.findByLabelText("What the updater did")).toBeInTheDocument();
    server.use(
      http.get("/api/v1/meta", () => HttpResponse.error()),
      http.get("/api/v1/admin/updates", () => HttpResponse.error()),
    );

    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);

    expect(await panel.findByRole("alert")).toHaveTextContent(
      "The server did not come back on 0.2.0 within 10 minutes.",
    );
    expect(panel.getByLabelText("What the updater did")).toHaveTextContent("Pulling server");
    expect(reloadPage).not.toHaveBeenCalled();
  });
});
