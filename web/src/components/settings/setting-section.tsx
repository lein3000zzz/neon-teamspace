import { useId, type ReactNode } from "react";

type SettingSectionProps = {
  title: ReactNode;
  hint?: ReactNode;
  /** Receives the heading's id, so a control group can be named by it. */
  children: (headingId: string) => ReactNode;
};

/** One setting: a heading, a line on what it changes, then its control. */
export function SettingSection({ title, hint, children }: SettingSectionProps) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="grid gap-3 border-t border-border pt-6">
      <div className="grid gap-1">
        <h2 id={headingId} className="text-base font-semibold">
          {title}
        </h2>
        {hint !== undefined && <p className="text-sm text-muted-foreground">{hint}</p>}
      </div>
      {children(headingId)}
    </section>
  );
}
