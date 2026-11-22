import type { ReactNode } from "react";
import { ApiError } from "@/api/errors";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { formatWait } from "@/lib/format";
import { strings } from "@/lib/strings";

const copy = strings.errors;

/**
 * What went wrong, for people. The server's own message where it sent one; a
 * wait in words for a rate limit, from its Retry-After; a plain "not allowed"
 * for a 403; and a hint about the connection when the server was not reached.
 */
export function errorText(error: unknown): string {
  if (!(error instanceof ApiError) || error.upstreamDown) {
    return copy.unreachable;
  }
  if (error.status === 429) {
    return error.retryAfter === undefined
      ? copy.rateLimitedSoon
      : copy.rateLimited(formatWait(error.retryAfter));
  }
  if (error.status === 403 && (error.code === "forbidden" || error.code === "insufficient_scope")) {
    return copy.forbidden;
  }
  if (!error.fromServer) {
    return copy.status(error.status);
  }
  return sentence(error.message);
}

/** The server writes its messages in lower case, to be read inside a sentence. */
export function sentence(message: string): string {
  const text = message.trim();
  const capital = text.charAt(0).toUpperCase() + text.slice(1);
  return /[.!?]$/.test(capital) ? capital : `${capital}.`;
}

type ErrorNoticeProps = {
  error: unknown;
  title?: ReactNode;
  onRetry?: () => void;
  retrying?: boolean;
  children?: ReactNode;
};

/** An error in a red-bordered box, announced as it appears, with a retry when one helps. */
export function ErrorNotice({ error, title, onRetry, retrying, children }: ErrorNoticeProps) {
  return (
    <Alert tone="error" live="assertive">
      {title !== undefined && <AlertTitle>{title}</AlertTitle>}
      <AlertDescription>
        <p className={title === undefined ? "text-foreground" : undefined}>{errorText(error)}</p>
        {children}
        {onRetry && (
          <div>
            <Button variant="outline" size="sm" disabled={retrying ?? false} onClick={onRetry}>
              {copy.retry}
            </Button>
          </div>
        )}
      </AlertDescription>
    </Alert>
  );
}

/** Shown in place of something still loading. */
export function Loading({ label = copy.loading }: { label?: string }) {
  return <output className="block text-sm text-muted-foreground">{label}</output>;
}
