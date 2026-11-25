import type { Cover } from "@/api/pages";

/** Every cover, in the order the picker shows them: the accent's, single hues, then gradients. */
export const COVERS = [
  "accent",
  "accent-gradient",
  "red",
  "orange",
  "yellow",
  "green",
  "teal",
  "blue",
  "violet",
  "pink",
  "sunrise",
  "lagoon",
  "aurora",
  "dusk",
] as const satisfies readonly Cover[];

/** The hue of each single-hue cover, as the server documents them. */
const HUES = {
  red: 25,
  orange: 55,
  yellow: 90,
  green: 145,
  teal: 185,
  blue: 250,
  violet: 295,
  pink: 345,
} as const;

/** The two hues of each gradient, from left to right. */
const GRADIENTS = {
  sunrise: [55, 345],
  lagoon: [185, 250],
  aurora: [145, 295],
  dusk: [250, 345],
} as const;

/** A hue at the lightness and chroma the palette and mode give covers. */
function tone(hue: number | string): string {
  return `oklch(var(--cover-l) var(--cover-c) ${hue})`;
}

function gradient(from: number | string, to: number | string): string {
  return `linear-gradient(120deg, ${tone(from)}, ${tone(to)})`;
}

/**
 * The CSS background image of a cover. Each is drawn from the theme's tokens,
 * so it follows the reader's palette and mode: `accent` is the reader's
 * accent, and `accent-gradient` runs from it to the hue opposite.
 */
export function coverImage(cover: Cover): string {
  switch (cover) {
    case "accent":
      return gradient("var(--accent-h)", "var(--accent-h)");
    case "accent-gradient":
      return gradient("var(--accent-h)", "calc(var(--accent-h) + 180)");
    case "sunrise":
    case "lagoon":
    case "aurora":
    case "dusk": {
      const [from, to] = GRADIENTS[cover];
      return gradient(from, to);
    }
    default:
      return gradient(HUES[cover], HUES[cover]);
  }
}
