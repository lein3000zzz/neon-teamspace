import * as decoding from "lib0/decoding";
import { useMemo, useSyncExternalStore } from "react";
import { messagePermissionDenied } from "y-protocols/auth";
import { applyAwarenessUpdate, modifyAwarenessUpdate } from "y-protocols/awareness";
import { messageAuth, messageAwareness, WebsocketProvider } from "y-websocket";
import * as Y from "yjs";
import {
  STARTING,
  type ClosedBy,
  type Connection,
  type ConnectionStatus,
  type Peer,
} from "./connection-state";
import { cleanState, collaboratorOf } from "./cursor";

/**
 * The XmlFragment of a page's Yjs document that holds its blocks. The server's
 * `neon_blocks::FRAGMENT` names the same one; BlockNote's own default differs,
 * so the editor is always given this one.
 */
export const FRAGMENT = "document-store";

/** y-protocols' auth message for access given, which the server sends and y-protocols only names the other of. */
const messagePermissionGranted = 1;

/**
 * The server's own message type, beside y-protocols': a thread of the page's
 * comments changed. Its payload is JSON, `{"type":"comments","thread_id"}`;
 * the server sends it to connections that ask, with `?comments=true`.
 */
export const messageComments = 100;

/** The first pause after the server asked for one, doubled each time up to the longest. */
const FIRST_PAUSE_MS = 1000;
const LONGEST_PAUSE_MS = 30 * 1000;
/** How long a connection stays in sync before the pauses start again from the first. */
const STEADY_MS = 30 * 1000;

/**
 * Where a page's document is synced: `/ws/pages/{id}` on this origin, over
 * wss: when the page came over https:. The session cookie goes with the
 * upgrade, as with any request to the origin.
 */
export function syncServer(location: Pick<Location, "protocol" | "host">): string {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${location.host}/ws/pages`;
}

/**
 * What follows the server closing the connection with `code`: trying `again`
 * at once, as y-websocket does, trying `later`, after a pause, when it says the
 * client fell behind (4408) or sent too much (4429), or stopping, for why it
 * closed. It stops where sending the same again would be refused the same
 * way: a change too large (1009, 4413), a session that ended (4401), access
 * taken away (4403, 4404), and a message it could not read (1003, 1007, and
 * the rest of 4400 to 4499, as y-websocket has those). Everything else passes,
 * such as a restart (1012), too many connections (1013) or an update that
 * needs changes the server lacks (4422), which the next sync brings.
 */
export function afterClose(code: number): "again" | "later" | ClosedBy {
  switch (code) {
    case 4401:
      return "signedOut";
    case 4403:
      return "readOnly";
    case 4404:
      return "accessLost";
    case 1009:
    case 4413:
      return "tooLarge";
    case 4408:
    case 4429:
      return "later";
    case 1003:
    case 1007:
      return "failed";
    default:
      return code >= 4400 && code < 4500 && code !== 4422 ? "failed" : "again";
  }
}

export type SyncSession = { doc: Y.Doc; provider: WebsocketProvider };

type Snapshot = { session: SyncSession | null; connection: Connection };

const CLOSED_SNAPSHOT: Snapshot = { session: null, connection: STARTING };

/**
 * A page's document and its live connection, opened while something listens
 * and closed when the last listener goes, as React's external stores are.
 * Tabs never sync with each other directly: every change goes through the
 * server, which checks what the user may do.
 */
function syncSource(
  pageId: string,
  selfId: string,
  onAccessChanged: () => void,
  onComments: () => void,
) {
  let snapshot = CLOSED_SNAPSHOT;
  return {
    snapshot: () => snapshot,
    subscribe(notify: () => void) {
      const doc = new Y.Doc();
      const provider = new WebsocketProvider(syncServer(window.location), pageId, doc, {
        disableBc: true,
        params: { comments: "true" },
        shouldReconnect: (event) => afterClose(event.code) === "again",
      });
      const session = { doc, provider };
      let loaded = false;
      let pauses = 0;
      let pause: ReturnType<typeof setTimeout> | undefined;
      let steady: ReturnType<typeof setTimeout> | undefined;
      const publish = (patch: Partial<Connection>) => {
        snapshot = { session, connection: { ...snapshot.connection, ...patch, loaded } };
        notify();
      };
      const status = (next: ConnectionStatus) => publish({ status: next, closedBy: null });

      provider.on("sync", (synced) => {
        if (synced) {
          loaded = true;
          status("connected");
          clearTimeout(steady);
          steady = setTimeout(() => {
            pauses = 0;
          }, STEADY_MS);
        }
      });
      provider.on("connection-close", () => {
        clearTimeout(steady);
        // What the server said this connection may do ends with it; the next is asked afresh.
        if (provider.shouldConnect) {
          publish({ status: "reconnecting", closedBy: null, access: null });
        }
      });
      provider.on("closed", ({ code }) => {
        const next = afterClose(code);
        if (next === "again") {
          return;
        }
        if (next === "later") {
          status("reconnecting");
          pause = setTimeout(
            () => provider.connect(),
            Math.min(FIRST_PAUSE_MS * 2 ** pauses, LONGEST_PAUSE_MS),
          );
          pauses += 1;
          return;
        }
        publish({ status: "closed", closedBy: next, access: null });
        onAccessChanged();
      });

      // The server turns a connection read-only, or back, as access changes,
      // with an auth message: y-websocket itself only logs a denial.
      provider.messageHandlers[messageAuth] = (_encoder, decoder) => {
        const kind = decoding.readVarUint(decoder);
        if (kind === messagePermissionDenied) {
          decoding.readVarString(decoder);
          publish({ access: "view" });
        } else if (kind === messagePermissionGranted) {
          publish({ access: "edit" });
        } else {
          return;
        }
        onAccessChanged();
      };
      // A thread changed: the comments are read again, whatever the message says.
      provider.messageHandlers[messageComments] = (_encoder, decoder) => {
        decoding.readVarUint8Array(decoder);
        onComments();
      };
      // Others' presence is cleaned before the editor sees it: see cleanState.
      provider.messageHandlers[messageAwareness] = (_encoder, decoder, from) => {
        const update = decoding.readVarUint8Array(decoder);
        applyAwarenessUpdate(from.awareness, modifyAwarenessUpdate(update, cleanState), from);
      };

      const awareness = provider.awareness;
      // Only the others count, so the user's own presence, which the editor
      // sets as it is made, changes nothing here.
      const watchPeers = () => {
        const found = new Map<string, Peer>();
        for (const [clientId, state] of awareness.getStates()) {
          const peer = clientId === doc.clientID ? undefined : collaboratorOf(state["user"]);
          if (peer !== undefined && peer.id !== selfId) {
            found.set(peer.id, peer);
          }
        }
        const peers = [...found.values()];
        if (!samePeers(peers, snapshot.connection.peers)) {
          publish({ peers });
        }
      };
      awareness.on("change", watchPeers);

      snapshot = { session, connection: STARTING };
      notify();
      return () => {
        clearTimeout(pause);
        clearTimeout(steady);
        awareness.off("change", watchPeers);
        provider.destroy();
        doc.destroy();
        snapshot = CLOSED_SNAPSHOT;
      };
    },
  };
}

function samePeers(a: readonly Peer[], b: readonly Peer[]): boolean {
  return (
    a.length === b.length &&
    a.every((peer, index) => {
      const other = b[index];
      return (
        other !== undefined &&
        peer.id === other.id &&
        peer.name === other.name &&
        peer.hue === other.hue &&
        peer.kind === other.kind &&
        peer.owner === other.owner
      );
    })
  );
}

/**
 * Opens a page's document and its live connection, and closes both when the
 * page goes or changes. `onAccessChanged` runs when the server says the user
 * may now do more or less with the page, or ends the connection for good, so
 * the page can read again what the user may do with it, and `onComments`
 * when a thread of its comments changed; a new one opens the connection
 * afresh, so each should keep its identity.
 */
export function useSyncSession(
  pageId: string,
  selfId: string,
  onAccessChanged: () => void,
  onComments: () => void,
): Snapshot {
  const source = useMemo(
    () => syncSource(pageId, selfId, onAccessChanged, onComments),
    [pageId, selfId, onAccessChanged, onComments],
  );
  return useSyncExternalStore(source.subscribe, source.snapshot);
}
