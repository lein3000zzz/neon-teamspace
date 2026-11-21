/** Named starting points for the accent slider, spread around the hue wheel. */
export const ACCENT_PRESETS = [
  { name: "Red", hue: 25 },
  { name: "Amber", hue: 70 },
  { name: "Lime", hue: 125 },
  { name: "Green", hue: 155 },
  { name: "Cyan", hue: 200 },
  { name: "Blue", hue: 250 },
  { name: "Violet", hue: 295 },
  { name: "Magenta", hue: 340 },
] as const satisfies readonly { name: string; hue: number }[];
