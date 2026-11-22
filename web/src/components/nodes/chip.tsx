import { cn } from "cn";
import { createContext, use, type ComponentProps } from "react";
import { HueDot } from "./hue-dot";
import { hueScope } from "./hue";

const InList = createContext(false);

/** Node shape 2: chips in a wrapping row. */
export function ChipList({ className, ...props }: ComponentProps<"ul">) {
  return (
    <InList value={true}>
      <ul className={cn("m-0 flex list-none flex-wrap gap-1.5 p-0", className)} {...props} />
    </InList>
  );
}

type ChipProps = Omit<ComponentProps<"span">, "style" | "ref"> & {
  /** "strong" outlines the chip in its accent, for the one chip that matters. */
  tone?: "default" | "strong";
  /** An entity's hue: adds a leading dot and scopes "strong" to that hue. */
  hue?: number;
};

/** A list item inside a ChipList, an inline span anywhere else. */
export function Chip({ tone = "default", hue, className, children, ...props }: ChipProps) {
  const Element = use(InList) ? "li" : "span";
  return (
    <Element
      {...hueScope(hue)}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs",
        tone === "strong"
          ? "border-neon text-neon"
          : "border-border bg-background text-muted-foreground",
        className,
      )}
      {...props}
    >
      {hue !== undefined && <HueDot className="size-1.5" />}
      {children}
    </Element>
  );
}
