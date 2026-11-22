import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, vi } from "vitest";
import { server } from "./server";

// Radix measures its slider thumb with ResizeObserver, which jsdom lacks.
class NoopResizeObserver implements ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= NoopResizeObserver;

// The router restores scroll positions on navigation; jsdom has no layout to scroll.
window.scrollTo = () => {};

// index.html's viewport, which BlockNote checks for as its editor starts.
const viewport = document.createElement("meta");
viewport.name = "viewport";
viewport.content =
  "width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content";
document.head.append(viewport);

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));

afterEach(() => {
  cleanup();
  server.resetHandlers();
  // Before touching storage, which a test may have blocked with a spy.
  vi.restoreAllMocks();
  vi.useRealTimers();
  window.localStorage.clear();
  window.sessionStorage.clear();
  const root = document.documentElement;
  for (const name of root.getAttributeNames()) {
    root.removeAttribute(name);
  }
});

afterAll(() => server.close());
