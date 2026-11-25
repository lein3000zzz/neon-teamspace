import { useQueryClient, type QueryClient, type QueryFilters } from "@tanstack/react-query";
import { useCallback, useMemo, useSyncExternalStore } from "react";
import {
  addRow,
  dropRow,
  patchRow,
  refreshRows,
  type Database,
  type Row,
  type Values,
} from "./databases";
import { keys } from "./keys";
import type { PageAccess } from "./pages";

/**
 * How a database's live feed stands:
 * - `connecting`: the first connection, until the server said it is ready;
 * - `connected`: every change to the database reaches this tab as it is saved;
 * - `reconnecting`: the connection dropped, or the server asked for a pause; it
 *   is tried again, and everything shown is read afresh once it is back;
 * - `closed`: the server ended it for a reason trying again does not cure.
 */
export type FeedStatus = "connecting" | "connected" | "reconnecting" | "closed";

/**
 * Why the server ended the feed for good: the session ended (4401), or the
 * user may no longer see the database (4404).
 */
export type FeedClosedBy = "signedOut" | "accessLost";

export type FeedState = {
  status: FeedStatus;
  closedBy: FeedClosedBy | null;
};

const STARTING: FeedState = { status: "connecting", closedBy: null };

/** A row as a feed message carries it: without what the caller may do, or its group. */
type FedRow = Omit<Row, "access" | "group">;

type RowChange = {
  row_id: string;
  values: Values;
  title?: string;
  last_edited_at: string;
  last_edited_by?: string | null;
};

/** The messages of `/ws/databases/{id}`, each with the database's id. */
export type FeedEvent = { database_id: string } & (
  | { type: "ready" | "access"; access: PageAccess }
  | { type: "row_created" | "row_restored"; row: FedRow }
  | ({ type: "row_updated" } & RowChange)
  | ({ type: "row_moved"; position: string } & RowChange)
  | { type: "row_trashed"; row_id: string }
  | { type: "schema_changed" }
  | { type: "view_changed"; view_id: string; deleted: boolean }
);

/**
 * Where a database's changes are told: `/ws/databases/{id}` on this origin,
 * over wss: when the page came over https:. The session cookie goes with the
 * upgrade, as with any request to the origin.
 */
export function feedServer(location: Pick<Location, "protocol" | "host">): string {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${location.host}/ws/databases`;
}

/**
 * What follows the server closing the feed with `code`, as the server's docs
 * ask of a feed: stopping once the session ended (4401) or the database is
 * no longer seen (4404); trying `later`, after a longer pause, when the
 * client fell behind (4408), sent too much (4429) or there are too many
 * connections (1013); and trying `again` soon after anything else, such as
 * a restart (1012), a message too large (1009) or a dropped connection. A
 * feed sends nothing, so nothing it sent is refused again.
 */
export function afterFeedClose(code: number): "again" | "later" | FeedClosedBy {
  switch (code) {
    case 4401:
      return "signedOut";
    case 4404:
      return "accessLost";
    case 1013:
    case 4408:
    case 4429:
      return "later";
    default:
      return "again";
  }
}

/** The first pause before trying again, doubled each time up to the longest. */
const FIRST_PAUSE_MS = 1000;
const LONGEST_PAUSE_MS = 30 * 1000;
/** How long a feed stays up before the pauses start again from the first. */
const STEADY_MS = 30 * 1000;
/** The pause before the first try again, so a server that drops at once is not flooded. */
const AGAIN_MS = 250;

/**
 * Applies a change the feed told of to what the cache holds: the row's new
 * values shown at once in every view and on its page, then every view's rows
 * read again shortly, as the server filters, sorts and groups them. A change
 * to the schema or a view reads those again.
 */
export function applyFeedEvent(
  queryClient: QueryClient,
  workspaceId: string,
  databaseId: string,
  event: FeedEvent,
) {
  switch (event.type) {
    case "row_created":
    case "row_restored": {
      const database = queryClient.getQueryData<Database>(keys.database(workspaceId, databaseId));
      if (database !== undefined) {
        addRow(queryClient, workspaceId, database, { ...event.row, access: database.page.access });
      }
      void queryClient.invalidateQueries({ queryKey: keys.row(workspaceId, event.row.id) });
      refreshRows(queryClient, workspaceId, databaseId);
      return;
    }
    case "row_updated":
    case "row_moved": {
      patchRow(queryClient, workspaceId, databaseId, event.row_id, (row) => changed(row, event));
      if (event.title !== undefined) {
        const title = event.title;
        queryClient.setQueryData<{ title: string }>(
          keys.page(workspaceId, event.row_id),
          (page) => page && { ...page, title },
        );
      }
      refreshRows(queryClient, workspaceId, databaseId);
      return;
    }
    case "row_trashed":
      dropRow(queryClient, workspaceId, databaseId, event.row_id);
      void queryClient.invalidateQueries({ queryKey: keys.row(workspaceId, event.row_id) });
      void queryClient.invalidateQueries({ queryKey: keys.page(workspaceId, event.row_id) });
      refreshRows(queryClient, workspaceId, databaseId);
      return;
    case "schema_changed":
      void queryClient.invalidateQueries({ queryKey: keys.database(workspaceId, databaseId) });
      void queryClient.invalidateQueries(rowsOf(workspaceId, databaseId));
      return;
    case "view_changed":
      if (event.deleted) {
        queryClient.removeQueries({
          queryKey: keys.viewRows(workspaceId, databaseId, event.view_id),
        });
      } else {
        void queryClient.invalidateQueries({
          queryKey: keys.viewRows(workspaceId, databaseId, event.view_id),
        });
      }
      void queryClient.invalidateQueries({
        queryKey: keys.database(workspaceId, databaseId),
        exact: true,
      });
      return;
    case "ready":
    case "access":
      return;
  }
}

/** A row with a change the feed told of written over it. */
function changed(row: Row, change: RowChange & { position?: string }): Row {
  const values: Values = { ...row.values };
  for (const [id, value] of Object.entries(change.values)) {
    if (value === null) {
      delete values[id];
    } else {
      values[id] = value;
    }
  }
  return {
    ...row,
    values,
    last_edited_at: change.last_edited_at,
    ...(change.last_edited_by === undefined ? {} : { last_edited_by: change.last_edited_by }),
    ...(change.title === undefined ? {} : { title: change.title }),
    ...(change.position === undefined ? {} : { position: change.position }),
  };
}

/** The rows of one database read on their own, as their pages show them. */
function rowsOf(workspaceId: string, databaseId: string): QueryFilters {
  return {
    queryKey: keys.rows(workspaceId),
    predicate: (query) => (query.state.data as Row | undefined)?.database_id === databaseId,
  };
}

/**
 * Everything a feed tells about, read afresh: as it connects, and whenever it
 * reconnects. A read already on its way may have begun before the feed heard
 * every change, so it is begun again rather than waited for.
 */
async function readAfresh(queryClient: QueryClient, workspaceId: string, databaseId: string) {
  const filters: QueryFilters[] = [
    { queryKey: keys.database(workspaceId, databaseId) },
    { queryKey: keys.page(workspaceId, databaseId), exact: true },
    rowsOf(workspaceId, databaseId),
  ];
  await Promise.all(filters.map((filter) => queryClient.cancelQueries(filter)));
  await Promise.all(filters.map((filter) => queryClient.invalidateQueries(filter)));
}

function isFeedEvent(value: unknown, databaseId: string): value is FeedEvent {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    typeof value.type === "string" &&
    "database_id" in value &&
    value.database_id === databaseId
  );
}

/**
 * One database's live feed, shared by everything that shows the database in
 * this tab: a full page, an inline block, a row's page. It opens with the
 * first listener and closes once the last is gone, a moment later, so a view
 * that remounts at once keeps it.
 */
class Feed {
  #state: FeedState = STARTING;
  readonly #listeners = new Set<() => void>();
  #socket: WebSocket | null = null;
  #pauses = 0;
  #retry: ReturnType<typeof setTimeout> | undefined;
  #steady: ReturnType<typeof setTimeout> | undefined;
  #stop: ReturnType<typeof setTimeout> | undefined;
  readonly #queryClient: QueryClient;
  readonly #workspaceId: string;
  readonly #databaseId: string;

  constructor(queryClient: QueryClient, workspaceId: string, databaseId: string) {
    this.#queryClient = queryClient;
    this.#workspaceId = workspaceId;
    this.#databaseId = databaseId;
  }

  get state(): FeedState {
    return this.#state;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    clearTimeout(this.#stop);
    if (this.#socket === null && this.#retry === undefined && this.#state.status !== "closed") {
      this.#connect();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#stop = setTimeout(() => this.#idle(), 0);
      }
    };
  }

  /**
   * Nothing shows the database any more: the feed closes, and opens afresh for
   * the next to show it, even after the server ended it for good, as what
   * ended it may have changed meanwhile.
   */
  #idle() {
    this.#close();
    this.#pauses = 0;
    this.#state = STARTING;
  }

  /** Opens the feed afresh after the server ended it for good. */
  restart() {
    this.#close();
    this.#publish(STARTING);
    this.#connect();
  }

  #publish(patch: Partial<FeedState>) {
    this.#state = { ...this.#state, ...patch };
    for (const listener of this.#listeners) {
      listener();
    }
  }

  #connect() {
    clearTimeout(this.#retry);
    this.#retry = undefined;
    const socket = new WebSocket(`${feedServer(window.location)}/${this.#databaseId}`);
    this.#socket = socket;
    socket.addEventListener("message", (message) => {
      if (this.#socket === socket && typeof message.data === "string") {
        this.#receive(message.data);
      }
    });
    socket.addEventListener("close", (event) => {
      if (this.#socket === socket) {
        this.#closed(event.code);
      }
    });
  }

  #receive(text: string) {
    let event: unknown;
    try {
      event = JSON.parse(text);
    } catch {
      return;
    }
    if (!isFeedEvent(event, this.#databaseId)) {
      return;
    }
    if (event.type === "ready") {
      this.#publish({ status: "connected", closedBy: null });
      void readAfresh(this.#queryClient, this.#workspaceId, this.#databaseId);
      clearTimeout(this.#steady);
      this.#steady = setTimeout(() => {
        this.#pauses = 0;
      }, STEADY_MS);
    } else if (event.type === "access") {
      // What the user may do changed: the database's page, and so its views, say it anew.
      void readAfresh(this.#queryClient, this.#workspaceId, this.#databaseId);
    } else {
      applyFeedEvent(this.#queryClient, this.#workspaceId, this.#databaseId, event);
    }
  }

  #closed(code: number) {
    this.#socket = null;
    clearTimeout(this.#steady);
    const next = afterFeedClose(code);
    if (next === "again" || next === "later") {
      const pause =
        next === "again" && this.#pauses === 0
          ? AGAIN_MS
          : Math.min(FIRST_PAUSE_MS * 2 ** this.#pauses, LONGEST_PAUSE_MS);
      this.#pauses += 1;
      this.#publish({ status: "reconnecting", closedBy: null });
      this.#retry = setTimeout(() => this.#connect(), pause);
      return;
    }
    this.#publish({ status: "closed", closedBy: next });
    if (next === "signedOut") {
      void this.#queryClient.invalidateQueries({ queryKey: keys.me });
    } else {
      void readAfresh(this.#queryClient, this.#workspaceId, this.#databaseId);
    }
  }

  #close() {
    clearTimeout(this.#retry);
    clearTimeout(this.#steady);
    clearTimeout(this.#stop);
    this.#retry = undefined;
    const socket = this.#socket;
    this.#socket = null;
    // Closing one still connecting makes the browser log an error: it closes once open.
    if (socket?.readyState === WebSocket.CONNECTING) {
      socket.addEventListener("open", () => socket.close());
    } else {
      socket?.close();
    }
  }
}

const feeds = new WeakMap<QueryClient, Map<string, Feed>>();

function feedOf(queryClient: QueryClient, workspaceId: string, databaseId: string): Feed {
  let mine = feeds.get(queryClient);
  if (mine === undefined) {
    mine = new Map();
    feeds.set(queryClient, mine);
  }
  const key = `${workspaceId}:${databaseId}`;
  let feed = mine.get(key);
  if (feed === undefined) {
    feed = new Feed(queryClient, workspaceId, databaseId);
    mine.set(key, feed);
  }
  return feed;
}

/**
 * Follows a database's changes live while the component is shown, applying
 * each to the cache, and says how the feed stands; `restart` opens it afresh
 * once the server ended it. Nothing is followed while `databaseId` is
 * undefined, as for a row whose database the user does not see.
 */
export function useDatabaseFeed(workspaceId: string, databaseId: string | undefined) {
  const queryClient = useQueryClient();
  const feed = useMemo(
    () => (databaseId === undefined ? null : feedOf(queryClient, workspaceId, databaseId)),
    [queryClient, workspaceId, databaseId],
  );
  const subscribe = useCallback(
    (listener: () => void) => feed?.subscribe(listener) ?? (() => {}),
    [feed],
  );
  const state = useSyncExternalStore(subscribe, () => feed?.state ?? STARTING);
  const restart = useCallback(() => feed?.restart(), [feed]);
  return { state, restart };
}
