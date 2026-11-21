import { vi } from "vitest";
import { DARK_QUERY } from "@/theme/preferences";

class FakeMediaQueryList extends EventTarget implements MediaQueryList {
  readonly media: string;
  onchange: MediaQueryList["onchange"] = null;
  #matches: () => boolean;

  constructor(media: string, matches: () => boolean) {
    super();
    this.media = media;
    this.#matches = matches;
  }

  get matches() {
    return this.#matches();
  }

  addListener(listener: (event: MediaQueryListEvent) => void) {
    this.addEventListener("change", listener as EventListener);
  }

  removeListener(listener: (event: MediaQueryListEvent) => void) {
    this.removeEventListener("change", listener as EventListener);
  }
}

/**
 * Replaces window.matchMedia (absent in jsdom) with one whose
 * prefers-color-scheme answer the test controls and can flip live.
 */
export function mockColorScheme(initial: "light" | "dark") {
  let dark = initial === "dark";
  const lists = new Map<string, FakeMediaQueryList>();
  vi.stubGlobal("matchMedia", (query: string) => {
    let list = lists.get(query);
    if (list === undefined) {
      list = new FakeMediaQueryList(query, () => query === DARK_QUERY && dark);
      lists.set(query, list);
    }
    return list;
  });
  return {
    set(next: "light" | "dark") {
      dark = next === "dark";
      lists.get(DARK_QUERY)?.dispatchEvent(new Event("change"));
    },
  };
}
