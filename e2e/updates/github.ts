/**
 * GitHub's API for make updates-e2e, as far as the server's release check reads it: the
 * repository's releases, of which there is one, the release UPDATES_E2E_RELEASE names, which
 * the run built and pushed to its registry. GET /healthz answers 200 for the container's
 * health check. It runs on Bun, from the run's Compose file.
 */
import { createServer } from "node:http";

const release = process.env.UPDATES_E2E_RELEASE ?? "";
if (!/^\d+\.\d+\.\d+$/.test(release)) {
  throw new Error(`UPDATES_E2E_RELEASE is ${JSON.stringify(release)}, not a release's version`);
}

const RELEASES_PATH = "/repos/lein3000zzz/neon-teamspace/releases";

/** The release as GitHub lists it, with the fields the server reads. */
const RELEASES = [
  {
    tag_name: `v${release}`,
    name: `Neon Teamspace v${release}`,
    draft: false,
    prerelease: false,
    html_url: `https://github.com/lein3000zzz/neon-teamspace/releases/tag/v${release}`,
    published_at: "2026-11-28T12:00:00Z",
  },
];

const server = createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://github.invalid").pathname;
  if (request.method === "GET" && path === "/healthz") {
    response.writeHead(200).end("ok");
    return;
  }
  if (request.method === "GET" && path === RELEASES_PATH) {
    response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(RELEASES));
    return;
  }
  response.writeHead(404, { "Content-Type": "application/json" }).end('{"message":"Not Found"}');
});

server.listen(8080, "0.0.0.0");
