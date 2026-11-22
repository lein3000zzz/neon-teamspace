import { useQuery } from "@tanstack/react-query";
import { useId, type ReactNode } from "react";
import { metaQuery } from "@/api/meta";
import { HueDot } from "@/components/nodes/hue-dot";
import { strings } from "@/lib/strings";

/**
 * The frame of the pages for people who are not signed in: the instance's
 * name over one card, centred on the dot grid, a phone's full width less its
 * margins and safe areas.
 */
export function AuthLayout({ children }: { children: ReactNode }) {
  const meta = useQuery(metaQuery);
  return (
    <div className="flex min-h-dvh flex-col items-center gap-6 safe-top-10 safe-right-4 safe-bottom-10 safe-left-4 sm:justify-center">
      <p className="flex min-h-(--tap) items-center gap-2.5 text-sm font-semibold tracking-tight">
        <HueDot className="size-2.5" />
        {meta.data?.name ?? strings.product}
      </p>
      <main id="main" tabIndex={-1} className="w-full max-w-sm">
        {children}
      </main>
    </div>
  );
}

type AuthCardProps = {
  title: ReactNode;
  lede?: ReactNode;
  /** Links to the other forms, under a rule. */
  footer?: ReactNode;
  children?: ReactNode;
};

/** One form or message of the signed-out pages, under its heading. */
export function AuthCard({ title, lede, footer, children }: AuthCardProps) {
  const headingId = useId();
  return (
    <section
      aria-labelledby={headingId}
      className="grid gap-5 rounded-lg border border-border bg-raised p-5 sm:p-6"
    >
      <header className="grid gap-2">
        <h1 id={headingId} className="text-2xl font-semibold">
          {title}
        </h1>
        {lede !== undefined && <p className="text-sm text-muted-foreground">{lede}</p>}
      </header>
      {children}
      {footer !== undefined && (
        <div className="grid gap-1 border-t border-border pt-4 text-sm text-muted-foreground">
          {footer}
        </div>
      )}
    </section>
  );
}

/** A footer line: a question, then the link that answers it. */
export function AuthFooterLine({ question, children }: { question: string; children: ReactNode }) {
  return (
    <p className="flex flex-wrap items-center gap-x-1.5">
      {question}
      {children}
    </p>
  );
}

/** A text link styled for the auth cards, still a full touch target tall. */
export const authLink =
  "inline-flex min-h-(--tap) items-center text-foreground underline underline-offset-4 hover:text-primary";
