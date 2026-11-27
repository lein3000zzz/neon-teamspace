import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { refreshInbox } from "./inbox";
import { keys } from "./keys";

/**
 * Where the user's inbox tells that it changed: `/ws/inbox` on this origin,
 * over wss: when the page came over https:. The session cookie goes with the
 * upgrade, as with any request to the origin.
 */
export function inboxFeedServer(location: Pick<Location, "protocol" | "host">): string {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${location.host}/ws/inbox`;
}

/**
 * What follows the server closing the feed with `code`: stopping once the
 * session ended (4401); trying `later`, after a longer pause, when the client
 * fell behind (4408), sent too much (4429) or has too many feeds open (1013);
 * and trying `again` soon after anything else, such as a restart (1012).
 */
export function afterInboxClose(code: number): "again" | "later" | "signedOut" {
  switch (code) {
    case 4401:
      return "signedOut";
    case 1013:
    case 4408:
    case 4429:
      return "later";
    default:
      return "again";
  }
}

const FIRST_PAUSE_MS = 1000;
const LONGEST_PAUSE_MS = 30 * 1000;
const STEADY_MS = 30 * 1000;
const AGAIN_MS = 250;

/**
 * The inbox's live feed while the app is open: each message reads the inbox
 * again, its parts and its count, as the server tells only that it changed;
 * so does `ready`, as the feed connects and reconnects, for what it missed.
 * Returns how to close it.
 */
export function followInbox(queryClient: QueryClient): () => void {
  let socket: WebSocket | null = null;
  let pauses = 0;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let steady: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;

  const connect = () => {
    retry = undefined;
    const opened = new WebSocket(inboxFeedServer(window.location));
    socket = opened;
    opened.addEventListener("message", (message) => {
      if (socket !== opened || typeof message.data !== "string") {
        return;
      }
      let event: unknown;
      try {
        event = JSON.parse(message.data);
      } catch {
        return;
      }
      if (typeof event !== "object" || event === null || !("type" in event)) {
        return;
      }
      if (event.type === "ready") {
        clearTimeout(steady);
        steady = setTimeout(() => {
          pauses = 0;
        }, STEADY_MS);
      }
      refreshInbox(queryClient);
    });
    opened.addEventListener("close", (event) => {
      if (socket !== opened || stopped) {
        return;
      }
      socket = null;
      clearTimeout(steady);
      const next = afterInboxClose(event.code);
      if (next === "signedOut") {
        void queryClient.invalidateQueries({ queryKey: keys.me });
        return;
      }
      const pause =
        next === "again" && pauses === 0
          ? AGAIN_MS
          : Math.min(FIRST_PAUSE_MS * 2 ** pauses, LONGEST_PAUSE_MS);
      pauses += 1;
      retry = setTimeout(connect, pause);
    });
  };

  connect();
  return () => {
    stopped = true;
    clearTimeout(retry);
    clearTimeout(steady);
    const open = socket;
    socket = null;
    // Closing one still connecting makes the browser log an error: it closes once open.
    if (open?.readyState === WebSocket.CONNECTING) {
      open.addEventListener("open", () => open.close());
    } else {
      open?.close();
    }
  };
}

/** Follows the inbox live while the signed-in shell is shown. */
export function useInboxFeed() {
  const queryClient = useQueryClient();
  useEffect(() => followInbox(queryClient), [queryClient]);
}
