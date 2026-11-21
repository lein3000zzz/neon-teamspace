import type { CSSProperties } from "react";

/**
 * Props that make an element carry its own hue instead of the user's accent:
 * `data-hue` re-derives the neon tokens and `--accent-h` sets the hue. React
 * writes the custom property through CSSOM, which a strict CSP allows.
 */
export function hueScope(hue: number | undefined): { "data-hue"?: ""; style?: CSSProperties } {
  if (hue === undefined) {
    return {};
  }
  return { "data-hue": "", style: { "--accent-h": String(hue) } as CSSProperties };
}
