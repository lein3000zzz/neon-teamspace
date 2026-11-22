import { useMemo, useSyncExternalStore } from "react";
import { WebsocketProvider } from "y-websocket";
import * as Y from "yjs";
import { STARTING, type Connection, type ConnectionStatus, type Peer } from "./connection-state";
import { collaboratorOf } from "./cursor";

/**
 * The XmlFragment of a page's Yjs document that holds its blocks. The server's
 * `neon_blocks::FRAGMENT` names the same one; BlockNote's own default differs,
 * so the editor is always given this one.
 */
export const FRAGMENT = "document-store";

/**
 * Where a page's document is synced: `/ws/pages/{id}` on this origin, over
 * wss: when the page came over https:. The session cookie goes with the
 * upgrade, as with any request to the origin.
 */
export function syncServer(location: Pick<Location, "protocol" | "host">): string {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${location.host}/ws/pages`;
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
function syncSource(pageId: string, selfId: string, onClosed: () => void) {
  let snapshot = CLOSED_SNAPSHOT;
  return {
    snapshot: () => snapshot,
    subscribe(notify: () => void) {
      const doc = new Y.Doc();
      const provider = new WebsocketProvider(syncServer(window.location), pageId, doc, {
        disableBc: true,
      });
      const session = { doc, provider };
      let loaded = false;
      const publish = (patch: Partial<Connection>) => {
        snapshot = { session, connection: { ...snapshot.connection, ...patch, loaded } };
        notify();
      };
      const status = (next: ConnectionStatus) => publish({ status: next });

      provider.on("sync", (synced) => {
        if (synced) {
          loaded = true;
          status("connected");
        }
      });
      provider.on("connection-close", () => {
        if (provider.shouldConnect) {
          status("reconnecting");
        }
      });
      provider.on("closed", () => {
        status("closed");
        onClosed();
      });
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
        peer.hue === other.hue
      );
    })
  );
}

/**
 * Opens a page's document and its live connection, and closes both when the
 * page goes or changes. `onClosed` runs when the server ends the connection
 * for good, so the page can read again what the user may now do with it; a
 * new one opens the connection afresh, so it should keep its identity.
 */
export function useSyncSession(pageId: string, selfId: string, onClosed: () => void): Snapshot {
  const source = useMemo(() => syncSource(pageId, selfId, onClosed), [pageId, selfId, onClosed]);
  return useSyncExternalStore(source.subscribe, source.snapshot);
}
