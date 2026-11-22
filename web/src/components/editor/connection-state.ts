import type { Collaborator } from "./cursor";

/**
 * How the page's live connection stands:
 * - `connecting`: the first connection, until the server has sent the document;
 * - `connected`: in sync, changes go both ways as they are made;
 * - `reconnecting`: the connection dropped, or the server asked for a pause;
 *   it is tried again, with changes made meanwhile kept here and sent once it
 *   is back;
 * - `closed`: the server ended it for a reason trying again does not cure,
 *   `closedBy` says which; it is not tried again by itself.
 */
export type ConnectionStatus = "connecting" | "connected" | "reconnecting" | "closed";

/**
 * Why the server ended the connection for good:
 * - `signedOut`: the session ended (4401);
 * - `accessLost`: the user may no longer see the page (4404);
 * - `readOnly`: a change came while the user may only view the page (4403);
 * - `tooLarge`: a change larger than the server takes at once, or that would
 *   make the page larger than it keeps (1009, 4413);
 * - `failed`: anything else that sending the same again would not cure, such
 *   as a message the server could not read.
 *
 * Changes the server did not take stay out of the page: the next connection
 * starts again from what the server holds.
 */
export type ClosedBy = "signedOut" | "accessLost" | "readOnly" | "tooLarge" | "failed";

export type Peer = Omit<Collaborator, "color">;

export type Connection = {
  status: ConnectionStatus;
  /** Set while the status is `closed`. */
  closedBy: ClosedBy | null;
  /** Whether the document came from the server at least once, so its content is the page's. */
  loaded: boolean;
  /**
   * What the server said the connection may do since it opened, if it said
   * anything: it turns a connection read-only, and back, as the user's access
   * changes, without closing it.
   */
  access: "edit" | "view" | null;
  /** The others with the page open, each once however many tabs they have. */
  peers: Peer[];
};

/** Before the first connection is made. */
export const STARTING: Connection = {
  status: "connecting",
  closedBy: null,
  loaded: false,
  access: null,
  peers: [],
};
