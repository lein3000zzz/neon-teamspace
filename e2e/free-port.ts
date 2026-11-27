/**
 * Prints a TCP port free on this machine's loopback, for make e2e to put the stack's web
 * origin on. The server must know its public URL before it starts, as its mail links there,
 * so the port is picked here rather than by Docker as the stack starts.
 */
import { createServer } from "node:net";

const server = createServer();
server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("the server is not on a TCP port");
  }
  server.close(() => process.stdout.write(`${address.port}\n`));
});
