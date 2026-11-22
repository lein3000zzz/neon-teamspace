import type { WebSocketData } from "msw";
import * as Y from "yjs";
import { server, syncLink } from "./server";

/** y-websocket's message types, y-protocols' sync steps, and its answers to auth. */
const MESSAGE_SYNC = 0;
const MESSAGE_AWARENESS = 1;
const MESSAGE_AUTH = 2;
const PERMISSION_DENIED = 0;
const PERMISSION_GRANTED = 1;
const STEP_1 = 0;
const STEP_2 = 1;
const UPDATE = 2;

/** An unsigned LEB128 number, as lib0 writes them. */
function varUint(value: number): number[] {
  const bytes: number[] = [];
  let rest = value;
  while (rest > 0x7f) {
    bytes.push((rest & 0x7f) | 0x80);
    rest >>>= 7;
  }
  bytes.push(rest);
  return bytes;
}

function readVarUint(data: Uint8Array, start: number): [number, number] {
  let value = 0;
  let shift = 0;
  let at = start;
  for (;;) {
    const byte = data[at] ?? 0;
    at += 1;
    value += (byte & 0x7f) * 2 ** shift;
    if (byte < 0x80) {
      return [value, at];
    }
    shift += 7;
  }
}

function syncMessage(step: number, payload: Uint8Array): Uint8Array<ArrayBuffer> {
  return new Uint8Array([
    ...varUint(MESSAGE_SYNC),
    ...varUint(step),
    ...varUint(payload.length),
    ...payload,
  ]);
}

/** An awareness message, as y-websocket sends one: someone's presence. */
export function awarenessMessage(update: Uint8Array): Uint8Array<ArrayBuffer> {
  return new Uint8Array([...varUint(MESSAGE_AWARENESS), ...varUint(update.length), ...update]);
}

/**
 * An auth message, as the server sends one when the user's access changes:
 * permission denied with its reason, or, without one, permission granted.
 */
export function authMessage(denied?: string): Uint8Array<ArrayBuffer> {
  if (denied === undefined) {
    return new Uint8Array([...varUint(MESSAGE_AUTH), ...varUint(PERMISSION_GRANTED)]);
  }
  const reason = new TextEncoder().encode(denied);
  return new Uint8Array([
    ...varUint(MESSAGE_AUTH),
    ...varUint(PERMISSION_DENIED),
    ...varUint(reason.length),
    ...reason,
  ]);
}

function bytesOf(data: WebSocketData): Uint8Array | undefined {
  if (data instanceof ArrayBuffer) {
    return new Uint8Array(data);
  }
  if (ArrayBuffer.isView(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return undefined;
}

type FakeSyncOptions = {
  /** False for a server that takes the connection and never answers. */
  answers?: boolean;
};

/**
 * The server's end of the live connection, speaking the y-websocket protocol
 * as the real one does: it sends its state vector as the connection opens,
 * answers a client's with what the client lacks, keeps every update, and
 * passes updates and awareness on to the page's other connections.
 */
export function fakeSync({ answers = true }: FakeSyncOptions = {}) {
  const doc = new Y.Doc();
  const urls: string[] = [];
  // What a test writes into the server's copy goes out as others' edits would.
  const fromClient = Symbol("client");
  doc.on("update", (update: Uint8Array, origin: unknown) => {
    if (origin !== fromClient) {
      syncLink.broadcast(syncMessage(UPDATE, update));
    }
  });
  server.use(
    syncLink.addEventListener("connection", ({ client }) => {
      urls.push(client.url.href);
      if (!answers) {
        return;
      }
      client.addEventListener("message", (event) => {
        const data = bytesOf(event.data);
        if (data === undefined) {
          return;
        }
        const [type, afterType] = readVarUint(data, 0);
        if (type === MESSAGE_AWARENESS) {
          syncLink.broadcastExcept(client, data.slice());
          return;
        }
        if (type !== MESSAGE_SYNC) {
          return;
        }
        const [step, afterStep] = readVarUint(data, afterType);
        const [length, start] = readVarUint(data, afterStep);
        const payload = data.slice(start, start + length);
        if (step === STEP_1) {
          client.send(syncMessage(STEP_2, Y.encodeStateAsUpdate(doc, payload)));
        } else if (step === STEP_2 || step === UPDATE) {
          Y.applyUpdate(doc, payload, fromClient);
          syncLink.broadcastExcept(client, syncMessage(UPDATE, payload));
        }
      });
      client.send(syncMessage(STEP_1, Y.encodeStateVector(doc)));
    }),
  );
  return {
    doc,
    urls,
    /** Ends every connection with a close code, as the server does. */
    close(code: number, reason = "") {
      for (const client of syncLink.clients) {
        client.close(code, reason);
      }
    },
    /** Sends a message to every connection, as the server does to tell them something. */
    send(message: Uint8Array<ArrayBuffer>) {
      syncLink.broadcast(message);
    },
  };
}
