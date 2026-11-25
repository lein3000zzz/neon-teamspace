import { useWindowVirtualizer, type VirtualItem } from "@tanstack/react-virtual";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** Items drawn beyond each edge of the screen, so a quick scroll shows no gap. */
const OVERSCAN = 10;

type Windowed<E extends HTMLElement> = {
  /** Goes on the element that holds the items: where the list starts on the page. */
  list: (element: E | null) => void;
  /** The items to draw, near the screen, each at its index of the whole list. */
  items: VirtualItem[];
  /** The height of the items above those drawn, and of those below them, as space. */
  before: number;
  after: number;
  /** Goes on each item drawn, with its index as `data-index`, so its height is known. */
  measure: (element: Element | null) => void;
  /** Scrolls the page to an item, as far as it takes to show it, which draws it. */
  scrollTo: (index: number) => void;
};

/**
 * Draws only the items of a long list near the screen, as the page scrolls:
 * a view of thousands of rows stays as quick as one of a few. Items not drawn
 * are kept as space, `before` and `after` those drawn, so the page scrolls as
 * though all were there. `keys` names each item, in order, so a measured
 * height follows it; `estimate` is an item's height until it is measured.
 */
export function useWindowed<E extends HTMLElement>(
  keys: readonly string[],
  estimate: number,
): Windowed<E> {
  const element = useRef<E | null>(null);
  // Where the list starts on the page, which the items' places count from.
  const [margin, setMargin] = useState(0);
  const remeasure = useCallback(() => {
    const top = element.current?.getBoundingClientRect().top;
    if (top !== undefined) {
      const start = Math.round(top + window.scrollY);
      setMargin((current) => (current === start ? current : start));
    }
  }, []);
  const list = useCallback(
    (next: E | null) => {
      element.current = next;
      remeasure();
    },
    [remeasure],
  );

  // What is above the list may change with every draw, and as the page's content does.
  useLayoutEffect(remeasure);
  useEffect(() => {
    const observer = new ResizeObserver(remeasure);
    observer.observe(document.body);
    return () => observer.disconnect();
  }, [remeasure]);

  const keyOf = useCallback((index: number) => keys[index] ?? String(index), [keys]);
  const estimateSize = useCallback(() => estimate, [estimate]);
  // The app is not built with React Compiler, whose memoizing this rule guards: the virtualizer
  // draws its component again on every change, and only its stable methods are passed on.
  // oxlint-disable-next-line react/incompatible-library
  const virtualizer = useWindowVirtualizer({
    count: keys.length,
    estimateSize,
    overscan: OVERSCAN,
    scrollMargin: margin,
    getItemKey: keyOf,
  });
  const scrollTo = useCallback(
    (index: number) => virtualizer.scrollToIndex(index, { align: "auto" }),
    [virtualizer],
  );
  const items = virtualizer.getVirtualItems();
  const first = items[0];
  const last = items.at(-1);
  return {
    list,
    items,
    before: first === undefined ? 0 : first.start - margin,
    after: last === undefined ? 0 : Math.max(0, virtualizer.getTotalSize() - (last.end - margin)),
    measure: virtualizer.measureElement,
    scrollTo,
  };
}
