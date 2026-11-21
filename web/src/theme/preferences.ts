/**
 * Theme preferences: what the user picked, how it is stored, and how it maps
 * onto the attributes the stylesheets key on.
 *
 * `public/theme.js` repeats the parsing and the attribute writes in plain
 * JavaScript so the theme is applied before first paint. The two are kept in
 * step by `boot-script.test.ts`.
 */

export const PALETTES = ["nothing", "soft", "warm"] as const;
export const MODE_PREFERENCES = ["system", "light", "dark"] as const;
export const UI_FONTS = ["default", "mono"] as const;

export type Palette = (typeof PALETTES)[number];
export type ModePreference = (typeof MODE_PREFERENCES)[number];
export type Mode = Exclude<ModePreference, "system">;
export type UiFont = (typeof UI_FONTS)[number];

export interface ThemePreferences {
  palette: Palette;
  mode: ModePreference;
  /** Integer hue in degrees, 0..359. */
  accentHue: number;
  uiFont: UiFont;
}

/** The largest accent hue; 360 would be 0 again. */
export const HUE_MAX = 359;

export const STORAGE_KEY = "neon.theme";
export const DARK_QUERY = "(prefers-color-scheme: dark)";

export const DEFAULT_PREFERENCES: Readonly<ThemePreferences> = Object.freeze({
  palette: "nothing",
  mode: "system",
  accentHue: 250,
  uiFont: "default",
});

/** Narrows an untrusted value, from storage or a control, to one of the allowed options. */
export function matchOption<T extends string>(
  options: readonly T[],
  value: unknown,
): T | undefined {
  return options.find((option) => option === value);
}

export function isHue(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= HUE_MAX;
}

/**
 * Reads stored preferences. Each field is validated on its own, so one bad or
 * missing value falls back to its default without discarding the others.
 */
export function parsePreferences(raw: string | null): ThemePreferences {
  let stored: Record<string, unknown> = {};
  try {
    const value: unknown = raw === null ? null : JSON.parse(raw);
    if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      stored = value as Record<string, unknown>;
    }
  } catch {
    // Unparseable storage reads as empty.
  }
  return {
    palette: matchOption(PALETTES, stored["palette"]) ?? DEFAULT_PREFERENCES.palette,
    mode: matchOption(MODE_PREFERENCES, stored["mode"]) ?? DEFAULT_PREFERENCES.mode,
    accentHue: isHue(stored["accentHue"]) ? stored["accentHue"] : DEFAULT_PREFERENCES.accentHue,
    uiFont: matchOption(UI_FONTS, stored["uiFont"]) ?? DEFAULT_PREFERENCES.uiFont,
  };
}

/** Storage can throw: private windows, blocked site data, a full quota. */
export function loadPreferences(storage: Storage | undefined): ThemePreferences {
  try {
    return parsePreferences(storage?.getItem(STORAGE_KEY) ?? null);
  } catch {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function savePreferences(storage: Storage | undefined, preferences: ThemePreferences) {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // A theme that does not persist still applies for this visit.
  }
}

/** Forgets the stored choice, so later changes to the defaults reach this user too. */
export function clearPreferences(storage: Storage | undefined) {
  try {
    storage?.removeItem(STORAGE_KEY);
  } catch {
    // Nothing stored that could be read back.
  }
}

export function resolveMode(preference: ModePreference, systemPrefersDark: boolean): Mode {
  if (preference === "system") {
    return systemPrefersDark ? "dark" : "light";
  }
  return preference;
}

/**
 * Writes the theme onto an element. The hue goes through CSSOM rather than a
 * style attribute, which a strict CSP without 'unsafe-inline' still allows.
 */
export function applyTheme(element: HTMLElement, preferences: ThemePreferences, mode: Mode) {
  element.dataset["palette"] = preferences.palette;
  element.dataset["mode"] = mode;
  element.dataset["uiFont"] = preferences.uiFont;
  element.style.setProperty("--accent-h", String(preferences.accentHue));
}
