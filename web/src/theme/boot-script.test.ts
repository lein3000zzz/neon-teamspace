import { describe, expect, it } from "vitest";
import bootScript from "../../public/theme.js?raw";
import { mockColorScheme } from "@/test/color-scheme";
import { blockStorage, fillStorage } from "@/test/storage";
import {
  applyTheme,
  browserStorage,
  DEFAULT_PREFERENCES,
  loadPreferences,
  resolveMode,
  STORAGE_KEY,
} from "./preferences";

/** Runs public/theme.js the way the browser does: once, against the live document. */
function runBootScript() {
  new Function(bootScript)();
}

function snapshot(element: HTMLElement) {
  return {
    palette: element.dataset["palette"],
    mode: element.dataset["mode"],
    uiFont: element.dataset["uiFont"],
    hue: element.style.getPropertyValue("--accent-h"),
  };
}

const cases: [string, string | null][] = [
  ["nothing stored", null],
  [
    "a full record",
    JSON.stringify({ palette: "warm", mode: "dark", accentHue: 0, uiFont: "mono" }),
  ],
  ["system mode", JSON.stringify({ palette: "soft", mode: "system", accentHue: 359 })],
  ["invalid values", JSON.stringify({ palette: "neon", mode: "sepia", accentHue: 360, uiFont: 1 })],
  ["a fractional hue", JSON.stringify({ accentHue: 12.5 })],
  ["a hue as a string", JSON.stringify({ accentHue: "120" })],
  ["an array", JSON.stringify(["warm"])],
  ["broken JSON", "{palette:"],
];

describe("public/theme.js", () => {
  describe.each(["light", "dark"] as const)("with a %s system scheme", (scheme) => {
    it.each(cases)("agrees with preferences.ts for %s", (_, raw) => {
      mockColorScheme(scheme);
      if (raw !== null) {
        window.localStorage.setItem(STORAGE_KEY, raw);
      }

      runBootScript();
      const fromScript = snapshot(document.documentElement);

      const expected = document.createElement("div");
      const preferences = loadPreferences(browserStorage());
      applyTheme(expected, preferences, resolveMode(preferences.mode, scheme === "dark"));
      expect(fromScript).toEqual(snapshot(expected));
    });
  });

  it("applies the defaults when the browser refuses storage", () => {
    mockColorScheme("dark");
    blockStorage();

    expect(runBootScript).not.toThrow();
    expect(snapshot(document.documentElement)).toEqual({
      palette: DEFAULT_PREFERENCES.palette,
      mode: "dark",
      uiFont: DEFAULT_PREFERENCES.uiFont,
      hue: String(DEFAULT_PREFERENCES.accentHue),
    });
  });

  it("still reads the stored theme when storage is full", () => {
    mockColorScheme("light");
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ palette: "warm", mode: "dark" }));
    fillStorage();

    runBootScript();
    expect(snapshot(document.documentElement)).toMatchObject({ palette: "warm", mode: "dark" });
  });
});
