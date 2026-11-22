import { useId, type ReactNode } from "react";

type PageSectionProps = {
  title: ReactNode;
  hint?: ReactNode;
  /** Beside the heading on wide screens, under it on narrow ones. */
  actions?: ReactNode;
  children?: ReactNode;
};

/** A part of a settings page: a rule, a heading, a line on what it is for, then its content. */
export function PageSection({ title, hint, actions, children }: PageSectionProps) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="grid gap-4 border-t border-border pt-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="grid max-w-[62ch] gap-1">
          <h2 id={headingId} className="text-base font-semibold">
            {title}
          </h2>
          {hint !== undefined && <p className="text-sm text-muted-foreground">{hint}</p>}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}

/** A card in a list of things on a settings page, such as a session or an invite. */
export function ItemCard({ children, ...props }: { children: ReactNode; "aria-label"?: string }) {
  return (
    <li
      className="grid gap-3 rounded-lg border border-border bg-card p-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start"
      {...props}
    >
      {children}
    </li>
  );
}

/** The buttons of an item card, wrapping under its details on a phone. */
export function ItemActions({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap gap-2 sm:justify-end">{children}</div>;
}
