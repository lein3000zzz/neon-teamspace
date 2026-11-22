import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { cn } from "cn";
import { ChevronRight, FileText, Plus } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import {
  showPage,
  useMovePage,
  useUpdatePage,
  type PagePlace,
  type Placement,
  type TreePage,
} from "@/api/pages";
import type { WorkspaceSummary } from "@/api/workspaces";
import { Button } from "@/components/ui/button";
import { edits } from "@/lib/page-rules";
import { forestOf, siblingsOf, subtreeOf, titleOf, type PageForest } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { PageMenu } from "./page-menu";
import { usePageNav } from "./page-nav-state";
import { besideOf, topOf, type Space } from "./places";
import { useNewPage } from "./use-new-page";

const copy = strings.pages.tree;

/**
 * A row of the sidebar: the line before or after it lights up, or the whole
 * row, while a dragged page would land there; its actions show on hover or
 * focus where every pointer hovers, and always on touch screens (index.css).
 */
export const treeRow =
  "relative flex min-h-(--tap) items-center gap-0.5 rounded-md pr-0.5 text-sm text-muted-foreground transition-colors before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:rounded-full before:bg-primary before:opacity-0 before:shadow-glow hover:bg-accent hover:text-foreground has-[[aria-current=page]]:bg-accent has-[[aria-current=page]]:text-foreground has-[[aria-current=page]]:before:opacity-100 data-[drop=after]:shadow-[inset_0_-2px_0_var(--primary)] data-[drop=before]:shadow-[inset_0_2px_0_var(--primary)] data-[drop=inside]:bg-neon-soft data-[drop=inside]:text-foreground";

/** The link inside a row, which takes the rest of its width. */
export const treeLink =
  "flex min-w-0 flex-1 items-center gap-2 self-stretch rounded-md px-1.5 outline-offset-[-2px]";

type Drop = "before" | "after" | "inside";

type PageTreeProps = {
  workspace: WorkspaceSummary;
  space: Space;
  pages: readonly TreePage[];
  label: string;
  onNavigate?: (() => void) | undefined;
  /** How far in the tree starts, under a teamspace's row. */
  depth?: number;
};

/**
 * A tree of pages: each with the way to open and close the pages under it, to
 * add one there, and its menu. Pages the user edits drag onto, before or after
 * another, and their menu moves them up, down or anywhere for the keyboard and
 * for touch screens, where dragging is not offered.
 */
export function PageTree({ workspace, space, pages, label, onNavigate, depth = 0 }: PageTreeProps) {
  const forest = useMemo(() => forestOf(pages), [pages]);
  return (
    <ul aria-label={label} className="grid gap-px">
      {forest.roots.map((page) => (
        <TreeRow
          key={page.id}
          workspace={workspace}
          space={space}
          forest={forest}
          page={page}
          depth={depth}
          onNavigate={onNavigate}
        />
      ))}
    </ul>
  );
}

type TreeRowProps = {
  workspace: WorkspaceSummary;
  space: Space;
  forest: PageForest;
  page: TreePage;
  depth: number;
  onNavigate?: (() => void) | undefined;
};

function TreeRow({ workspace, space, forest, page, depth, onNavigate }: TreeRowProps) {
  const nav = usePageNav();
  const move = useMovePage(workspace.id);
  const newPage = useNewPage(workspace, onNavigate);
  const [renaming, setRenaming] = useState(false);
  const [drop, setDrop] = useState<Drop | null>(null);
  const link = useRef<HTMLAnchorElement>(null);
  // Back on the link once a rename ends from the keyboard.
  const refocus = useRef(false);
  const title = titleOf(page);
  const children = forest.children.get(page.id) ?? [];
  const expandable = children.length > 0;
  const open = expandable && nav.isOpen(page.id);
  const editor = edits(page.access);
  const beside = besideOf(space, page, forest);
  const siblings = siblingsOf(forest, page);
  const index = siblings.findIndex((sibling) => sibling.id === page.id);
  const before = siblings[index - 1];
  const after = siblings[index + 1];
  const report = (error: unknown) => nav.setNotice({ tone: "error", error });

  useEffect(() => {
    if (refocus.current && !renaming) {
      refocus.current = false;
      link.current?.focus();
    }
  }, [renaming]);

  const moveTo = (place: PagePlace, placement: Placement) =>
    move.mutate({ id: page.id, ...place, placement }, { onError: report });

  const zoneOf = (event: DragEvent<HTMLElement>): Drop | null => {
    const dragged = nav.dragged;
    if (dragged === null || dragged.blocked.has(page.id)) {
      return null;
    }
    const box = event.currentTarget.getBoundingClientRect();
    const at = box.height > 0 ? (event.clientY - box.top) / box.height : 0.5;
    if (beside !== undefined && at < 0.25) {
      return "before";
    }
    if (beside !== undefined && at > 0.75) {
      return "after";
    }
    if (editor) {
      return "inside";
    }
    return beside === undefined ? null : at < 0.5 ? "before" : "after";
  };

  const dropHere = (event: DragEvent<HTMLElement>) => {
    const zone = zoneOf(event);
    const dragged = nav.dragged;
    setDrop(null);
    nav.setDragged(null);
    if (zone === null || dragged === null) {
      return;
    }
    event.preventDefault();
    if (zone === "inside") {
      nav.setOpen(page.id, true);
      move.mutate({ id: dragged.id, parent_id: page.id, placement: "last" }, { onError: report });
    } else if (beside !== undefined) {
      move.mutate(
        {
          id: dragged.id,
          ...beside,
          placement: zone === "before" ? { before: page.id } : { after: page.id },
        },
        { onError: report },
      );
    }
  };

  return (
    <li>
      <div
        data-row=""
        {...(drop === null ? {} : { "data-drop": drop })}
        draggable={editor && !renaming}
        onDragStart={(event) => {
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", title);
          nav.setDragged({ id: page.id, title, blocked: subtreeOf(forest, page.id) });
        }}
        onDragEnd={() => {
          setDrop(null);
          nav.setDragged(null);
        }}
        onDragOver={(event) => {
          const zone = zoneOf(event);
          if (zone !== null) {
            event.preventDefault();
            event.dataTransfer.dropEffect = "move";
          }
          setDrop(zone);
        }}
        onDragLeave={() => setDrop(null)}
        onDrop={dropHere}
        className={treeRow}
        style={{ paddingLeft: `${depth * 0.75}rem` }}
      >
        <Disclosure
          expandable={expandable}
          open={open}
          title={title}
          onToggle={() => nav.setOpen(page.id, !open)}
        />
        {renaming ? (
          <RenameField
            workspace={workspace}
            page={page}
            onDone={(fromKeyboard) => {
              refocus.current = fromKeyboard;
              setRenaming(false);
            }}
          />
        ) : (
          <Link
            ref={link}
            to="/w/$slug/p/$pageId"
            params={{ slug: workspace.slug, pageId: page.id }}
            onClick={onNavigate}
            draggable={false}
            className={treeLink}
          >
            <PageGlyph icon={page.icon} />
            <span className="min-w-0 flex-1 truncate">{title}</span>
          </Link>
        )}
        <div data-row-actions="" className="flex shrink-0 items-center">
          <PageMenu
            workspace={workspace}
            page={page}
            onRename={() => setRenaming(true)}
            onAddInside={() => newPage.start({ parent_id: page.id })}
            onMoveUp={
              beside !== undefined && before !== undefined
                ? () => moveTo(beside, { before: before.id })
                : undefined
            }
            onMoveDown={
              beside !== undefined && after !== undefined
                ? () => moveTo(beside, { after: after.id })
                : undefined
            }
          />
          {editor && (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={copy.add(title)}
              disabled={newPage.pending}
              onClick={() => newPage.start({ parent_id: page.id })}
            >
              <Plus aria-hidden />
            </Button>
          )}
        </div>
      </div>
      {open && (
        <ul className="grid gap-px">
          {children.map((child) => (
            <TreeRow
              key={child.id}
              workspace={workspace}
              space={space}
              forest={forest}
              page={child}
              depth={depth + 1}
              onNavigate={onNavigate}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

type DisclosureProps = {
  expandable: boolean;
  open: boolean;
  title: string;
  onToggle: () => void;
};

/** Opens and closes the pages under a row; a row with none keeps the space, for alignment. */
export function Disclosure({ expandable, open, title, onToggle }: DisclosureProps) {
  if (!expandable) {
    return <span aria-hidden className="size-(--tap-sm) shrink-0" />;
  }
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-expanded={open}
      aria-label={open ? copy.collapse(title) : copy.expand(title)}
      onClick={onToggle}
      className="shrink-0 text-muted-foreground"
    >
      <ChevronRight aria-hidden className={cn("transition-transform", open && "rotate-90")} />
    </Button>
  );
}

/** A page's icon in a list, or a page glyph without one. */
export function PageGlyph({ icon }: { icon?: string | null | undefined }) {
  const text = icon?.trim() ?? "";
  if (text === "") {
    return <FileText aria-hidden className="size-4 shrink-0" />;
  }
  return (
    <span aria-hidden className="grid w-4 shrink-0 place-items-center overflow-hidden leading-none">
      {text}
    </span>
  );
}

type RenameFieldProps = {
  workspace: WorkspaceSummary;
  page: TreePage;
  /** Told whether Enter or Escape ended it, rather than leaving the field. */
  onDone: (fromKeyboard: boolean) => void;
};

/** The title in place of the link: Enter or leaving it saves, Escape puts the old one back. */
function RenameField({ workspace, page, onDone }: RenameFieldProps) {
  const queryClient = useQueryClient();
  const nav = usePageNav();
  const update = useUpdatePage(workspace.id, page.id);
  const [value, setValue] = useState(page.title);
  const field = useRef<HTMLInputElement>(null);
  // Enter or Escape ends it, and the field then leaves the page, which blurs it.
  const finished = useRef(false);

  // Opened from the menu on purpose, to type at once.
  useEffect(() => field.current?.focus(), []);

  const finish = (save: boolean, fromKeyboard: boolean) => {
    if (finished.current) {
      return;
    }
    finished.current = true;
    if (save && value !== page.title) {
      showPage(queryClient, workspace.id, page.id, { title: value });
      update.mutate(
        { title: value },
        {
          onError: (error) => {
            showPage(queryClient, workspace.id, page.id, { title: page.title });
            nav.setNotice({ tone: "error", error });
          },
        },
      );
    }
    onDone(fromKeyboard);
  };

  return (
    <input
      ref={field}
      aria-label={copy.rename(titleOf(page))}
      maxLength={500}
      value={value}
      onChange={(event) => setValue(event.target.value.replaceAll(/[\r\n]/g, " "))}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true, true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          finish(false, true);
        }
      }}
      onBlur={() => finish(true, false)}
      className="h-(--tap-sm) min-w-0 flex-1 rounded-md border border-primary bg-background px-1.5 text-base text-foreground md:text-sm"
    />
  );
}

/**
 * The handlers that make a section's heading, such as a teamspace's row, take
 * a dragged page to the top of its space, last there.
 */
export function useSpaceDrop(workspace: WorkspaceSummary, space: Space) {
  const nav = usePageNav();
  const move = useMovePage(workspace.id);
  const [over, setOver] = useState(false);
  const place = topOf(space);
  if (place === undefined) {
    return { over: false, handlers: {} };
  }
  return {
    over,
    handlers: {
      onDragOver: (event: DragEvent<HTMLElement>) => {
        if (nav.dragged !== null) {
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
          setOver(true);
        }
      },
      onDragLeave: () => setOver(false),
      onDrop: (event: DragEvent<HTMLElement>) => {
        const dragged = nav.dragged;
        setOver(false);
        nav.setDragged(null);
        if (dragged === null) {
          return;
        }
        event.preventDefault();
        move.mutate(
          { id: dragged.id, ...place, placement: "last" },
          { onError: (error) => nav.setNotice({ tone: "error", error }) },
        );
      },
    },
  };
}
