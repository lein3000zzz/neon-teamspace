import { useQuery } from "@tanstack/react-query";
import { ApiError } from "@/api/errors";
import { metaQuery } from "@/api/meta";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

/**
 * The sidebar footer: which instance this is and what server version it runs.
 * The <output> stays mounted across states so screen readers announce each
 * change. Status is never colour alone; each state has its own glyph and text.
 */
export function InstanceStatus() {
  const meta = useQuery(metaQuery);

  return (
    <div className="flex min-h-(--tap) items-center justify-between gap-2 text-xs">
      <output className="grid min-w-0 gap-0.5">
        {meta.isPending && (
          <span className="flex items-center gap-2 text-muted-foreground">
            <span aria-hidden>○</span>
            {strings.instance.connecting}
          </span>
        )}
        {meta.isError && (
          <span className="flex items-center gap-2 text-muted-foreground">
            <span aria-hidden className="text-bad">
              ×
            </span>
            {failureText(meta.error)}
          </span>
        )}
        {meta.isSuccess && (
          <>
            <span className="flex min-w-0 items-center gap-2">
              <span aria-hidden className="text-ok">
                ●
              </span>
              <span className="truncate font-medium text-foreground">{meta.data.name}</span>
            </span>
            <span className="pl-4 font-mono text-muted-foreground">
              {strings.instance.version(meta.data.version)}
            </span>
          </>
        )}
      </output>
      {meta.isError && (
        <Button
          variant="ghost"
          size="sm"
          disabled={meta.isFetching}
          onClick={() => void meta.refetch()}
        >
          {strings.instance.retry}
        </Button>
      )}
    </div>
  );
}

/** Anything but an answer from the server itself means it could not be reached. */
function failureText(error: unknown): string {
  if (error instanceof ApiError && !error.upstreamDown) {
    return strings.instance.serverError(error.status);
  }
  return strings.instance.unreachable;
}
