/**
 * Runs the built desktop app in its real webview against a hostile server, on Linux under a
 * display (xvfb-run in CI), where WebKitGTK names the page an IPC message comes from by the
 * URL the window is loading. The server's page calls the picker's commands without pause, among
 * them the check for updates, and the updater's own, and meanwhile sends the window to the app's
 * own origin by a redirect. The check passes once the app has shown the picker in its own way,
 * through a blank page, with every call refused and the server list as it was; it fails when a
 * call went through, or the picker never came back.
 *
 *   bun scripts/webview-check.ts <the app's binary>
 */

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

const TIMEOUT_MS = 60_000;
const VICTIM = "https://victim.example";
/** The picker, as Tauri serves the app's own pages on Linux. */
const PICKER = "tauri://localhost/index.html";

const binary = process.argv[2];
if (binary === undefined) {
  console.error("usage: bun scripts/webview-check.ts <the app's binary>");
  process.exit(2);
}
const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")) as {
  identifier: string;
};

/** What the hostile page reported, and what the app logged. */
const reports: string[] = [];
let log = "";

/** The commands the page calls, each of which it must be refused. */
const COMMANDS = ["rename_server", "check_for_updates", "plugin:updater|check"];

/**
 * The page: calls each command without pause, and after a second goes to /slow. The app refuses
 * a call with Tauri's "not allowed", or with the code not_the_picker; any other answer means
 * the command ran, such as a check for updates that could not reach GitHub.
 */
const page = `<!doctype html><title>Hostile</title>
<script>
const report = (what) => fetch("/report?" + new URLSearchParams({ what }));
let n = 0;
const refused = new Set();
async function call(command, args) {
  try {
    await window.__TAURI_INTERNALS__.invoke(command, args);
    report("taken " + command + " " + n);
  } catch (error) {
    const text = typeof error === "string" ? error : JSON.stringify(error);
    if (!text.includes("not allowed") && !text.includes('"not_the_picker"')) {
      report("taken " + command + ", which failed with " + text.slice(0, 80));
    } else if (!refused.has(command)) {
      refused.add(command);
      report("refused " + command + ": " + text.slice(0, 80));
    }
  }
}
setInterval(() => {
  for (let i = 0; i < 20; i += 1) {
    n += 1;
    call("rename_server", { origin: "${VICTIM}", label: "taken-" + n });
  }
  call("check_for_updates", {});
  call("plugin:updater|check", {});
}, 5);
setTimeout(() => { location.href = "/slow"; }, 1000);
</script>`;

const server = createServer((request, response) => {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");
  if (url.pathname === "/api/v1/meta") {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ product: "neon-teamspace", name: "Hostile", version: "0.1.0" }));
  } else if (url.pathname === "/report") {
    reports.push(url.searchParams.get("what") ?? "");
    response.writeHead(204).end();
  } else if (url.pathname === "/slow") {
    // The page keeps calling while this answers.
    setTimeout(() => response.writeHead(302, { location: PICKER }).end(), 500);
  } else {
    response.writeHead(200, {
      "content-type": "text/html",
      // As Caddy's policy: no fetch to the IPC protocol, so messages go by postMessage.
      "content-security-policy":
        "default-src 'self'; script-src 'self' 'unsafe-inline'; connect-src 'self'",
    });
    response.end(page);
  }
});
await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
const address = server.address();
if (address === null || typeof address === "string") {
  throw new Error("the hostile server has no port");
}
const hostile = `http://127.0.0.1:${address.port}`;

const home = mkdtempSync(join(tmpdir(), "neon-webview-check-"));
const dataDir = join(home, "data", config.identifier);
mkdirSync(dataDir, { recursive: true });
const listFile = join(dataDir, "servers.json");
writeFileSync(
  listFile,
  JSON.stringify({
    format: 1,
    servers: [
      { origin: VICTIM, label: "Work", name: "Victim", version: "0.1.0" },
      { origin: hostile, name: "Hostile", version: "0.1.0" },
    ],
    last: hostile,
  }),
);

const app = spawn(binary, [], {
  env: {
    ...process.env,
    HOME: home,
    XDG_DATA_HOME: join(home, "data"),
    XDG_CONFIG_HOME: join(home, "config"),
    XDG_CACHE_HOME: join(home, "cache"),
  },
  stdio: ["ignore", "pipe", "pipe"],
});
const done = new Promise<void>((resolve, reject) => {
  const timer = setTimeout(
    () => reject(new Error(`the picker did not come back within ${TIMEOUT_MS / 1000} s`)),
    TIMEOUT_MS,
  );
  const read = (chunk: Buffer) => {
    log += chunk.toString();
    // The app's own way back, after the server's page asked for the picker: the blank page,
    // then the picker started once more, with the server's page gone.
    const shown = log.indexOf("refused, and the picker shown in its place");
    if (shown !== -1 && log.slice(shown).includes("page started on tauri://localhost")) {
      clearTimeout(timer);
      resolve();
    }
  };
  app.stdout.on("data", read);
  app.stderr.on("data", read);
  app.on("exit", (code) => {
    clearTimeout(timer);
    reject(new Error(`the app exited with ${code}`));
  });
  app.on("error", (error) => {
    clearTimeout(timer);
    reject(new Error(`the app did not start: ${error.message}`));
  });
});

let failure = await done.then(
  () => null,
  (error: unknown) => (error instanceof Error ? error.message : String(error)),
);
app.removeAllListeners("exit");
app.kill();
server.closeAllConnections();
server.close();

// A call that went through says more than how the run ended.
const list = JSON.parse(readFileSync(listFile, "utf8")) as {
  servers: { origin: string; label?: string }[];
};
rmSync(home, { recursive: true, force: true });
const victim = list.servers.find((saved) => saved.origin === VICTIM);
const taken = reports.filter((report) => report.startsWith("taken"));
if (taken.length > 0) {
  failure = `${taken.length} of the page's calls went through, the last ${taken.at(-1)}`;
} else if (victim?.label !== "Work") {
  failure = `the server list changed: ${JSON.stringify(victim)}`;
} else if (failure === null) {
  const unproven = COMMANDS.filter(
    (command) => !reports.some((report) => report.startsWith(`refused ${command}:`)),
  );
  if (unproven.length > 0) {
    failure = `the page reported no refused call of ${unproven.join(", ")}, so the check proved nothing`;
  }
}

if (failure !== null) {
  console.error(`webview check failed: ${failure}\n--- the app's log\n${log}`);
  console.error(`--- the page's reports\n${reports.join("\n")}`);
  process.exit(1);
}
console.log(
  `webview check passed: the server's page reached none of the picker's commands (${COMMANDS.join(", ")})`,
);
