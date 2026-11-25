import type { PointerEvent as ReactPointerEvent } from "react";

export type Point = { x: number; y: number };

type DragHandlers = {
  /** The pointer moved past the threshold: the drag began where it was pressed. */
  onStart?: (origin: Point) => void;
  onMove: (point: Point, origin: Point) => void;
  onDrop: (point: Point, origin: Point) => void;
  /** Escape, or the browser took the pointer over, as for a scroll. */
  onCancel: () => void;
};

/**
 * Follows a press of the mouse or a pen as a drag, once it moves `threshold`
 * pixels: the page's own reaction to the press is held back, so no text is
 * selected and an editor around it does not take it, and the click that ends
 * a drag is swallowed. A press that never moves far is a click as usual.
 * Touch is left to scroll; touch screens move things with their menus.
 */
export function startDrag(event: ReactPointerEvent, handlers: DragHandlers, threshold = 5) {
  if (event.button !== 0 || event.pointerType === "touch") {
    return;
  }
  event.preventDefault();
  const origin = { x: event.clientX, y: event.clientY };
  const pointerId = event.pointerId;
  let active = false;

  const stop = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key, true);
  };
  const move = (moved: PointerEvent) => {
    if (moved.pointerId !== pointerId) {
      return;
    }
    const point = { x: moved.clientX, y: moved.clientY };
    if (!active) {
      if (Math.hypot(point.x - origin.x, point.y - origin.y) < threshold) {
        return;
      }
      active = true;
      handlers.onStart?.(origin);
    }
    handlers.onMove(point, origin);
  };
  const up = (released: PointerEvent) => {
    if (released.pointerId !== pointerId) {
      return;
    }
    stop();
    if (active) {
      swallowClick();
      handlers.onDrop({ x: released.clientX, y: released.clientY }, origin);
    }
  };
  const cancel = () => {
    stop();
    if (active) {
      handlers.onCancel();
    }
  };
  const key = (pressed: KeyboardEvent) => {
    if (pressed.key === "Escape" && active) {
      pressed.preventDefault();
      pressed.stopPropagation();
      cancel();
    }
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", key, true);
}

function swallow(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
}

/** The click a browser sends as a drag ends is not a click on what it ended over. */
function swallowClick() {
  window.addEventListener("click", swallow, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
}

/** The element under a point that carries a data attribute, by its name in a dataset. */
export function underPoint(point: Point, attribute: string): HTMLElement | undefined {
  for (const element of document.elementsFromPoint(point.x, point.y)) {
    if (element instanceof HTMLElement && element.dataset[attribute] !== undefined) {
      return element;
    }
  }
  return undefined;
}

/** How near a container's edge a drag scrolls it, in pixels. */
const EDGE = 48;

/**
 * Scrolls a container sideways while a drag is inside it, near one of its
 * edges: the nearer, the further each move scrolls.
 */
export function scrollNearEdge(container: HTMLElement | null, point: Point) {
  if (container === null) {
    return;
  }
  const box = container.getBoundingClientRect();
  if (point.y < box.top || point.y > box.bottom || point.x < box.left || point.x > box.right) {
    return;
  }
  const right = point.x - (box.right - EDGE);
  const left = box.left + EDGE - point.x;
  if (right > 0) {
    container.scrollBy({ left: Math.ceil(right / 2) });
  } else if (left > 0) {
    container.scrollBy({ left: -Math.ceil(left / 2) });
  }
}
