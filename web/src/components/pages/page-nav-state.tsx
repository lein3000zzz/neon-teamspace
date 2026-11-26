import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { browserStorage } from "@/theme/preferences";

/** A page being dragged in the sidebar, and what it may not be dropped on. */
export type DraggedPage = {
  id: string;
  title: string;
  /** The page and the pages under it: a page never goes under itself. */
  blocked: ReadonlySet<string>;
};

/**
 * What went wrong with the last change made from the sidebar, or what it did:
 * `restore` names the page its Undo brings back from the trash. The notice
 * does that itself, as the row the page was trashed from is gone by then.
 */
export type NavNotice =
  { tone: "error"; error: unknown } | { tone: "info"; text: string; restore?: string };

type PageNavState = {
  /** Pages are closed until opened; sections, such as a teamspace's, open until folded. */
  isOpen: (id: string) => boolean;
  setOpen: (id: string, open: boolean) => void;
  /** Opens every page above the one in view, so it shows in its tree. */
  openAll: (ids: readonly string[]) => void;
  isFolded: (section: string) => boolean;
  setFolded: (section: string, folded: boolean) => void;
  dragged: DraggedPage | null;
  setDragged: (page: DraggedPage | null) => void;
  notice: NavNotice | null;
  setNotice: (notice: NavNotice | null) => void;
};

const PageNavContext = createContext<PageNavState | null>(null);

/** One entry per account, so people sharing a browser each keep their own. */
const foldedKey = (userId: string) => `neon.folded.${userId}`;

/** The sections the user folded, as this browser keeps them; none where storage is refused. */
function storedFolded(userId: string): ReadonlySet<string> {
  try {
    const parsed: unknown = JSON.parse(browserStorage()?.getItem(foldedKey(userId)) ?? "[]");
    return new Set(
      Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : [],
    );
  } catch {
    return new Set();
  }
}

function storeFolded(userId: string, folded: ReadonlySet<string>) {
  try {
    browserStorage()?.setItem(foldedKey(userId), JSON.stringify([...folded]));
  } catch {
    // Kept while the app runs, in state.
  }
}

type PageNavProviderProps = {
  /** Whose sidebar it is: the sections they fold stay folded in this browser. */
  userId: string;
  children: ReactNode;
};

/**
 * What the page trees in the sidebar keep while the app runs: which pages are
 * open, which sections folded, which this browser also keeps for next time,
 * the page being dragged, and the last notice. The column and the drawer show
 * the same sidebar, so they share it.
 */
export function PageNavProvider({ userId, children }: PageNavProviderProps) {
  const [open, setOpenIds] = useState<ReadonlySet<string>>(() => new Set());
  const [folded, setFoldedIds] = useState<ReadonlySet<string>>(() => storedFolded(userId));
  const [dragged, setDragged] = useState<DraggedPage | null>(null);
  const [notice, setNotice] = useState<NavNotice | null>(null);

  const setOpen = useCallback((id: string, next: boolean) => {
    setOpenIds((current) => toggled(current, id, next));
  }, []);
  const openAll = useCallback((ids: readonly string[]) => {
    setOpenIds((current) =>
      ids.every((id) => current.has(id)) ? current : new Set([...current, ...ids]),
    );
  }, []);
  const setFolded = useCallback((section: string, next: boolean) => {
    setFoldedIds((current) => toggled(current, section, next));
  }, []);
  useEffect(() => storeFolded(userId, folded), [userId, folded]);

  const value = useMemo(
    () => ({
      isOpen: (id: string) => open.has(id),
      setOpen,
      openAll,
      isFolded: (section: string) => folded.has(section),
      setFolded,
      dragged,
      setDragged,
      notice,
      setNotice,
    }),
    [open, folded, dragged, notice, setOpen, openAll, setFolded],
  );
  return <PageNavContext value={value}>{children}</PageNavContext>;
}

function toggled(set: ReadonlySet<string>, id: string, member: boolean): ReadonlySet<string> {
  if (set.has(id) === member) {
    return set;
  }
  const next = new Set(set);
  if (member) {
    next.add(id);
  } else {
    next.delete(id);
  }
  return next;
}

export function usePageNav(): PageNavState {
  const state = use(PageNavContext);
  if (state === null) {
    throw new Error("usePageNav must be used inside <PageNavProvider>");
  }
  return state;
}
