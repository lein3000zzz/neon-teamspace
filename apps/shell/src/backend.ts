import { invoke } from "@tauri-apps/api/core";

/** A saved server, as the app's Rust side lists it. */
export interface SavedServer {
  /** Its origin, such as `https://neon.example.com`: the list's key. */
  origin: string;
  /** The name someone gave it, if any. */
  label?: string;
  /** The instance's name, as its last check read it. */
  name: string;
  /** The server's version, as its last check read it. */
  version: string;
  /** Whether it is reached over https. */
  encrypted: boolean;
}

export interface ServerList {
  servers: SavedServer[];
  /** The origin of the server opened last. */
  last: string | null;
}

/** Why the window came back to the picker from a server it was opening. */
export interface Interrupted {
  origin: string;
  /** `cancelled`, or `page_timed_out` when its page did not start to arrive in time. */
  code: string;
  message: string;
}

export interface Startup extends ServerList {
  /** The server to open at once, as the app starts. */
  open: string | null;
  interrupted: Interrupted | null;
  /** Whether the picker offers a check for updates: on Android, where the app has no menu. */
  updateCheck: boolean;
}

/** What a check for updates found; a dialog of the app's own offers a newer release. */
export type UpdateCheck =
  { state: "current"; version: string } | { state: "available"; version: string; current: string };

/** Why a command failed: a stable code, and a sentence to show. */
export interface Failure {
  code: string;
  message: string;
}

/** The commands of src-tauri/src/commands.rs. */
export const backend = {
  startup: () => invoke<Startup>("startup"),
  /** Checks, saves and opens a new address; `insecure` until `acceptInsecure` for http. */
  connect: (address: string, acceptInsecure: boolean) =>
    invoke<ServerList>("connect", { address, acceptInsecure }),
  openServer: (origin: string) => invoke<ServerList>("open_server", { origin }),
  renameServer: (origin: string, label: string) =>
    invoke<ServerList>("rename_server", { origin, label }),
  removeServer: (origin: string) => invoke<ServerList>("remove_server", { origin }),
  /** Stops opening a server: true when the window goes back to the picker to stop its page. */
  cancelOpening: () => invoke<boolean>("cancel_opening"),
  /** Checks for a newer release of the app. */
  checkForUpdates: () => invoke<UpdateCheck>("check_for_updates"),
};

/** What a rejected command said, as a {@link Failure} whatever it was. */
export function failure(error: unknown): Failure {
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "message" in error &&
    typeof error.code === "string" &&
    typeof error.message === "string"
  ) {
    return { code: error.code, message: error.message };
  }
  return { code: "internal", message: `Something went wrong in the app: ${String(error)}` };
}

/** The name a server shows: the one someone gave it, or its instance's. */
export function displayName(server: SavedServer): string {
  return server.label ?? server.name;
}
