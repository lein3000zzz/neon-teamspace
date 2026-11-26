import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { FileText, Rows3, Search, Table2 } from "lucide-react";
import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import type { Page, PageKind } from "@/api/pages";
import { quickSearchQuery, recentPagesQuery, type TextRun } from "@/api/search";
import type { WorkspaceSummary } from "@/api/workspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { useCurrentWorkspace } from "@/components/workspaces/workspace-context";
import { useDebounced } from "@/lib/debounce";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { Runs } from "./runs";

const copy = strings.search.quick;

/** How long typing pauses before the quick search asks the server. */
const TYPING_PAUSE_MS = 150;

type QuickSearchState = {
  /** Whether there is a workspace to search. */
  available: boolean;
  open: () => void;
};

const QuickSearchContext = createContext<QuickSearchState>({ available: false, open: () => {} });

/** Opens the quick search, from a button. */
export function useQuickSearch(): QuickSearchState {
  return use(QuickSearchContext);
}

/** Whether the keyboard's command key is ⌘, as on Apple's systems, rather than Ctrl. */
export function commandIsMeta(): boolean {
  return /Mac|iPhone|iPad/.test(navigator.userAgent);
}

/**
 * The quick search of the workspace the sidebar shows, opened by ⌘K or Ctrl+K
 * anywhere, the editor included, and by the sidebar's button.
 */
export function QuickSearchProvider({ children }: { children: ReactNode }) {
  const { current } = useCurrentWorkspace();
  const [open, setOpen] = useState(false);
  const available = current !== undefined;

  useEffect(() => {
    if (!available) {
      return;
    }
    // Caught on the way down, so the editor's own shortcut for links does not take it first.
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (
        (event.metaKey || event.ctrlKey) &&
        !event.altKey &&
        !event.shiftKey &&
        event.key.toLowerCase() === "k"
      ) {
        event.preventDefault();
        event.stopPropagation();
        setOpen((was) => !was);
      }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [available]);

  const state = useMemo(() => ({ available, open: () => setOpen(true) }), [available]);
  return (
    <QuickSearchContext value={state}>
      {children}
      {current !== undefined && (
        <Dialog open={open} onOpenChange={setOpen}>
          {open && <QuickSearch workspace={current} onClose={() => setOpen(false)} />}
        </Dialog>
      )}
    </QuickSearchContext>
  );
}

/** What the list offers: a page found or changed lately, or the search page itself. */
type Option =
  | { kind: "page"; page: Page; title: TextRun[]; snippet: TextRun[] }
  | { kind: "everything"; query: string };

const glyphs: Record<PageKind, typeof FileText> = {
  page: FileText,
  database: Table2,
  row: Rows3,
};

/**
 * The quick search itself. Focus starts in the field; the arrow keys move it
 * down the results and back up to the field, Enter opens the result focused,
 * or from the field the first of what was typed, once its results came, and
 * Escape closes it all. The dialog's own button closes it, so the field shows
 * no clear button of the browser's beside it.
 */
function QuickSearch({ workspace, onClose }: { workspace: WorkspaceSummary; onClose: () => void }) {
  const navigate = useNavigate();
  const field = useRef<HTMLInputElement>(null);
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const [text, setText] = useState("");
  const typed = text.trim();
  const query = useDebounced(typed, TYPING_PAUSE_MS);
  const searching = query !== "";
  const recent = useQuery({ ...recentPagesQuery(workspace.id), enabled: !searching });
  const found = useQuery({ ...quickSearchQuery(workspace.id, query), enabled: searching });
  // What was typed when Enter was pressed in the field before its results came, which then
  // opens the first of them.
  const [entered, setEntered] = useState<string | null>(null);
  // The list is that of what is typed: not of what was typed before, or of nothing.
  const settled =
    query === typed && (searching ? found.isSuccess && !found.isPlaceholderData : recent.isSuccess);

  const options: Option[] = useMemo(() => {
    if (!searching) {
      return (recent.data ?? []).map((page) => ({ kind: "page", page, title: [], snippet: [] }));
    }
    const pages: Option[] = (found.data?.results ?? []).map((result) => ({
      kind: "page",
      page: result.page,
      title: result.title,
      snippet: result.snippet,
    }));
    return [...pages, { kind: "everything", query }];
  }, [searching, recent.data, found.data, query]);

  const choose = useCallback(
    (option: Option | undefined) => {
      if (option === undefined) {
        return;
      }
      if (option.kind === "page") {
        void navigate({
          to: "/w/$slug/p/$pageId",
          params: { slug: workspace.slug, pageId: option.page.id },
        });
      } else {
        void navigate({
          to: "/w/$slug/search",
          params: { slug: workspace.slug },
          search: { q: option.query },
        });
      }
      onClose();
    },
    [navigate, workspace.slug, onClose],
  );

  // Choosing closes the quick search, which ends what was entered with it.
  useEffect(() => {
    if (entered !== null && entered === typed && settled) {
      choose(options[0]);
    }
  }, [entered, typed, settled, options, choose]);

  /** Moves focus to the result at `index`; above the first, to the field. */
  const focusAt = (index: number) => {
    if (index < 0) {
      field.current?.focus();
    } else {
      buttons.current[Math.min(index, options.length - 1)]?.focus();
    }
  };

  const onFieldKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" && options.length > 0) {
      event.preventDefault();
      focusAt(0);
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (settled) {
        choose(options[0]);
      } else {
        setEntered(typed);
      }
    }
  };

  const onOptionKey = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusAt(index + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      focusAt(index - 1);
    }
  };

  const loading = searching ? found.isPending : recent.isPending;
  const error = searching ? found.error : recent.error;
  const pages = options.filter((option) => option.kind === "page").length;
  return (
    <DialogContent
      closeLabel={copy.close}
      aria-describedby={undefined}
      className="top-[12vh] w-[min(40rem,calc(100vw-2rem))] translate-y-0 gap-3 p-3"
    >
      <DialogTitle className="sr-only">{copy.title}</DialogTitle>
      <search className="flex items-center gap-2 pr-10">
        <Search aria-hidden className="ml-1 size-4 shrink-0 text-muted-foreground" />
        <input
          ref={field}
          type="search"
          aria-label={copy.label}
          placeholder={copy.placeholder}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setEntered(null);
          }}
          onKeyDown={onFieldKey}
          className="h-(--tap) min-w-0 flex-1 bg-transparent text-base outline-none placeholder:text-muted-foreground md:text-sm [&::-webkit-search-cancel-button]:appearance-none"
        />
      </search>
      <section aria-label={searching ? copy.results : copy.recent} className="grid gap-1">
        <p aria-hidden className="px-1 text-xs font-medium text-muted-foreground">
          {searching ? copy.results : copy.recent}
        </p>
        {error !== null && <ErrorNotice error={error} />}
        <ul className="grid gap-0.5">
          {options.map((option, index) => (
            <li key={option.kind === "page" ? option.page.id : "everything"}>
              <button
                type="button"
                ref={(button) => {
                  buttons.current[index] = button;
                }}
                onClick={() => choose(option)}
                onKeyDown={(event) => onOptionKey(event, index)}
                className="grid min-h-(--tap) w-full content-center gap-0.5 rounded-md px-2.5 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent"
              >
                {option.kind === "page" ? (
                  <PageOption option={option} />
                ) : (
                  <span className="flex items-center gap-2 text-muted-foreground">
                    <Search aria-hidden className="size-4 shrink-0" />
                    {copy.everything(option.query)}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
        {!loading && error === null && pages === 0 && (
          <p className="px-2.5 pb-1 text-sm text-muted-foreground">
            {searching ? copy.none : copy.noRecent}
          </p>
        )}
        {loading && <p className="px-2.5 pb-1 text-sm text-muted-foreground">{copy.searching}</p>}
      </section>
      <p className="hidden px-1 text-xs text-muted-foreground md:block">{copy.hint}</p>
    </DialogContent>
  );
}

function PageOption({ option }: { option: Extract<Option, { kind: "page" }> }) {
  const Glyph = glyphs[option.page.kind];
  return (
    <>
      <span className="flex min-w-0 items-center gap-2">
        {option.page.icon ? (
          <span aria-hidden className="w-4 shrink-0 text-center leading-none">
            {option.page.icon}
          </span>
        ) : (
          <Glyph aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        )}
        <span className="truncate font-medium">
          {option.title.length > 0 ? <Runs runs={option.title} /> : titleOf(option.page)}
        </span>
        <span className="sr-only">, {strings.search.kinds[option.page.kind]}</span>
      </span>
      {option.snippet.length > 0 && (
        <span className="line-clamp-1 pl-6 text-xs text-muted-foreground">
          <Runs runs={option.snippet} />
        </span>
      )}
    </>
  );
}
