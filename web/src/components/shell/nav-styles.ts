// The router marks the current link with aria-current="page"; the accent bar
// on its left edge is the active item border, which glows in dark mode.
export const navLink =
  "relative flex min-h-(--tap) items-center gap-2.5 rounded-md px-2.5 text-sm text-muted-foreground transition-colors before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:rounded-full before:bg-primary before:opacity-0 before:shadow-glow hover:bg-accent hover:text-foreground aria-[current=page]:bg-accent aria-[current=page]:text-foreground aria-[current=page]:before:opacity-100";

/** The small capitals over a group of links. */
export const navHeading =
  "px-2.5 font-mono text-[0.68rem] tracking-widest text-muted-foreground uppercase";
