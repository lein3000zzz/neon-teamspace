/** The web app's palettes, from web/src/styles/palettes.css. */
export const PALETTES = ["nothing", "soft", "warm"] as const;
export type Palette = (typeof PALETTES)[number];

/** Where the chosen palette is kept; public/theme.js reads it before first paint. */
export const PALETTE_KEY = "neon.shell.palette";

function isPalette(value: string | null): value is Palette {
  return PALETTES.some((palette) => palette === value);
}

export function storedPalette(): Palette {
  try {
    const value = window.localStorage.getItem(PALETTE_KEY);
    return isPalette(value) ? value : "nothing";
  } catch {
    return "nothing";
  }
}

export function applyPalette(palette: Palette): void {
  document.documentElement.setAttribute("data-palette", palette);
  try {
    window.localStorage.setItem(PALETTE_KEY, palette);
  } catch {
    // Blocked storage: the palette holds until the page reloads.
  }
}

/** Keeps the mode on the system's light or dark setting as it changes. */
export function followTheSystem(): void {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () =>
    document.documentElement.setAttribute("data-mode", query.matches ? "dark" : "light");
  apply();
  query.addEventListener("change", apply);
}

/** The palette radios: checked as stored, and applied as chosen. */
export function setUpPaletteChoice(fieldset: HTMLFieldSetElement | null): void {
  if (fieldset === null) {
    return;
  }
  const current = storedPalette();
  for (const radio of fieldset.querySelectorAll<HTMLInputElement>('input[name="palette"]')) {
    radio.checked = radio.value === current;
    radio.addEventListener("change", () => {
      if (radio.checked && isPalette(radio.value)) {
        applyPalette(radio.value);
      }
    });
  }
}
