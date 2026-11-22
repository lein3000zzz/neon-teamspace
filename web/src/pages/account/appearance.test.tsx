import { screen, waitFor, within } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import type { Schemas } from "@/api/client";
import { mockColorScheme } from "@/test/color-scheme";
import { apiError, defaultPreferences } from "@/test/fixtures";
import { renderApp } from "@/test/render-app";
import { server } from "@/test/server";
import { blockStorage, fillStorage } from "@/test/storage";
import { DEFAULT_PREFERENCES, STORAGE_KEY } from "@/theme/preferences";

const html = () => document.documentElement;
const stored = () => JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "null") as unknown;

/** The account's look on the server, which records every save. */
function accountLook(initial: Schemas["Preferences"] = defaultPreferences) {
  const saves: Schemas["Preferences"][] = [];
  let current = initial;
  server.use(
    http.get("/api/v1/account/preferences", () => HttpResponse.json(current)),
    http.put("/api/v1/account/preferences", async ({ request }) => {
      current = (await request.json()) as Schemas["Preferences"];
      saves.push(current);
      return HttpResponse.json(current);
    }),
  );
  return saves;
}

async function renderAppearance() {
  const app = renderApp("/account/appearance");
  await screen.findByRole("radiogroup", { name: "Palette" });
  return app;
}

function group(name: string) {
  return screen.getByRole("radiogroup", { name });
}

/**
 * Waits for the last change to reach the account. A save still waiting when a
 * test ends would go out as the page unmounts, into the next test's server.
 */
async function settled() {
  expect(await screen.findByText("Saved to your account.")).toBeInTheDocument();
}

describe("appearance settings", () => {
  it("starts from the defaults and follows the system mode", async () => {
    mockColorScheme("dark");
    accountLook();
    await renderAppearance();

    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "dark", uiFont: "default" });
    expect(html().style.getPropertyValue("--accent-h")).toBe(String(DEFAULT_PREFERENCES.accentHue));
    expect(within(group("Mode")).getByRole("radio", { name: "System" })).toBeChecked();
    expect(screen.getByText(/currently Dark/)).toBeInTheDocument();
  });

  it("takes the account's look on sign-in and caches it for the next first paint", async () => {
    mockColorScheme("light");
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ palette: "soft" }));
    accountLook({ palette: "warm", mode: "dark", accent_hue: 25, ui_font: "mono" });
    await renderAppearance();

    await waitFor(() => expect(html().dataset["palette"]).toBe("warm"));
    expect(html().dataset).toMatchObject({ mode: "dark", uiFont: "mono" });
    expect(html().style.getPropertyValue("--accent-h")).toBe("25");
    expect(stored()).toEqual({ palette: "warm", mode: "dark", accentHue: 25, uiFont: "mono" });
  });

  it("caches the account's defaults as no choice at all", async () => {
    mockColorScheme("light");
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ palette: "soft" }));
    accountLook();
    await renderAppearance();

    await waitFor(() => expect(html().dataset["palette"]).toBe("nothing"));
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("switches the palette and saves it to the account", async () => {
    mockColorScheme("light");
    const saves = accountLook();
    const { user } = await renderAppearance();

    await user.click(within(group("Palette")).getByRole("radio", { name: "Warm" }));
    expect(html().dataset["palette"]).toBe("warm");
    expect(stored()).toMatchObject({ palette: "warm" });
    await waitFor(() => expect(saves).toEqual([{ ...defaultPreferences, palette: "warm" }]));
    expect(await screen.findByText("Saved to your account.")).toBeInTheDocument();
  });

  it("names the device's scheme in the hint while another mode is chosen", async () => {
    mockColorScheme("dark");
    accountLook();
    const { user } = await renderAppearance();

    await user.click(within(group("Mode")).getByRole("radio", { name: "Light" }));
    expect(html().dataset["mode"]).toBe("light");
    expect(screen.getByText(/currently Dark/)).toBeInTheDocument();
    await settled();
  });

  it("switches the mode and keeps a choice when its button is pressed again", async () => {
    mockColorScheme("light");
    accountLook();
    const { user } = await renderAppearance();
    const dark = within(group("Mode")).getByRole("radio", { name: "Dark" });

    await user.click(dark);
    expect(html().dataset["mode"]).toBe("dark");
    await user.click(dark);
    expect(html().dataset["mode"]).toBe("dark");
    expect(dark).toBeChecked();
    expect(stored()).toMatchObject({ mode: "dark" });
    await settled();
  });

  it("sets the accent from a preset and from the slider, saving once the moves stop", async () => {
    mockColorScheme("light");
    const saves = accountLook();
    const { user } = await renderAppearance();

    await user.click(within(group("Accent")).getByRole("radio", { name: "Violet" }));
    expect(html().style.getPropertyValue("--accent-h")).toBe("295");

    const slider = screen.getByRole("slider", { name: "Accent hue" });
    expect(slider).toHaveAttribute("aria-valuetext", "295 degrees");
    slider.focus();
    await user.keyboard("{ArrowRight}{ArrowRight}");
    expect(html().style.getPropertyValue("--accent-h")).toBe("297");
    expect(within(group("Accent")).queryByRole("radio", { checked: true })).toBeNull();
    expect(stored()).toMatchObject({ accentHue: 297 });

    await waitFor(() => expect(saves.at(-1)).toMatchObject({ accent_hue: 297 }));
    expect(saves.length).toBeLessThan(3);
    await settled();
  });

  it("switches the interface font", async () => {
    mockColorScheme("light");
    accountLook();
    const { user } = await renderAppearance();

    await user.click(within(group("Interface font")).getByRole("radio", { name: "Mono" }));
    expect(html().dataset["uiFont"]).toBe("mono");
    await settled();
  });

  it("resets to the defaults, forgets the stored choice and saves the defaults", async () => {
    mockColorScheme("light");
    const saves = accountLook({ palette: "soft", mode: "dark", accent_hue: 25, ui_font: "mono" });
    const { user } = await renderAppearance();
    await waitFor(() => expect(html().dataset["palette"]).toBe("soft"));

    await user.click(screen.getByRole("button", { name: "Reset to defaults" }));
    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "light", uiFont: "default" });
    expect(html().style.getPropertyValue("--accent-h")).toBe(String(DEFAULT_PREFERENCES.accentHue));
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
    await waitFor(() => expect(saves).toEqual([defaultPreferences]));
    await settled();
  });

  it("keeps the choice on this device when saving it to the account fails", async () => {
    mockColorScheme("light");
    accountLook();
    server.use(
      http.put("/api/v1/account/preferences", () =>
        apiError(400, "invalid_input", "a hue is 0 to 359"),
      ),
    );
    const { user } = await renderAppearance();

    await user.click(within(group("Palette")).getByRole("radio", { name: "Soft" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("saving it to your account failed");
    expect(screen.getByRole("alert")).toHaveTextContent("A hue is 0 to 359.");
    expect(html().dataset["palette"]).toBe("soft");
  });

  it.each([
    ["refuses storage", blockStorage],
    ["has no room left in storage", fillStorage],
  ])("renders and applies changes when the browser %s", async (_, breakStorage) => {
    mockColorScheme("light");
    accountLook();
    breakStorage();
    const { user } = await renderAppearance();
    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "light", uiFont: "default" });

    await user.click(within(group("Palette")).getByRole("radio", { name: "Warm" }));
    expect(html().dataset["palette"]).toBe("warm");
    await user.click(screen.getByRole("button", { name: "Reset to defaults" }));
    expect(html().dataset["palette"]).toBe("nothing");
    await settled();
  });
});
