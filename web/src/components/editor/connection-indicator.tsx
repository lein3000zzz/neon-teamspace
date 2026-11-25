import { cn } from "cn";
import { HueDot } from "@/components/nodes/hue-dot";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { strings } from "@/lib/strings";
import type { Connection } from "./connection-state";
import { collaboratorLabel } from "./cursor";

const copy = strings.pages.editor;

/** At most this many dots for the others on the page; their names are all said. */
const SHOWN_PEERS = 4;

type ConnectionIndicatorProps = {
  connection: Connection;
  /** Whether the user only views the page: once in sync it says so instead of connected. */
  readOnly: boolean;
};

/**
 * How the page's live connection stands, in a word and a dot that glows in the
 * accent while it is in sync, and who else has the page open, each a dot in
 * their own hue. Announced politely as it changes.
 */
export function ConnectionIndicator({ connection, readOnly }: ConnectionIndicatorProps) {
  const state = connection.status === "connected" && readOnly ? "readOnly" : connection.status;
  const live = connection.status === "connected";
  const names = connection.peers.map(collaboratorLabel).join(", ");
  return (
    <div className="flex items-center gap-2">
      {connection.peers.length > 0 && (
        <p className="flex items-center gap-1">
          <span className="flex items-center -space-x-0.5" aria-hidden>
            {connection.peers.slice(0, SHOWN_PEERS).map((peer) => (
              <HueDot key={peer.id} hue={peer.hue} className="size-2.5 ring-2 ring-background" />
            ))}
          </span>
          <span className="sr-only">{copy.here(names)}</span>
        </p>
      )}
      <output
        title={copy.hints[state]}
        className="inline-flex h-(--tap-sm) items-center gap-1.5 rounded-full border border-border px-2.5 text-xs text-muted-foreground"
      >
        <span
          aria-hidden
          className={cn(
            "inline-block size-2 rounded-full",
            live ? "bg-primary shadow-glow" : "border border-muted-foreground",
          )}
        />
        {copy.states[state]}
        <span className="sr-only">. {copy.hints[state]}</span>
      </output>
    </div>
  );
}

type ConnectionClosedProps = {
  connection: Connection;
  /** Opens the connection afresh, from what the server holds. */
  onReconnect: () => void;
};

/** Why the server ended the page's live connection for good, and the way to start again. */
export function ConnectionClosed({ connection, onReconnect }: ConnectionClosedProps) {
  if (connection.status !== "closed" || connection.closedBy === null) {
    return null;
  }
  return (
    <Alert tone="error" live="assertive">
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-foreground">{copy.closed[connection.closedBy]}</p>
        <Button variant="outline" size="sm" onClick={onReconnect}>
          {copy.reconnect}
        </Button>
      </div>
    </Alert>
  );
}
