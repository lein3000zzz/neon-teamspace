import { cn } from "cn";
import { ArrowRight } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";

/** Node shape 3: numbered steps joined by arrows, wrapping on narrow screens. */
export function StepFlow({ className, ...props }: ComponentProps<"ol">) {
  return (
    <ol
      className={cn(
        "m-0 flex list-none flex-wrap items-center gap-2 p-0 [counter-reset:step]",
        className,
      )}
      {...props}
    />
  );
}

type StepNodeProps = Omit<ComponentProps<"li">, "title"> & {
  title: ReactNode;
  detail?: ReactNode;
  /** A measurement for the step, set in mono with tabular figures. */
  timing?: ReactNode;
  /** The step currently in play: accent border, and a glow in dark mode. */
  active?: boolean;
};

export function StepNode({ title, detail, timing, active, className, ...props }: StepNodeProps) {
  return (
    <li
      aria-current={active ? "step" : undefined}
      className={cn("group flex items-center gap-2 [counter-increment:step]", className)}
      {...props}
    >
      <div
        className={cn(
          "grid grid-cols-[1.5rem_minmax(0,1fr)] items-start gap-x-2 rounded-lg border bg-card px-3 py-2",
          active ? "border-primary shadow-glow" : "border-border",
        )}
      >
        <span
          aria-hidden
          className={cn(
            "row-span-2 grid size-6 place-items-center rounded-full border font-mono text-xs before:content-[counter(step)]",
            active
              ? "border-primary bg-primary text-primary-foreground"
              : "border-border text-muted-foreground",
          )}
        />
        <span className="flex items-baseline justify-between gap-3 text-sm">
          <span>{title}</span>
          {timing !== undefined && (
            <span className="font-mono text-xs whitespace-nowrap text-muted-foreground tabular-nums">
              {timing}
            </span>
          )}
        </span>
        {detail !== undefined && <span className="text-xs text-muted-foreground">{detail}</span>}
      </div>
      <ArrowRight aria-hidden className="size-4 text-muted-foreground group-last:hidden" />
    </li>
  );
}
