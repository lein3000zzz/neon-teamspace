/**
 * A webhook receiver for the end-to-end stack: the server delivers to it on the stack's
 * network, and the tests read what it got. It keeps every POST it gets, its path, headers and
 * body as sent, answers 200, and lists them at GET /deliveries. GET /healthz answers 200 for
 * the container's health check. It runs on Bun, from the stack's Compose file.
 */
import { createServer } from "node:http";

type Delivery = { path: string; headers: Record<string, string>; body: string };

/** The most deliveries it keeps, the oldest dropped first. */
const KEPT = 500;

const deliveries: Delivery[] = [];

const server = createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://receiver.invalid").pathname;
  if (request.method === "GET" && path === "/healthz") {
    response.writeHead(200).end("ok");
    return;
  }
  if (request.method === "GET" && path === "/deliveries") {
    response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(deliveries));
    return;
  }
  if (request.method !== "POST") {
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
    deliveries.push({ path, headers, body: Buffer.concat(chunks).toString("utf8") });
    if (deliveries.length > KEPT) {
      deliveries.shift();
    }
    response.writeHead(200).end("ok");
  });
});

server.listen(8080, "0.0.0.0");
