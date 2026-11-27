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

// ProseMirror measures the selection to scroll it into view as text is typed; jsdom
// measures elements as empty, and ranges not at all.
Range.prototype.getClientRects ??= function getClientRects() {
  return document.createElement("span").getClientRects();
};
Range.prototype.getBoundingClientRect ??= function getBoundingClientRect() {
  return document.createElement("span").getBoundingClientRect();
};
// BlockNote's suggestion menus note where they open with a DOMRect's toJSON; jsdom measures
// elements with plain objects, which lack it.
const measure = Element.prototype.getBoundingClientRect;
Element.prototype.getBoundingClientRect = function getBoundingClientRect() {
  const { left, top, width, height } = measure.call(this);
  return new DOMRect(left, top, width, height);
};
// The comments panel scrolls the thread opened into view, and the emoji menu its list.
Element.prototype.scrollIntoView ??= () => {};
Element.prototype.scrollTo ??= () => {};
// BlockNote looks for the block under the pointer, to offer its side menu there.
Document.prototype.elementFromPoint ??= () => null;
Document.prototype.elementsFromPoint ??= () => [];

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
