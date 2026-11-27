import type { FloatingUIOptions } from "@blocknote/react";
import {
  detectOverflow,
  flip,
  offset,
  shift,
  size,
  type Middleware,
  type Placement,
  type Rect,
} from "@floating-ui/react";

/** The gap between a menu and the text it is for. */
const GAP = 8;

/** The room a menu keeps from the edges of the window. */
const EDGE = 12;

/**
 * The tallest a menu of suggestions grows, whatever room there is: a longer
 * list scrolls. Opened above the caret, it stays clear of the page's header.
 */
export const MENU_MAX_HEIGHT = 352;

/**
 * The room a menu keeps from each edge of the window: more at the bottom while
 * the editing toolbar of a touch screen shows there, above the keyboard, so a
 * menu never goes under it.
 */
function edges(): { top: number; right: number; bottom: number; left: number } {
  const toolbar = document.querySelector<HTMLElement>("[data-editing-toolbar]:not([hidden])");
  const covered = toolbar === null ? 0 : toolbar.getBoundingClientRect().height;
  return { top: EDGE, right: EDGE, bottom: EDGE + covered, left: EDGE };
}

/**
 * Below the caret, unless a menu of the tallest height has no room there and
 * more above. The menu's own height is not asked, as its items come after it
 * opens, so it keeps its side while the list is filtered.
 */
const belowUnlessCramped: Middleware = {
  name: "belowUnlessCramped",
  async fn(state) {
    const { rects, placement } = state;
    const floating = { ...rects.floating, height: MENU_MAX_HEIGHT };
    const padding = edges();
    const at = (y: number) =>
      detectOverflow({ ...state, rects: { ...rects, floating }, y }, { padding });
    const below = await at(rects.reference.y + rects.reference.height + GAP);
    let wanted: Placement = "bottom-start";
    if (below.bottom > 0) {
      const above = await at(rects.reference.y - GAP - MENU_MAX_HEIGHT);
      if (above.top < below.bottom) {
        wanted = "top-start";
      }
    }
    return wanted === placement ? {} : { reset: { placement: wanted } };
  },
};

/**
 * Keeps a menu at the widest it has been since it opened, so it does not
 * shrink and grow as the list is filtered. Whole pixels, so it never loops.
 */
function keepWidest(element: HTMLElement) {
  const width = element.getBoundingClientRect().width;
  if (width >= (Number.parseFloat(element.style.minWidth) || 0) + 1) {
    element.style.minWidth = `${Math.ceil(width)}px`;
  }
}

/**
 * Where the slash and mention menus go: below the caret unless cramped there,
 * clear of the window's edges and the editing toolbar by a margin, as wide as
 * they have been, and no taller than the room on their side or
 * {@link MENU_MAX_HEIGHT}.
 */
export const suggestionMenuOptions: FloatingUIOptions = {
  useFloatingOptions: {
    middleware: [
      belowUnlessCramped,
      offset(GAP),
      shift(() => ({ padding: edges() })),
      size(() => ({
        padding: edges(),
        apply({ elements, availableHeight, availableWidth }) {
          const style = elements.floating.style;
          style.maxHeight = `${Math.max(0, Math.min(availableHeight, MENU_MAX_HEIGHT))}px`;
          style.maxWidth = `${Math.max(0, availableWidth)}px`;
          keepWidest(elements.floating);
        },
      })),
    ],
  },
};

/**
 * Where the formatting toolbar may go: over the page's text, from the left of
 * the editor to its right or to the comments panel's left edge, whichever
 * comes first, so it never covers the panel open beside the page.
 */
function pageColumn(editor: HTMLElement | undefined): Rect | undefined {
  if (editor === undefined) {
    return undefined;
  }
  const column = editor.getBoundingClientRect();
  const panel = document.querySelector("[data-comments-panel]")?.getBoundingClientRect();
  const right = Math.min(column.right, panel?.left ?? column.right);
  return {
    x: column.left,
    y: 0,
    width: Math.max(0, right - column.left),
    height: window.innerHeight,
  };
}

/** The formatting toolbar above the text selected, kept over the page's text. */
export function formattingToolbarOptions(editor: () => HTMLElement | undefined): FloatingUIOptions {
  return {
    useFloatingOptions: {
      middleware: [
        offset(10),
        shift(() => {
          const boundary = pageColumn(editor());
          return boundary === undefined ? {} : { boundary };
        }),
        flip(),
      ],
    },
  };
}
