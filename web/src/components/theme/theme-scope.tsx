import { cn } from "cn";
import type { ComponentProps } from "react";
import type { Mode, Palette } from "@/theme/preferences";

type ThemeScopeProps = ComponentProps<"div"> & {
  palette: Palette;
  mode: Mode;
};

/**
 * A subtree drawn in another palette and mode than the page. The stylesheets
 * key every token on data-palette and data-mode, so everything inside
 * re-derives from them; the accent hue is inherited from the page.
 */
export function ThemeScope({ palette, mode, className, ...props }: ThemeScopeProps) {
  return (
    <div
      data-palette={palette}
      data-mode={mode}
      className={cn("bg-background text-foreground", className)}
      {...props}
    />
  );
}
