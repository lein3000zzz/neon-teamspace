import { readFileSync } from "node:fs";
import { defineConfig } from "vite";

/** The policy the app gives the picker, from tauri.conf.json, which the preview serves too. */
function appPolicy(): string {
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")) as {
    app: { security: { csp: Record<string, string> } };
  };
  return Object.entries(config.app.security.csp)
    .map(([directive, sources]) => `${directive} ${sources}`)
    .join("; ");
}

/**
 * The bundled picker. Tauri loads dist/ from inside the app, and `tauri dev` the development
 * server on port 1420. The picker reads the web app's palettes, outside this directory, which
 * the development server is allowed to serve. The preview serves dist/ under the app's
 * Content-Security-Policy, for the tests.
 */
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    fs: { allow: [".", "../../web/src/styles"] },
  },
  preview: {
    host: "127.0.0.1",
    strictPort: true,
    headers: { "Content-Security-Policy": appPolicy() },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Tauri reads the files as they are; no source maps go into the app.
    sourcemap: false,
  },
});
