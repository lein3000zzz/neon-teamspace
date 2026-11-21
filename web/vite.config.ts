import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

/** The `neon` server's app port; the dev server forwards everything it owns there. */
const backend = "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [
    // Must run before the React plugin so route files are transformed first.
    tanstackRouter({ target: "react", autoCodeSplitting: true }),
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  server: {
    proxy: {
      "/api": backend,
      "/ws": { target: backend, ws: true },
      "/mcp": backend,
      "/hooks": backend,
      "/.well-known": backend,
    },
  },
  build: {
    outDir: "dist",
    // No data: URIs, so the production CSP can keep img-src and font-src at 'self'.
    assetsInlineLimit: 0,
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
    unstubGlobals: true,
  },
});
