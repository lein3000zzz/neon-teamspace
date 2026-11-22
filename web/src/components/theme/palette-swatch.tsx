import type { Mode, Palette } from "@/theme/preferences";
import { ThemeScope } from "./theme-scope";

/** A thumbnail of a palette: its ground and dot grid, ink, a rule and the accent. */
export function PaletteSwatch({ palette, mode }: { palette: Palette; mode: Mode }) {
  return (
    <ThemeScope
      palette={palette}
      mode={mode}
      aria-hidden
      className="grid h-20 w-full content-end gap-2 rounded-lg border border-border dot-grid p-2.5 text-left"
    >
      <span className="text-base leading-none font-semibold">Aa</span>
      <span className="flex items-center gap-1.5">
        <span className="h-2 w-10 rounded-full bg-primary shadow-glow" />
        <span className="h-2 w-6 rounded-full border border-border" />
      </span>
    </ThemeScope>
  );
}
