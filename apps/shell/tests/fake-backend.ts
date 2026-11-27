import type { Page } from "@playwright/test";

/** What a check of an address finds: a Neon Teamspace server, or a failure as Rust words it. */
export type Finding = { name: string; version: string } | { code: string; message: string };

export interface Saved {
  origin: string;
  name: string;
  version: string;
  label?: string;
}

/** The fake's starting state. */
export interface FakeConfig {
  servers: Saved[];
  last: string | null;
  /** Whether startup offers the last server to open, as the app does once as it starts. */
  open: boolean;
  /** What checking each origin finds; an origin not listed is unreachable. */
  network: Record<string, Finding>;
}

/** A command the picker sent, with its arguments. */
export interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__: unknown;
    __calls: Call[];
    __opened: string[];
  }
}

/**
 * Stands in for the commands of src-tauri/src/commands.rs, as `window.__TAURI_INTERNALS__`,
 * which `invoke` of @tauri-apps/api calls. It keeps the list in the page, normalises addresses
 * as far as the tests need, and records each call and each server it would have opened.
 */
export async function fakeBackend(page: Page, config: FakeConfig): Promise<void> {
  await page.addInitScript((initial: FakeConfig) => {
    // The app's state outlives the page, as the Rust side's does when the picker loads again.
    const KEY = "fake-backend";
    const kept = window.sessionStorage.getItem(KEY);
    const state: { servers: Saved[]; last: string | null; offered: boolean } =
      kept === null
        ? {
            servers: initial.servers.map((server) => ({ ...server })),
            last: initial.last,
            offered: !initial.open,
          }
        : JSON.parse(kept);
    const keep = () => window.sessionStorage.setItem(KEY, JSON.stringify(state));
    keep();
    window.__calls = [];
    window.__opened = [];

    // The script runs in the page, apart from this module, so its helpers live inside it.
    // oxlint-disable-next-line unicorn/consistent-function-scoping
    const fail = (code: string, message: string) => ({ code, message });
    const view = () => ({
      servers: state.servers.map((server) => ({
        ...server,
        encrypted: server.origin.startsWith("https://"),
      })),
      last: state.last,
    });
    const find = (origin: unknown) => state.servers.find((server) => server.origin === origin);
    const check = (origin: string) => {
      const finding = initial.network[origin];
      if (finding === undefined) {
        throw fail("unreachable", `Could not reach ${origin}: connection refused.`);
      }
      if ("code" in finding) {
        throw finding;
      }
      return finding;
    };
    const open = (origin: string) => {
      state.last = origin;
      window.__opened.push(origin);
    };

    const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
      startup() {
        const openNow = state.offered ? null : state.last;
        state.offered = true;
        return { ...view(), open: openNow };
      },
      connect(args) {
        const typed = String(args.address).trim();
        if (typed === "") {
          throw fail("empty", "Enter the server's address, such as neon.example.com.");
        }
        let origin: string;
        try {
          origin = new URL(typed.includes("://") ? typed : `https://${typed}`).origin;
        } catch {
          throw fail("invalid", "That is not a web address (invalid domain character).");
        }
        if (origin.startsWith("http://") && args.acceptInsecure !== true) {
          throw fail(
            "insecure",
            `${origin} is reached over plain http://, so anyone on the network between you and it can read what you send, your password included.`,
          );
        }
        const found = check(origin);
        const saved = find(origin);
        if (saved === undefined) {
          state.servers.push({ origin, ...found });
        } else {
          Object.assign(saved, found);
        }
        open(origin);
        return view();
      },
      open_server(args) {
        const saved = find(args.origin);
        if (saved === undefined) {
          throw fail("unknown_server", "That server is not in the list.");
        }
        Object.assign(saved, check(saved.origin));
        open(saved.origin);
        return view();
      },
      rename_server(args) {
        const saved = find(args.origin);
        if (saved === undefined) {
          throw fail("unknown_server", "That server is not in the list.");
        }
        const label = String(args.label).trim();
        if (label.length > 80) {
          throw fail("label", "A server's name can be at most 80 characters, with no line breaks.");
        }
        if (label === "") {
          delete saved.label;
        } else {
          saved.label = label;
        }
        return view();
      },
      remove_server(args) {
        const at = state.servers.findIndex((server) => server.origin === args.origin);
        if (at === -1) {
          throw fail("unknown_server", "That server is not in the list.");
        }
        state.servers.splice(at, 1);
        if (state.last === args.origin) {
          state.last = null;
        }
        return view();
      },
    };

    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        window.__calls.push({ cmd, args });
        const command = commands[cmd];
        if (command === undefined) {
          throw new Error(`${cmd} not allowed`);
        }
        try {
          return command(args);
        } finally {
          keep();
        }
      },
    };
  }, config);
}

/** The calls the picker made so far. */
export function calls(page: Page): Promise<Call[]> {
  return page.evaluate(() => window.__calls);
}

/** The servers the picker had the app open so far. */
export function opened(page: Page): Promise<string[]> {
  return page.evaluate(() => window.__opened);
}
