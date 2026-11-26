import type { FloatingUIOptions } from "@blocknote/react";
import { flip, offset, shift, type Rect } from "@floating-ui/react";

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
