/**
 * make updates-e2e: a one-click update, for real, on this machine's Docker. It builds the
 * server, web and updater images twice, at the workspace's version and at the next patch
 * version from a copy of the checkout whose versions say so, and pushes both to a registry
 * of its own on loopback. It starts a stack of the older images with the updater profile, as
 * an install from the Compose archive runs it, with the release check pointed at a stand-in
 * for GitHub that lists the newer release; then update.spec.ts updates it from the admin
 * settings, and this script checks what the update left. The images carry no signature, so
 * the updater runs with NEON_UPDATER_VERIFY=false.
 *
 * Everything it starts is under the Compose project UPDATES_E2E_PROJECT, and it removes all of
 * it however the run ends: the stack with its volumes, the registry, the images it built and
 * the directory it worked in. CI does not run it.
 *
 * Usage, from the repository's root: bun e2e/updates/run.ts
 */
import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createServer, type Server } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { bumpVersion, copyCheckout, nextPatch, workspaceVersion } from "./release";

const HERE = import.meta.dirname;
const ROOT = resolve(HERE, "../..");
const PROJECT = process.env.UPDATES_E2E_PROJECT || "neon-updates-e2e";
const CARGO_PROFILE = process.env.E2E_CARGO_PROFILE || "e2e";

/** The registry the images go to, Distribution's, pinned. */
const REGISTRY_IMAGE =
  "registry:3.1.2@sha256:82f6ff4a7f982960c8aa51617fea3aabc485df6e55cce80d22deb1de36f1bc3c";

/** The images an update moves, by their Compose service. */
const SERVICES = ["server", "web", "updater"] as const;

/** The first admin of the stack, whom update.spec.ts signs in as. */
const ADMIN_EMAIL = "admin@updates-e2e.example";

/** Where the updater listens, inside its container. */
const UPDATER_URL = "http://127.0.0.1:8080";

function say(text: string) {
  console.log(`updates-e2e: ${text}`);
}

type Options = { cwd?: string; env?: NodeJS.ProcessEnv };

/** Runs a command with its output shown, and fails unless it succeeds. */
function run(command: string, args: string[], options: Options = {}) {
  const done = spawnSync(command, args, { stdio: "inherit", ...options });
  if (done.status !== 0) {
    throw new Error(`${command} ${args[0] ?? ""}: exit status ${done.status ?? done.signal}`);
  }
}

/** Runs a command and returns its status and what it printed on each stream. */
function capture(command: string, args: string[], options: Options = {}) {
  const done = spawnSync(command, args, { encoding: "utf8", ...options });
  return { status: done.status, stdout: done.stdout ?? "", stderr: done.stderr ?? "" };
}

/** Runs a command and returns what it printed, failing unless it succeeds. */
function output(command: string, args: string[], options: Options = {}): string {
  const done = capture(command, args, options);
  if (done.status !== 0) {
    throw new Error(`${command} ${args[0] ?? ""}: exit status ${done.status}\n${done.stderr}`);
  }
  return done.stdout.trim();
}

function check(condition: boolean, what: string) {
  if (!condition) {
    throw new Error(`check failed: ${what}`);
  }
  say(`ok: ${what}`);
}

/** `count` TCP ports free on loopback, each a different one. */
async function freePorts(count: number): Promise<number[]> {
  const servers: Server[] = [];
  const ports: number[] = [];
  for (let index = 0; index < count; index += 1) {
    const server = createServer();
    servers.push(server);
    ports.push(
      await new Promise<number>((done, failed) => {
        server.once("error", failed);
        server.listen(0, "127.0.0.1", () => {
          const address = server.address();
          if (address === null || typeof address === "string") {
            failed(new Error("the server is not on a TCP port"));
          } else {
            done(address.port);
          }
        });
      }),
    );
  }
  await Promise.all(servers.map((server) => new Promise((done) => server.close(done))));
  return ports;
}

async function waitFor(what: string, ready: () => Promise<boolean>, seconds = 60) {
  for (let tried = 0; tried < seconds; tried += 1) {
    if (await ready().catch(() => false)) {
      return;
    }
    await new Promise((done) => setTimeout(done, 1000));
  }
  throw new Error(`${what} was not ready within ${seconds} seconds`);
}

const scratch = realpathSync(mkdtempSync(join(tmpdir(), `${PROJECT}-`)));
const cleanups: (() => void)[] = [];

/** Undoes what the run did, the last first, whatever failed. */
function cleanUp() {
  for (const step of cleanups.reverse()) {
    try {
      step();
    } catch (error) {
      console.error(`updates-e2e: cleaning up: ${String(error)}`);
    }
  }
  cleanups.length = 0;
  rmSync(scratch, { recursive: true, force: true });
}

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    cleanUp();
    process.exit(130);
  });
}

async function main() {
  const from = workspaceVersion(ROOT);
  const to = nextPatch(from);
  const [registryPort, httpPort, httpsPort] = await freePorts(3);
  const registry = `127.0.0.1:${registryPort}`;
  const image = (service: string, version: string) =>
    `${registry}/lein3000zzz/neon-teamspace-${service}:${version}`;
  say(`updating ${from} to ${to}, as the Compose project ${PROJECT}, in ${scratch}`);

  // The registry, on loopback, which Docker takes plain HTTP from.
  const hadRegistryImage = capture("docker", ["image", "inspect", REGISTRY_IMAGE]).status === 0;
  cleanups.push(() => {
    capture("docker", ["container", "rm", "--force", "--volumes", `${PROJECT}-registry`]);
    if (!hadRegistryImage) {
      capture("docker", ["image", "rm", REGISTRY_IMAGE]);
    }
  });
  run("docker", [
    "run",
    "--detach",
    "--name",
    `${PROJECT}-registry`,
    "--publish",
    `${registry}:5000`,
    REGISTRY_IMAGE,
  ]);
  await waitFor("the registry", async () => (await fetch(`http://${registry}/v2/`)).ok);

  // Both releases' images, each checked to state its version, pushed, then dropped here, so
  // that the stack and the update pull them from the registry.
  cleanups.push(() => {
    for (const version of [from, to]) {
      for (const service of SERVICES) {
        capture("docker", ["image", "rm", image(service, version)]);
      }
    }
  });
  const source = join(scratch, "source");
  mkdirSync(source);
  copyCheckout(ROOT, source);
  bumpVersion(source, from, to);
  run("bun", ["scripts/version.ts", "--neon", `neon ${to}`, "--updater", `neon-updater ${to}`], {
    cwd: source,
  });
  for (const [dir, version] of [
    [ROOT, from],
    [source, to],
  ] as const) {
    say(`building the images of ${version}`);
    run(
      "docker",
      [
        "buildx",
        "bake",
        "-f",
        "deployments/docker-compose.build.yml",
        ...SERVICES.flatMap((service) => [
          "--set",
          `${service}.context=.`,
          "--set",
          `${service}.tags=${image(service, version)}`,
        ]),
        "--set",
        `server.args.CARGO_PROFILE=${CARGO_PROFILE}`,
        "--set",
        `updater.args.CARGO_PROFILE=${CARGO_PROFILE}`,
        "--load",
        ...SERVICES,
      ],
      { cwd: dir },
    );
    check(
      output("docker", ["run", "--rm", image("server", version), "--version"]) ===
        `neon ${version}`,
      `the server image of ${version} says neon ${version}`,
    );
    check(
      output("docker", ["run", "--rm", image("updater", version), "version"]) ===
        `neon-updater ${version}`,
      `the updater image of ${version} says neon-updater ${version}`,
    );
    for (const service of SERVICES) {
      run("docker", ["push", "--quiet", image(service, version)]);
      run("docker", ["image", "rm", image(service, version)]);
    }
  }
  rmSync(source, { recursive: true, force: true });

  // The stack, as an install from the Compose archive has it, with the run's overrides beside
  // it, and every variable in its env files.
  const deployments = join(scratch, "stack", "deployments");
  mkdirSync(deployments, { recursive: true });
  copyFileSync(
    join(ROOT, "deployments/docker-compose.yml"),
    join(deployments, "docker-compose.yml"),
  );
  copyFileSync(
    join(HERE, "docker-compose.yml"),
    join(deployments, "docker-compose.updates-e2e.yml"),
  );
  copyFileSync(join(HERE, "github.ts"), join(deployments, "github.ts"));
  const token = randomBytes(32).toString("hex");
  const envFile = join(deployments, ".env");
  const env = [
    "# The stack of make updates-e2e, removed once the run ends.",
    `POSTGRES_PASSWORD=${randomBytes(16).toString("hex")}`,
    `NEON_VERSION=${from}`,
    "COMPOSE_PROFILES=updater",
    `NEON_UPDATER_TOKEN=${token}`,
    "# The run's images carry no signature.",
    "NEON_UPDATER_VERIFY=false",
    `NEON_HTTP_PORT=127.0.0.1:${httpPort}`,
    `NEON_HTTPS_PORT=127.0.0.1:${httpsPort}`,
    `UPDATES_E2E_REGISTRY=${registry}`,
    `UPDATES_E2E_RELEASE=${to}`,
    "",
  ].join("\n");
  writeFileSync(envFile, env);
  const adminPassword = randomBytes(16).toString("hex");
  writeFileSync(
    join(deployments, "server.env"),
    [
      `NEON_SERVER__PUBLIC_URL=http://127.0.0.1:${httpPort}`,
      `NEON_SECRETS__KEYS=updates-e2e:${randomBytes(32).toString("base64")}`,
      `NEON_ADMIN__EMAIL=${ADMIN_EMAIL}`,
      `NEON_ADMIN__PASSWORD=${adminPassword}`,
      "",
    ].join("\n"),
  );
  const compose = [
    "compose",
    "--project-name",
    PROJECT,
    "--env-file",
    envFile,
    "-f",
    join(deployments, "docker-compose.yml"),
    "-f",
    join(deployments, "docker-compose.updates-e2e.yml"),
  ];
  let passed = false;
  cleanups.push(() => {
    if (!passed) {
      run("docker", [...compose, "logs", "--no-color", "--tail=200"]);
    }
    capture("docker", ["container", "rm", "--force", `${PROJECT}-update`]);
    capture("docker", [...compose, "down", "--volumes", "--remove-orphans"]);
  });
  say(`starting ${from} on http://127.0.0.1:${httpPort}`);
  run("docker", [...compose, "pull", "--quiet", ...SERVICES]);
  run("docker", [...compose, "up", "--detach", "--wait"]);

  /** Asks the updater at `path` with `bearer`, from inside its container. */
  const askUpdater = (bearer: string, path: string, post?: string) =>
    capture("docker", [
      ...compose,
      "exec",
      "-T",
      "updater",
      "wget",
      "-q",
      "-O",
      "-",
      "--header",
      `Authorization: Bearer ${bearer}`,
      ...(post === undefined
        ? []
        : ["--header", "Content-Type: application/json", "--post-data", post]),
      `${UPDATER_URL}${path}`,
    ]);
  const updaterStatus = () => {
    const asked = askUpdater(token, "/v1/status");
    if (asked.status !== 0) {
      throw new Error(`the updater's status: ${asked.stderr}`);
    }
    return JSON.parse(asked.stdout) as { state: string; updater_version: string };
  };

  // The updater itself refuses a wrong token, to read its status or to start an update.
  const wrong = randomBytes(32).toString("hex");
  const refusedStatus = askUpdater(wrong, "/v1/status");
  check(
    refusedStatus.status !== 0 && refusedStatus.stderr.includes("401"),
    "the updater answers a wrong token's status request with 401",
  );
  const refusedUpdate = askUpdater(wrong, "/v1/update", JSON.stringify({ version: to }));
  check(
    refusedUpdate.status !== 0 && refusedUpdate.stderr.includes("401"),
    "the updater answers a wrong token's update request with 401",
  );
  const idle = updaterStatus();
  check(
    idle.state === "idle" && idle.updater_version === from,
    `the updater, ${from}, started no update for the wrong token`,
  );

  // The update, from the admin settings.
  say("updating from the admin settings, in Chromium");
  run("node_modules/.bin/playwright", ["test", "--config", "updates/playwright.config.ts"], {
    cwd: join(ROOT, "e2e"),
    env: {
      ...process.env,
      E2E_BASE_URL: `http://127.0.0.1:${httpPort}`,
      E2E_ADMIN_EMAIL: ADMIN_EMAIL,
      E2E_ADMIN_PASSWORD: adminPassword,
      UPDATES_E2E_FROM: from,
      UPDATES_E2E_TO: to,
    },
  });

  // What the update left.
  const meta = (await (await fetch(`http://127.0.0.1:${httpPort}/api/v1/meta`)).json()) as {
    version: string;
  };
  check(meta.version === to, `GET /api/v1/meta says ${to}`);
  const running = new Map(
    output("docker", [...compose, "ps", "--format", "{{.Service}} {{.Image}} {{.State}}"])
      .split("\n")
      .map((line) => line.split(" "))
      .map(([service, ...rest]) => [service ?? "", rest] as const),
  );
  for (const service of SERVICES) {
    const [ran, state] = running.get(service) ?? [];
    check(
      ran === image(service, to) && state === "running",
      `${service} runs ${image(service, to)}`,
    );
  }
  check(
    output("docker", [...compose, "exec", "-T", "server", "neon", "--version"]) === `neon ${to}`,
    `the server says neon ${to}`,
  );
  check(
    output("docker", [...compose, "exec", "-T", "updater", "neon-updater", "version"]) ===
      `neon-updater ${to}`,
    `the updater says neon-updater ${to}`,
  );
  const after = updaterStatus();
  check(
    after.state === "idle" && after.updater_version === to,
    `the updater, now ${to}, is idle, knowing of no update before it`,
  );
  check(
    readFileSync(envFile, "utf8") ===
      env.replace(`\nNEON_VERSION=${from}\n`, `\nNEON_VERSION='${to}'\n`),
    `deployments/.env says NEON_VERSION='${to}', its other lines as they were`,
  );
  check(
    output("docker", [
      "container",
      "ls",
      "--all",
      "--quiet",
      "--filter",
      `name=^${PROJECT}-update$`,
    ]) === "",
    "the update's container is gone",
  );
  passed = true;
}

try {
  await main();
  say("passed");
} catch (error) {
  console.error(`updates-e2e: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
} finally {
  cleanUp();
}
