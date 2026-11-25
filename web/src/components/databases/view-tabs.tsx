import { cn } from "cn";
import {
  ArrowLeft,
  ArrowRight,
  Copy,
  Kanban,
  List,
  MoreHorizontal,
  Pencil,
  Plus,
  Table2,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import {
  useChangeView,
  useCreateView,
  useDeleteView,
  useDuplicateView,
  useMoveView,
  type CreateView,
  type Layout,
  type View,
} from "@/api/databases";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { groups } from "@/lib/database-rules";
import { strings } from "@/lib/strings";
import { useDatabaseScope } from "./database-scope";

const copy = strings.databases.views;

export const LAYOUT_ICONS: Record<Layout, LucideIcon> = {
  table: Table2,
  board: Kanban,
  list: List,
};

const LAYOUTS: readonly Layout[] = ["table", "board", "list"];

type ViewTabsProps = {
  view: View;
  /** The id of the panel the views show in, which each tab controls. */
  panelId: string;
  onSelect: (viewId: string) => void;
};

/**
 * The database's views as tabs: arrow keys move between them, and the one in
 * view has a menu to rename it, duplicate it, show it in another layout, move
 * it, or delete it, for those who edit the database; then the way to add one.
 */
export function ViewTabs({ view: active, panelId, onSelect }: ViewTabsProps) {
  const workspace = useWorkspace();
  const { database, editsDatabase, report } = useDatabaseScope();
  const create = useCreateView(workspace.id, database.page.id);
  const [renaming, setRenaming] = useState<string | null>(null);
  const tabs = useRef(new Map<string, HTMLButtonElement>());
  const views = database.views;
  const groupable = database.properties.find((property) => groups(property.type));

  const onKeyDown = (event: KeyboardEvent, index: number) => {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    const target =
      event.key === "Home"
        ? views[0]
        : event.key === "End"
          ? views.at(-1)
          : step === 0
            ? undefined
            : views[(index + step + views.length) % views.length];
    if (target !== undefined) {
      event.preventDefault();
      onSelect(target.id);
      tabs.current.get(target.id)?.focus();
    }
  };

  const add = (layout: Layout) => {
    const name = freshViewName(views, copy.layouts[layout]);
    const body: CreateView = {
      name,
      layout,
      ...(layout === "board" && groupable !== undefined
        ? { group: { property: groupable.id } }
        : {}),
    };
    create.mutate(body, { onSuccess: (made) => onSelect(made.id), onError: report });
  };

  return (
    <div className="flex min-w-0 items-center gap-1">
      <div
        role="tablist"
        aria-label={copy.label}
        className="relative flex min-w-0 items-center gap-0.5 overflow-x-auto overscroll-x-contain"
      >
        {views.map((view, index) => {
          const Icon = LAYOUT_ICONS[view.layout];
          const selected = view.id === active.id;
          if (renaming === view.id) {
            return <RenameView key={view.id} view={view} onDone={() => setRenaming(null)} />;
          }
          return (
            <span key={view.id} className="flex shrink-0 items-center">
              <button
                ref={(element) => {
                  if (element === null) {
                    tabs.current.delete(view.id);
                  } else {
                    tabs.current.set(view.id, element);
                  }
                }}
                type="button"
                role="tab"
                id={`${panelId}-${view.id}`}
                aria-selected={selected}
                aria-controls={panelId}
                tabIndex={selected ? 0 : -1}
                onClick={() => onSelect(view.id)}
                onKeyDown={(event) => onKeyDown(event, index)}
                className={cn(
                  "inline-flex h-(--tap-sm) items-center gap-1.5 rounded-md px-2.5 text-sm whitespace-nowrap outline-offset-[-2px] transition-colors",
                  selected
                    ? "bg-accent font-medium text-foreground shadow-[inset_0_-2px_0_var(--primary)]"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground",
                )}
              >
                <Icon aria-hidden className="size-4" />
                {view.name}
              </button>
              {selected && editsDatabase && (
                <ViewMenu
                  view={view}
                  index={index}
                  onRename={() => setRenaming(view.id)}
                  onSelect={onSelect}
                />
              )}
            </span>
          );
        })}
      </div>
      {editsDatabase && (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={copy.add}
              disabled={create.isPending}
            >
              <Plus aria-hidden />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            {LAYOUTS.map((layout) => {
              const Icon = LAYOUT_ICONS[layout];
              const blocked = layout === "board" && groupable === undefined;
              return (
                <DropdownMenuItem key={layout} disabled={blocked} onSelect={() => add(layout)}>
                  <Icon aria-hidden />
                  {copy.layouts[layout]}
                </DropdownMenuItem>
              );
            })}
            {groupable === undefined && (
              <p className="px-2.5 py-1.5 text-xs text-muted-foreground">{copy.noBoard}</p>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}

/** A name for a new view no other has: "Board", then "Board 2". */
function freshViewName(views: readonly View[], base: string): string {
  const taken = new Set(views.map((view) => view.name.toLowerCase()));
  if (!taken.has(base.toLowerCase())) {
    return base;
  }
  for (let n = 2; ; n += 1) {
    if (!taken.has(`${base} ${n}`.toLowerCase())) {
      return `${base} ${n}`;
    }
  }
}

type ViewMenuProps = {
  view: View;
  index: number;
  onRename: () => void;
  onSelect: (viewId: string) => void;
};

function ViewMenu({ view, index, onRename, onSelect }: ViewMenuProps) {
  const workspace = useWorkspace();
  const { database, report } = useDatabaseScope();
  const change = useChangeView(workspace.id, database.page.id);
  const duplicate = useDuplicateView(workspace.id, database.page.id);
  const move = useMoveView(workspace.id, database.page.id);
  const remove = useDeleteView(workspace.id, database.page.id);
  const [confirming, setConfirming] = useState(false);
  const views = database.views;
  const before = views[index - 1];
  const after = views[index + 1];
  const groupable = database.properties.find((property) => groups(property.type));

  const relayout = (layout: Layout) => {
    const needsGroup = layout === "board" && (view.group ?? null) === null;
    if (needsGroup && groupable === undefined) {
      return;
    }
    change.mutate(
      {
        viewId: view.id,
        layout,
        ...(needsGroup && groupable !== undefined ? { group: { property: groupable.id } } : {}),
      },
      { onError: report },
    );
  };

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={copy.menu(view.name)}>
            <MoreHorizontal aria-hidden />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={onRename}>
            <Pencil aria-hidden />
            {copy.rename}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() =>
              duplicate.mutate(view.id, { onSuccess: (made) => onSelect(made.id), onError: report })
            }
          >
            <Copy aria-hidden />
            {copy.duplicate}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {LAYOUTS.filter((layout) => layout !== view.layout).map((layout) => {
            const Icon = LAYOUT_ICONS[layout];
            return (
              <DropdownMenuItem
                key={layout}
                disabled={
                  layout === "board" && (view.group ?? null) === null && groupable === undefined
                }
                onSelect={() => relayout(layout)}
              >
                <Icon aria-hidden />
                {copy.layout(copy.layouts[layout].toLowerCase())}
              </DropdownMenuItem>
            );
          })}
          <DropdownMenuSeparator />
          {before !== undefined && (
            <DropdownMenuItem
              onSelect={() =>
                move.mutate(
                  { viewId: view.id, placement: { before: before.id } },
                  { onError: report },
                )
              }
            >
              <ArrowLeft aria-hidden />
              {copy.moveLeft}
            </DropdownMenuItem>
          )}
          {after !== undefined && (
            <DropdownMenuItem
              onSelect={() =>
                move.mutate(
                  { viewId: view.id, placement: { after: after.id } },
                  { onError: report },
                )
              }
            >
              <ArrowRight aria-hidden />
              {copy.moveRight}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem disabled={views.length <= 1} onSelect={() => setConfirming(true)}>
            <Trash2 aria-hidden />
            {copy.delete}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ConfirmDialog
        open={confirming}
        title={copy.deleteTitle(view.name)}
        description={copy.deleteLede}
        confirm={copy.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          const next = before ?? after;
          remove.mutate(view.id, {
            onSuccess: () => next !== undefined && onSelect(next.id),
            onError: report,
          });
        }}
      />
    </>
  );
}

/** A view's name in place of its tab: Enter or leaving it saves, Escape keeps the old one. */
function RenameView({ view, onDone }: { view: View; onDone: () => void }) {
  const workspace = useWorkspace();
  const { database, report } = useDatabaseScope();
  const change = useChangeView(workspace.id, database.page.id);
  const [name, setName] = useState(view.name);
  const field = useRef<HTMLInputElement>(null);
  const finished = useRef(false);

  useEffect(() => {
    field.current?.focus();
    field.current?.select();
  }, []);

  const finish = (save: boolean) => {
    if (finished.current) {
      return;
    }
    finished.current = true;
    const next = name.trim();
    if (save && next !== "" && next !== view.name) {
      change.mutate({ viewId: view.id, name: next }, { onError: report });
    }
    onDone();
  };

  return (
    <input
      ref={field}
      aria-label={copy.renameTitle(view.name)}
      maxLength={100}
      value={name}
      onChange={(event) => setName(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      className="h-(--tap-sm) w-40 shrink-0 rounded-md border border-primary bg-background px-2 text-base md:text-sm"
    />
  );
}
