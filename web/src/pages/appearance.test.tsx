import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { mockColorScheme } from "@/test/color-scheme";
import { renderApp } from "@/test/render-app";
import { blockStorage, fillStorage } from "@/test/storage";
import { DEFAULT_PREFERENCES, STORAGE_KEY } from "@/theme/preferences";

const html = () => document.documentElement;
const stored = () => JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "null") as unknown;

async function renderAppearance() {
  const app = renderApp("/settings/appearance");
  await screen.findByRole("heading", { level: 1, name: "Appearance" });
  return app;
}

function group(name: string) {
  return screen.getByRole("radiogroup", { name });
}

describe("appearance settings", () => {
  it("starts from the defaults and follows the system mode", async () => {
    mockColorScheme("dark");
    await renderAppearance();

    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "dark", uiFont: "default" });
    expect(html().style.getPropertyValue("--accent-h")).toBe(String(DEFAULT_PREFERENCES.accentHue));
    expect(within(group("Mode")).getByRole("radio", { name: "System" })).toBeChecked();
    expect(screen.getByText(/currently Dark/)).toBeInTheDocument();
  });

  it("switches the palette", async () => {
    mockColorScheme("light");
    const { user } = await renderAppearance();

    await user.click(within(group("Palette")).getByRole("radio", { name: "Warm" }));
    expect(html().dataset["palette"]).toBe("warm");
    expect(stored()).toMatchObject({ palette: "warm" });
  });

  it("names the device's scheme in the hint while another mode is chosen", async () => {
    mockColorScheme("dark");
    const { user } = await renderAppearance();

    await user.click(within(group("Mode")).getByRole("radio", { name: "Light" }));
    expect(html().dataset["mode"]).toBe("light");
    expect(screen.getByText(/currently Dark/)).toBeInTheDocument();
  });

  it("switches the mode and keeps a choice when its button is pressed again", async () => {
    mockColorScheme("light");
    const { user } = await renderAppearance();
    const dark = within(group("Mode")).getByRole("radio", { name: "Dark" });

    await user.click(dark);
    expect(html().dataset["mode"]).toBe("dark");
    await user.click(dark);
    expect(html().dataset["mode"]).toBe("dark");
    expect(dark).toBeChecked();
    expect(stored()).toMatchObject({ mode: "dark" });
  });

  it("sets the accent from a preset and from the slider", async () => {
    mockColorScheme("light");
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
  });

  it("switches the interface font", async () => {
    mockColorScheme("light");
    const { user } = await renderAppearance();

    await user.click(within(group("Interface font")).getByRole("radio", { name: "Mono" }));
    expect(html().dataset["uiFont"]).toBe("mono");
  });

  it("resets to the defaults and forgets the stored choice", async () => {
    mockColorScheme("light");
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ palette: "soft", mode: "dark", accentHue: 25, uiFont: "mono" }),
    );
    const { user } = await renderAppearance();
    expect(html().dataset["palette"]).toBe("soft");

    await user.click(screen.getByRole("button", { name: "Reset to defaults" }));
    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "light", uiFont: "default" });
    expect(html().style.getPropertyValue("--accent-h")).toBe(String(DEFAULT_PREFERENCES.accentHue));
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it.each([
    ["refuses storage", blockStorage],
    ["has no room left in storage", fillStorage],
  ])("renders and applies changes when the browser %s", async (_, breakStorage) => {
    mockColorScheme("light");
    breakStorage();
    const { user } = await renderAppearance();
    expect(html().dataset).toMatchObject({ palette: "nothing", mode: "light", uiFont: "default" });

    await user.click(within(group("Palette")).getByRole("radio", { name: "Warm" }));
    expect(html().dataset["palette"]).toBe("warm");
    await user.click(screen.getByRole("button", { name: "Reset to defaults" }));
    expect(html().dataset["palette"]).toBe("nothing");
  });
});
