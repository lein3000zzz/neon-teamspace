import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";
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

beforeAll(() => server.listen({ onUnhandledFrame: "error" }));

afterEach(() => {
  cleanup();
  server.resetHandlers();
  window.localStorage.clear();
  const root = document.documentElement;
  for (const name of root.getAttributeNames()) {
    root.removeAttribute(name);
  }
});

afterAll(() => server.close());
