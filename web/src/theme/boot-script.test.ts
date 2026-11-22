import { describe, expect, it } from "vitest";
import bootScript from "../../public/theme.js?raw";
import { mockColorScheme } from "@/test/color-scheme";
import { blockStorage, fillStorage } from "@/test/storage";
import {
  applyTheme,
  browserStorage,
  DEFAULT_PREFERENCES,
  FALLBACK_ACCENT_HUE,
  FOLLOWED_HUE_KEY,
  loadFollowedHue,
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

/** What is stored: the theme, and the hue of the workspace last in view. */
const cases: [string, string | null, string | null][] = [
  ["nothing stored", null, null],
  [
    "a full record",
    JSON.stringify({ palette: "warm", mode: "dark", accentHue: 0, uiFont: "mono" }),
    null,
  ],
  ["system mode", JSON.stringify({ palette: "soft", mode: "system", accentHue: 359 }), null],
  [
    "invalid values",
    JSON.stringify({ palette: "neon", mode: "sepia", accentHue: 360, uiFont: 1 }),
    null,
  ],
  ["a fractional hue", JSON.stringify({ accentHue: 12.5 }), null],
  ["a hue as a string", JSON.stringify({ accentHue: "120" }), null],
  ["an array", JSON.stringify(["warm"]), null],
  ["broken JSON", "{palette:", null],
  ["a workspace's hue and no accent", JSON.stringify({ palette: "soft" }), "120"],
  ["a workspace's hue and an own accent", JSON.stringify({ accentHue: 40 }), "120"],
  ["a workspace's hue alone", null, "359"],
  ["a workspace's hue past the wheel", null, "360"],
  ["a workspace's hue as a word", null, "blue"],
];

describe("public/theme.js", () => {
  describe.each(["light", "dark"] as const)("with a %s system scheme", (scheme) => {
    it.each(cases)("agrees with preferences.ts for %s", (_, raw, followed) => {
      mockColorScheme(scheme);
      if (raw !== null) {
        window.localStorage.setItem(STORAGE_KEY, raw);
      }
      if (followed !== null) {
        window.localStorage.setItem(FOLLOWED_HUE_KEY, followed);
      }

      runBootScript();
      const fromScript = snapshot(document.documentElement);

      const expected = document.createElement("div");
      const preferences = loadPreferences(browserStorage());
      applyTheme(
        expected,
        preferences,
        resolveMode(preferences.mode, scheme === "dark"),
        loadFollowedHue(browserStorage()),
      );
      expect(fromScript).toEqual(snapshot(expected));
    });
  });

  it("draws someone without an accent of their own in the workspace's hue", () => {
    mockColorScheme("light");
    window.localStorage.setItem(FOLLOWED_HUE_KEY, "120");
    runBootScript();
    expect(snapshot(document.documentElement).hue).toBe("120");
  });

  it("applies the defaults when the browser refuses storage", () => {
    mockColorScheme("dark");
    blockStorage();

    expect(runBootScript).not.toThrow();
    expect(snapshot(document.documentElement)).toEqual({
      palette: DEFAULT_PREFERENCES.palette,
      mode: "dark",
      uiFont: DEFAULT_PREFERENCES.uiFont,
      hue: String(FALLBACK_ACCENT_HUE),
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
