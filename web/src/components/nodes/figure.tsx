import { cn } from "cn";
import type { ComponentProps, ReactNode } from "react";

/** Node shape 4: a grid of figures that adds columns as the width allows. */
export function FigureGrid({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      className={cn("grid grid-cols-[repeat(auto-fit,minmax(6.5rem,1fr))] gap-4", className)}
      {...props}
    />
  );
}

type FigureProps = Omit<ComponentProps<"div">, "children"> & {
  value: ReactNode;
  label: ReactNode;
  caption?: ReactNode;
};

/**
 * Three ranks, so a figure reads as number, name, explanation rather than a
 * number followed by two lines that look alike.
 */
export function Figure({ value, label, caption, className, ...props }: FigureProps) {
  return (
    <div className={cn("grid content-start gap-0.5", className)} {...props}>
      <span className="text-2xl leading-tight tracking-tight tabular-nums">{value}</span>
      <span className="text-[0.68rem] tracking-widest text-foreground uppercase">{label}</span>
      {caption !== undefined && (
        <span className="text-xs leading-snug text-pretty text-muted-foreground">{caption}</span>
      )}
    </div>
  );
}
