import type { Collaborator } from "./cursor";

/**
 * How the page's live connection stands:
 * - `connecting`: the first connection, until the server has sent the document;
 * - `connected`: in sync, changes go both ways as they are made;
 * - `reconnecting`: the connection dropped; it is tried again, with changes
 *   made meanwhile kept here and sent once it is back;
 * - `closed`: the server ended it for good, with a close code from 4400 to 4499,
 *   as when the page is no longer visible to the user; it is not tried again.
 */
export type ConnectionStatus = "connecting" | "connected" | "reconnecting" | "closed";

export type Peer = Omit<Collaborator, "color">;

export type Connection = {
  status: ConnectionStatus;
  /** Whether the document came from the server at least once, so its content is the page's. */
  loaded: boolean;
  /** The others with the page open, each once however many tabs they have. */
  peers: Peer[];
};

/** Before the first connection is made. */
export const STARTING: Connection = { status: "connecting", loaded: false, peers: [] };
