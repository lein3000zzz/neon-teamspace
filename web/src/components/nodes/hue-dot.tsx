import { cn } from "cn";
import type { ComponentProps } from "react";
import { hueScope } from "./hue";

type HueDotProps = Omit<ComponentProps<"span">, "children" | "style"> & {
  /** An entity's hue; the user's accent when omitted. */
  hue?: number;
};

/**
 * A small accent-coloured dot: presence, or the colour an entity is drawn in.
 * It carries a hue, so it is one of the few things that glow in dark mode.
 * Decorative: whatever it marks must also be said in text.
 */
export function HueDot({ hue, className, ...props }: HueDotProps) {
  return (
    <span
      aria-hidden
      {...hueScope(hue)}
      className={cn("inline-block size-2 shrink-0 rounded-full bg-neon shadow-glow", className)}
      {...props}
    />
  );
}
