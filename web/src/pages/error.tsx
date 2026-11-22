import type { ErrorComponentProps } from "@tanstack/react-router";
import { PageHeader } from "@/components/shell/page-header";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

/** Shown in place of a route that threw while rendering or loading. */
export function ErrorPage({ error }: ErrorComponentProps) {
  return (
    <PageHeader
      eyebrow={strings.errors.eyebrow}
      title={strings.errors.title}
      lede={strings.errors.lede}
    >
      <pre className="mt-2 overflow-x-auto rounded-lg border border-border bg-card p-3 font-mono text-xs whitespace-pre-wrap text-muted-foreground">
        {error instanceof Error ? error.message : String(error)}
      </pre>
      <div className="mt-3">
        <Button onClick={() => window.location.reload()}>{strings.errors.reload}</Button>
      </div>
    </PageHeader>
  );
}
