import { describe, expect, it } from "vitest";
import { blockStorage } from "@/test/storage";
import {
  applyTheme,
  browserStorage,
  DEFAULT_PREFERENCES,
  loadPreferences,
  parsePreferences,
  resolveMode,
  savePreferences,
  STORAGE_KEY,
} from "./preferences";

const stored = (value: unknown) => JSON.stringify(value);

describe("parsePreferences", () => {
  it("returns the defaults when nothing is stored", () => {
    expect(parsePreferences(null)).toEqual(DEFAULT_PREFERENCES);
  });

  it("reads a complete, valid record", () => {
    const value = { palette: "warm", mode: "dark", accentHue: 0, uiFont: "mono" };
    expect(parsePreferences(stored(value))).toEqual(value);
  });

  it.each([
    ["unparseable JSON", "{palette:"],
    ["a JSON array", stored(["warm"])],
    ["a JSON string", stored("warm")],
    ["a JSON number", "42"],
    ["JSON null", "null"],
  ])("falls back to the defaults for %s", (_, raw) => {
    expect(parsePreferences(raw)).toEqual(DEFAULT_PREFERENCES);
  });

  it("replaces each bad field on its own and keeps the good ones", () => {
    const value = { palette: "neon", mode: "dark", accentHue: "250", uiFont: "comic" };
    expect(parsePreferences(stored(value))).toEqual({
      ...DEFAULT_PREFERENCES,
      mode: "dark",
    });
  });

  it.each([
    [0, 0],
    [359, 359],
    [360, DEFAULT_PREFERENCES.accentHue],
    [-1, DEFAULT_PREFERENCES.accentHue],
    [12.5, DEFAULT_PREFERENCES.accentHue],
    [null, DEFAULT_PREFERENCES.accentHue],
  ])("accepts only whole hues in 0..359: %s reads as %s", (accentHue, expected) => {
    expect(parsePreferences(stored({ accentHue })).accentHue).toBe(expected);
  });
});

describe("resolveMode", () => {
  it("follows the system in system mode", () => {
    expect(resolveMode("system", true)).toBe("dark");
    expect(resolveMode("system", false)).toBe("light");
  });

  it("ignores the system when a mode is chosen", () => {
    expect(resolveMode("light", true)).toBe("light");
    expect(resolveMode("dark", false)).toBe("dark");
  });
});

describe("storage", () => {
  const throwing = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("quota");
    },
  } as unknown as Storage;

  it("round-trips through storage", () => {
    const value = { palette: "soft", mode: "light", accentHue: 125, uiFont: "default" } as const;
    savePreferences(window.localStorage, value);
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe(stored(value));
    expect(loadPreferences(window.localStorage)).toEqual(value);
  });

  it("survives storage that throws", () => {
    expect(loadPreferences(throwing)).toEqual(DEFAULT_PREFERENCES);
    expect(() => savePreferences(throwing, DEFAULT_PREFERENCES)).not.toThrow();
  });

  it("survives having no storage at all", () => {
    expect(loadPreferences(undefined)).toEqual(DEFAULT_PREFERENCES);
  });

  it("finds no storage where reading window.localStorage throws", () => {
    expect(browserStorage()).toBe(window.localStorage);
    blockStorage();
    expect(browserStorage()).toBeUndefined();
  });
});

describe("applyTheme", () => {
  it("writes the attributes and the hue the stylesheets key on", () => {
    const element = document.createElement("div");
    applyTheme(element, { palette: "warm", mode: "system", accentHue: 70, uiFont: "mono" }, "dark");
    expect(element.dataset).toMatchObject({ palette: "warm", mode: "dark", uiFont: "mono" });
    expect(element.style.getPropertyValue("--accent-h")).toBe("70");
  });
});
