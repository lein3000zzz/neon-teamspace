/**
 * A webhook receiver for the end-to-end stack: the server delivers to it on the stack's
 * network, and the tests read what it got. It keeps every POST it gets, its path, headers and
 * body as sent, answers 200, and lists them at GET /deliveries. GET /healthz answers 200 for
 * the container's health check. It runs on Bun, from the stack's Compose file.
 *
 * It also stands in for the two services the server's updates ask, so that the stack's
 * release check never leaves the machine:
 *
 * - GitHub's API, under /github: the repository's releases, made up, at a version no real
 *   release will reach, with a pre-release and a draft beside it;
 * - the updater, under /updater: it takes UPDATER_TOKEN, starts an update when asked, and
 *   says a few seconds later that the update failed, as there is no stack for it to update.
 *   The update requests it gets are kept as deliveries too.
 */
import { createServer, type ServerResponse } from "node:http";

type Delivery = { path: string; headers: Record<string, string>; body: string };

/** The most deliveries it keeps, the oldest dropped first. */
const KEPT = 500;

/** The token the stand-in updater takes, obviously not a secret; tests/updates.spec.ts has it too. */
const UPDATER_TOKEN = "e2e-updater-token-not-a-secret-at-all";

/** How long an update runs before the stand-in updater says it failed. */
const UPDATE_RUNS_MS = 4_000;

const RELEASES_PATH = "/github/repos/lein3000zzz/neon-teamspace/releases";

/** A release as GitHub lists it, with the fields the server reads. */
function release(tag: string, prerelease: boolean, draft = false) {
  return {
    tag_name: tag,
    name: `Neon Teamspace ${tag}`,
    draft,
    prerelease,
    html_url: `https://github.com/lein3000zzz/neon-teamspace/releases/tag/${tag}`,
    published_at: draft ? null : "2026-11-27T12:00:00Z",
  };
}

const RELEASES = [
  release("v99.1.0-rc.1", true),
  release("v99.0.0", false),
  release("v100.0.0", false, true),
];

type UpdaterStatus = {
  state: "idle" | "updating" | "done" | "failed";
  version?: string;
  error?: string;
  log?: string;
  started_at?: string;
  finished_at?: string;
  updater_version: string;
};

let updater: UpdaterStatus = { state: "idle", updater_version: "0.0.0-e2e" };

const deliveries: Delivery[] = [];

function json(response: ServerResponse, status: number, body: unknown) {
  response.writeHead(status, { "Content-Type": "application/json" }).end(JSON.stringify(body));
}

function refuse(response: ServerResponse, status: number, code: string, message: string) {
  json(response, status, { error: { code, message } });
}

/** Answers as the updater does, once the token is right. */
function answerAsUpdater(
  response: ServerResponse,
  method: string,
  path: string,
  authorization: string | undefined,
  body: string,
) {
  if (authorization !== `Bearer ${UPDATER_TOKEN}`) {
    refuse(response, 401, "unauthenticated", "a missing or wrong token");
    return;
  }
  if (method === "GET" && path === "/updater/v1/status") {
    json(response, 200, updater);
    return;
  }
  if (method !== "POST" || path !== "/updater/v1/update") {
    refuse(response, 404, "not_found", "no such route");
    return;
  }
  if (updater.state === "updating") {
    refuse(response, 409, "update_in_progress", "an update is under way already");
    return;
  }
  let version: unknown;
  try {
    version = (JSON.parse(body) as { version?: unknown }).version;
  } catch {
    version = undefined;
  }
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
    refuse(response, 400, "invalid_version", "want a release's version, such as 0.2.1");
    return;
  }
  const log = `Pulling server, web at ${version}\n`;
  updater = {
    state: "updating",
    version,
    log,
    started_at: new Date().toISOString(),
    updater_version: updater.updater_version,
  };
  setTimeout(() => {
    updater = {
      ...updater,
      state: "failed",
      error: "the end-to-end tests' receiver stands in for the updater, and updates nothing",
      log: `${log}No stack to update here\n`,
      finished_at: new Date().toISOString(),
    };
  }, UPDATE_RUNS_MS);
  json(response, 202, updater);
}

const server = createServer((request, response) => {
  const method = request.method ?? "GET";
  const path = new URL(request.url ?? "/", "http://receiver.invalid").pathname;
  if (method === "GET" && path === "/healthz") {
    response.writeHead(200).end("ok");
    return;
  }
  if (method === "GET" && path === "/deliveries") {
    json(response, 200, deliveries);
    return;
  }
  if (method === "GET" && path === RELEASES_PATH) {
    json(response, 200, RELEASES);
    return;
  }
  if (method === "GET" && path.startsWith("/updater/")) {
    answerAsUpdater(response, method, path, request.headers.authorization, "");
    return;
  }
  if (method !== "POST") {
    response.writeHead(405).end();
    return;
  }
  const chunks: Buffer[] = [];
  request.on("data", (chunk: Buffer) => chunks.push(chunk));
  request.on("end", () => {
    const headers: Record<string, string> = {};
    for (const [name, value] of Object.entries(request.headers)) {
      headers[name] = Array.isArray(value) ? value.join(", ") : (value ?? "");
    }
    const body = Buffer.concat(chunks).toString("utf8");
    deliveries.push({ path, headers, body });
    if (deliveries.length > KEPT) {
      deliveries.shift();
    }
    if (path.startsWith("/updater/")) {
      answerAsUpdater(response, method, path, request.headers.authorization, body);
      return;
    }
    response.writeHead(200).end("ok");
  });
});

server.listen(8080, "0.0.0.0");
