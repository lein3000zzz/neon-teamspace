import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { thirdPartyNotices } from "./scripts/licenses/notices.ts";

/**
 * The `neon` server's app port. The dev server forwards the paths the server
 * owns there and keeps the browser's Host header, as Caddy does in production,
 * so anything the server derives from Host behaves the same in both.
 */
const forward = { target: "http://127.0.0.1:8080", changeOrigin: false } as const;

/**
 * A proxy key for a path and everything below it, but not a longer name:
 * /api and /api/v1/meta, never /apiary. Vite reads keys starting with ^ as
 * regular expressions, tested against the path and query.
 */
function under(path: string): string {
  const literal = path.replaceAll(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return `^${literal}(?:[/?]|$)`;
}

export default defineConfig({
  plugins: [
    // Must run before the React plugin so route files are transformed first.
    tanstackRouter({ target: "react", autoCodeSplitting: true }),
    react(),
    tailwindcss(),
    // The shipped bundle carries its dependencies' and fonts' license notices.
    thirdPartyNotices(),
  ],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  server: {
    proxy: {
      [under("/api")]: forward,
      [under("/ws")]: { ...forward, ws: true },
      [under("/mcp")]: forward,
      [under("/hooks")]: forward,
      [under("/.well-known")]: forward,
    },
  },
  build: {
    outDir: "dist",
    // No data: URIs, so the production CSP can keep img-src and font-src at 'self'.
    assetsInlineLimit: 0,
    // The editor, BlockNote on ProseMirror, is one chunk of about 1.2 MB that only a page
    // loads; everything else stays well under the default 500 kB.
    chunkSizeWarningLimit: 1300,
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
    unstubGlobals: true,
    unstubEnvs: true,
  },
});
