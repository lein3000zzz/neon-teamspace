import { cn } from "cn";
import { hueScope } from "./hue";

type EntityMarkProps = {
  name: string;
  /** An emoji or a short text; the name's first letter without one. */
  icon?: string | null | undefined;
  hue: number;
  size?: "sm" | "md" | "lg";
  className?: string;
};

const SIZES = {
  sm: "size-5 text-[0.7rem]",
  md: "size-7 text-sm",
  lg: "size-11 text-xl",
} as const;

const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** The first `count` characters as a reader sees them, so an emoji never splits. */
function leading(text: string, count: number): string {
  return Array.from(graphemes.segment(text), (part) => part.segment)
    .slice(0, count)
    .join("");
}

/**
 * The square a workspace or teamspace is drawn with: its icon, or the first
 * letter of its name, outlined in its own hue. Decorative: its name is always
 * beside it in text.
 */
export function EntityMark({ name, icon, hue, size = "md", className }: EntityMarkProps) {
  const trimmed = icon?.trim() ?? "";
  const glyph = trimmed === "" ? leading(name.trim(), 1).toUpperCase() : leading(trimmed, 2);
  return (
    <span
      aria-hidden
      {...hueScope(hue)}
      className={cn(
        "inline-grid shrink-0 place-items-center overflow-hidden rounded-md border border-neon bg-neon-soft leading-none font-medium text-neon",
        SIZES[size],
        className,
      )}
    >
      {glyph}
    </span>
  );
}
