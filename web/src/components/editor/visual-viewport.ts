import { useSyncExternalStore } from "react";

/**
 * The part of the page the screen shows, in the layout viewport's CSS pixels:
 * what is left above an on-screen keyboard, which a phone's browser lays over
 * the page rather than shrinking it, and how far it is zoomed in.
 */
export type ViewportBox = {
  top: number;
  left: number;
  width: number;
  height: number;
  scale: number;
  /**
   * How much of the layout viewport's bottom the screen does not show: the
   * keyboard's height, where it covers the page; zero where the page shrank
   * for it instead, or without one.
   */
  covered: number;
};

let last: ViewportBox | null = null;

function read(): ViewportBox {
  const viewport = window.visualViewport;
  const layoutHeight = document.documentElement.clientHeight || window.innerHeight;
  const next: ViewportBox =
    viewport === null || viewport === undefined
      ? {
          top: 0,
          left: 0,
          width: window.innerWidth,
          height: window.innerHeight,
          scale: 1,
          covered: 0,
        }
      : {
          top: viewport.offsetTop,
          left: viewport.offsetLeft,
          width: viewport.width,
          height: viewport.height,
          scale: viewport.scale,
          covered: Math.max(0, Math.round(layoutHeight - viewport.offsetTop - viewport.height)),
        };
  if (
    last !== null &&
    last.top === next.top &&
    last.left === next.left &&
    last.width === next.width &&
    last.height === next.height &&
    last.scale === next.scale &&
    last.covered === next.covered
  ) {
    return last;
  }
  last = next;
  return next;
}

function subscribe(notify: () => void) {
  const viewport = window.visualViewport;
  viewport?.addEventListener("resize", notify);
  viewport?.addEventListener("scroll", notify);
  window.addEventListener("resize", notify);
  return () => {
    viewport?.removeEventListener("resize", notify);
    viewport?.removeEventListener("scroll", notify);
    window.removeEventListener("resize", notify);
  };
}

/**
 * The visual viewport as it changes: with the VisualViewport API where the
 * browser has it, as every current one and the apps' webviews do, and the
 * window otherwise.
 */
export function useVisualViewport(): ViewportBox {
  return useSyncExternalStore(subscribe, read);
}

/**
 * Where an element fixed at the layout viewport's top left goes to sit along
 * the bottom of the visual viewport, as wide as the screen and at its own
 * size however far the page is zoomed: above the keyboard, where one covers
 * the page.
 */
export function bottomOf(box: ViewportBox): {
  width: string;
  transform: string;
  transformOrigin: string;
} {
  return {
    width: `${box.width * box.scale}px`,
    transform: `translate(${box.left}px, ${box.top + box.height}px) translateY(-100%) scale(${1 / box.scale})`,
    transformOrigin: "0 100%",
  };
}
