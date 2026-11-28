import { useRouter, type ParsedLocation, type RouterHistory } from "@tanstack/react-router";
import { useEffect, useId, useReducer, useRef } from "react";
import { COARSE_QUERY, useMediaQuery } from "./media";

/**
 * Panels over a page, such as the drawer, a sheet, a dialog or the comments,
 * as entries of the browser's history on a touch screen, so the back gesture
 * of a phone, which goes back in the history, or a phone browser's Back
 * closes the panel open on top rather than leaving the page.
 *
 * A panel that opens gets an entry of its own, at the page's address, whose
 * state lists the panels open in it, outermost first. Going back past it
 * closes the panel, and going forward to it again opens it again, while it is
 * still there to open. A panel closed any other way, with its button, Escape,
 * or as what holds it goes, takes its entry back with it, so Back then leaves
 * the page as it would have before. A navigation from a panel's entry, such as
 * a link in the drawer, takes the place of the panels' entries rather than
 * going on top of them, so none is left behind to step through. The entries
 * share the scroll position of the page's own.
 */

/** A panel, as long as it is mounted. */
type Panel = {
  /** Opens or closes it, as a step through the history asks. */
  set: (open: boolean) => void;
  open: boolean;
  /** When it last opened, so panels opened at once get their entries in that order. */
  since: number;
};

type State = RouterHistory["location"]["state"];

/** What the router's history adds to each entry's state: its key, and its place. */
const KEY = "__TSR_key";
const INDEX = "__TSR_index";

/** The panels open in a history entry, outermost first. */
function panelsOf(state: State): readonly string[] {
  return Array.isArray(state.panels) ? state.panels : [];
}

/** The key of the page's entry a panel's entry is over, or of the entry itself. */
function pageKeyOf(state: State): string | undefined {
  return state.panelsOver ?? state[KEY];
}

/** The state of an entry less what the history adds to each, which a new entry gets afresh. */
function ownState(state: State): Partial<State> {
  const own: Partial<State> = { ...state };
  delete own.key;
  delete own[KEY];
  delete own[INDEX];
  return own;
}

/** The path part of an address the history takes. */
function pathOf(href: string): string {
  return href.split(/[?#]/, 1)[0] ?? href;
}

/** How long to wait for the browser to take a step back before giving up on it. */
const STEP_TIMEOUT = 2000;

/** The history a router keeps its panels in, by the history it was made with. */
const histories = new WeakMap<RouterHistory, PanelHistory>();

class PanelHistory {
  readonly #history: RouterHistory;
  readonly #push: RouterHistory["push"];
  readonly #replace: RouterHistory["replace"];
  readonly #panels = new Map<string, Panel>();
  /** The panels the entry last seen lists. */
  #listed: readonly string[];
  /** A step back asked of the browser that has not arrived, and what to do once it does. */
  #stepping: { after: () => void; timer: ReturnType<typeof setTimeout> } | null = null;
  /** While this changes the history itself, rather than the router or the browser. */
  #own = false;
  #scheduled = false;
  #opened = 0;

  constructor(history: RouterHistory) {
    this.#history = history;
    this.#push = history.push.bind(history);
    this.#replace = history.replace.bind(history);
    history.push = (path, state, options) => this.#navigate("push", path, state, options);
    history.replace = (path, state, options) => this.#navigate("replace", path, state, options);
    this.#listed = panelsOf(history.location.state);
    history.subscribe(({ action }) => this.#moved(action.type));
    // An entry left with panels in it, as by a reload, has none open any more.
    this.#schedule();
  }

  /** Follows a mounted panel, closed until it says otherwise; the function returned forgets it. */
  register(id: string, set: (open: boolean) => void): () => void {
    const panel: Panel = { set, open: false, since: 0 };
    this.#panels.set(id, panel);
    return () => {
      if (this.#panels.get(id) === panel) {
        this.#panels.delete(id);
        this.#schedule();
      }
    };
  }

  /** Whether the panel is open, as it says each time that changes, or it was asked to open. */
  setOpen(id: string, open: boolean) {
    const panel = this.#panels.get(id);
    if (panel === undefined || panel.open === open) {
      return;
    }
    panel.open = open;
    if (open) {
      panel.since = ++this.#opened;
    }
    this.#schedule();
  }

  /**
   * The router's own navigations. From an entry with panels, one to another page leaves
   * them: a push takes the place of their entries, a replace that of the page's under them.
   * One that only changes the page's address keeps them open over it.
   */
  #navigate(
    kind: "push" | "replace",
    path: string,
    state: Parameters<RouterHistory["push"]>[1],
    options?: Parameters<RouterHistory["push"]>[2],
  ) {
    if (this.#stepping !== null) {
      const { after } = this.#stepping;
      this.#stepping.after = () => {
        after();
        this.#navigate(kind, path, state, options);
      };
      return;
    }
    const location = this.#history.location;
    const panels = panelsOf(location.state);
    if (panels.length === 0) {
      (kind === "push" ? this.#push : this.#replace)(path, state, options);
    } else if (kind === "replace" && pathOf(path) === location.pathname) {
      this.#replace(path, { ...state, panels, panelsOver: pageKeyOf(location.state) }, options);
    } else {
      this.#back(kind === "push" ? panels.length - 1 : panels.length, () =>
        this.#replace(path, state, options),
      );
    }
  }

  /**
   * The history moved: to an entry this asked for, or one the router or the browser went to.
   * Back closes the panels whose entries it passed, and forward opens them again; a panel that
   * does not open so says it is closed, which takes the step back again.
   */
  #moved(action: string) {
    const before = this.#listed;
    const after = panelsOf(this.#history.location.state);
    this.#listed = after;
    const stepping = this.#stepping;
    const arrived = stepping !== null && (action === "BACK" || action === "GO");
    if (arrived) {
      clearTimeout(stepping.timer);
      this.#stepping = null;
    }
    if (!this.#own) {
      for (const [id, panel] of this.#panels) {
        if (panel.open && before.includes(id) && !after.includes(id)) {
          panel.open = false;
          panel.set(false);
        } else if (!panel.open && after.includes(id) && !before.includes(id)) {
          panel.open = true;
          panel.since = ++this.#opened;
          panel.set(true);
        }
      }
    }
    if (arrived) {
      stepping.after();
    }
    this.#schedule();
  }

  #schedule() {
    if (!this.#scheduled) {
      this.#scheduled = true;
      queueMicrotask(() => {
        this.#scheduled = false;
        this.#settle();
      });
    }
  }

  /**
   * Brings the history in line with the panels open: back out of the entries of panels
   * closed or gone, then an entry for each panel opened since, in the order they opened.
   */
  #settle() {
    if (this.#stepping !== null) {
      return;
    }
    const listed = panelsOf(this.#history.location.state);
    const closed = listed.findIndex((id) => this.#panels.get(id)?.open !== true);
    if (closed >= 0) {
      this.#back(listed.length - closed, () => {});
      return;
    }
    const opening = [...this.#panels]
      .filter(([id, panel]) => panel.open && !listed.includes(id))
      .toSorted(([, a], [, b]) => a.since - b.since);
    for (const [id] of opening) {
      const location = this.#history.location;
      this.#own = true;
      try {
        this.#push(location.href, {
          ...ownState(location.state),
          panels: [...panelsOf(location.state), id],
          panelsOver: pageKeyOf(location.state),
        });
      } finally {
        this.#own = false;
      }
    }
  }

  /**
   * Goes back `steps` entries, past panels' entries to the page's, then does `after`. Where
   * the browser does not take the step, as there is no entry that far back, the entry in view
   * is made the page's own instead.
   */
  #back(steps: number, after: () => void) {
    if (steps <= 0) {
      after();
      return;
    }
    const plain = () => {
      this.#stepping = null;
      const location = this.#history.location;
      const state = ownState(location.state);
      delete state.panels;
      delete state.panelsOver;
      this.#own = true;
      try {
        this.#replace(location.href, state);
      } finally {
        this.#own = false;
      }
      after();
    };
    if (this.#history.location.state[INDEX] < steps) {
      plain();
      return;
    }
    this.#stepping = { after, timer: setTimeout(plain, STEP_TIMEOUT) };
    // A push still queued for the browser goes first, so the step starts from it.
    this.#history.flush();
    this.#history.go(-steps);
  }
}

/**
 * Keeps panels' entries in `history`, for a router to be made with; the router's own
 * navigations go through it.
 */
export function withPanels(history: RouterHistory): RouterHistory {
  if (!histories.has(history)) {
    histories.set(history, new PanelHistory(history));
  }
  return history;
}

/**
 * Where the router keeps the scroll position of a history entry: a panel's entry keeps the
 * page's, under it, so opening or closing a panel scrolls nothing.
 */
export function scrollKeyOf(location: ParsedLocation): string {
  return pageKeyOf(location.state) ?? location.href;
}

/**
 * Gives a panel an entry in the history while it is open, on a touch screen, where the
 * back gesture or Back closes it: `onOpenChange(false)` as the entry is left, and
 * `onOpenChange(true)` as forward returns to it. Where the main pointer is fine, as on a
 * computer, the history is left as it was. `enabled` false leaves out a panel the address
 * itself opens, whose entry the history has already.
 */
export function useBackCloses(
  open: boolean,
  onOpenChange: (open: boolean) => void,
  enabled = true,
) {
  const router = useRouter({ warn: false }) as ReturnType<typeof useRouter> | undefined;
  const panels = router === undefined ? undefined : histories.get(router.history);
  const touch = useMediaQuery(COARSE_QUERY);
  const id = useId();
  const change = useRef(onOpenChange);
  useEffect(() => {
    change.current = onOpenChange;
  });
  // Renders again as forward asks it to open, to say whether it did: not every panel opens
  // again from onOpenChange alone.
  const [, render] = useReducer((times: number) => times + 1, 0);
  const active = panels !== undefined && touch && enabled;

  useEffect(() => {
    if (panels === undefined || !active) {
      return;
    }
    return panels.register(id, (next) => {
      if (next) {
        render();
      }
      change.current(next);
    });
  }, [panels, active, id]);

  // After each render, it says whether it is open.
  useEffect(() => {
    if (panels !== undefined && active) {
      panels.setOpen(id, open);
    }
  });
}
