import { cn } from "cn";
import type { FeedState } from "@/api/database-feed";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";

const copy = strings.databases.live;

/**
 * How the database's live changes stand, in a word and a dot that glows in
 * the accent while they come in, announced politely as it changes. Compact,
 * inside another page, it is the dot alone, its words said to screen readers.
 */
export function FeedIndicator({ state, compact }: { state: FeedState; compact: boolean }) {
  const live = state.status === "connected";
  return (
    <output
      title={copy.hints[state.status]}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground",
        !compact && "h-(--tap-sm) rounded-full border border-border px-2.5",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "inline-block size-2 rounded-full",
          live ? "bg-primary shadow-glow" : "border border-muted-foreground",
        )}
      />
      <span className={cn(compact && "sr-only")}>{copy.states[state.status]}</span>
      <span className="sr-only">. {copy.hints[state.status]}</span>
    </output>
  );
}

/** Why the server ended the live changes for good, and the way to start them again. */
export function FeedClosed({ state, onRestart }: { state: FeedState; onRestart: () => void }) {
  if (state.status !== "closed" || state.closedBy === null) {
    return null;
  }
  return (
    <Alert tone="error" live="assertive">
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-foreground">{copy.closed[state.closedBy]}</p>
        <Button variant="outline" size="sm" onClick={onRestart}>
          {copy.reconnect}
        </Button>
      </div>
    </Alert>
  );
}
