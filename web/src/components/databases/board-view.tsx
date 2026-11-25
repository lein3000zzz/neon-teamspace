import { useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  EyeOff,
  Eye,
  MoreHorizontal,
  Plus,
} from "lucide-react";
import { useRef, useState, type PointerEvent } from "react";
import { createPortal } from "react-dom";
import {
  useMoveRow,
  viewRowsQuery,
  type ChangeView,
  type GroupKey,
  type Property,
  type QueryGroup,
  type Row,
  type View,
  type ViewRows,
} from "@/api/databases";
import { hueScope } from "@/components/nodes/hue";
import { Button } from "@/components/ui/button";
import {
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import {
  isEmpty,
  movedCard,
  propertyOf,
  shownColumns,
  valueOf,
  type CardMove,
  type Column,
} from "@/lib/database-rules";
import { edits } from "@/lib/page-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { CellValue } from "./cell-value";
import { useDatabaseScope, useOpenRow } from "./database-scope";
import { scrollNearEdge, startDrag, underPoint, type Point } from "./drag";
import { groupHue, GroupLabel, groupName } from "./group-label";
import { ListView } from "./list-view";
import { RowIcon, RowMenu, TitleField, useRefocus } from "./row-parts";
import type { ViewBodyProps } from "./view-body";

const copy = strings.databases;

type Target = { key: GroupKey; before: string | null };

type Dragging = {
  row: Row;
  from: GroupKey;
  /** Where the card was taken, inside it, and how wide it is, for the copy that follows the pointer. */
  offset: Point;
  width: number;
  point: Point;
  target: Target | null;
};

/**
 * A view as a board: a column per group of its grouping property, a select,
 * a status or a person, in the view's order, each in its option's hue, with
 * the hidden ones folded away at the end. Cards drag between and within
 * columns; their menu moves them from the keyboard and on touch screens.
 */
export function BoardView(props: ViewBodyProps) {
  const { view, answer } = props;
  const { database } = useDatabaseScope();
  const grouping = view.group ? propertyOf(database, view.group.property) : undefined;
  if (grouping === undefined || answer.groups === null) {
    // The grouping property is gone: the rows show ungrouped until another is chosen.
    return <ListView {...props} />;
  }
  return <Board {...props} grouping={grouping} groups={answer.groups} />;
}

type BoardProps = ViewBodyProps & { grouping: Property; groups: readonly QueryGroup[] };

function Board({
  view,
  answer,
  grouping,
  groups,
  editingTitle,
  onEditingTitle,
  onChangeView,
  onNewRow,
}: BoardProps) {
  const workspace = useWorkspace();
  const queryClient = useQueryClient();
  const { database, editsDatabase, people, seesPeople, report } = useDatabaseScope();
  const move = useMoveRow(workspace.id, database.page.id);
  // Guests neither set nor clear people: on a board of people, a card stays in its column.
  const mayRegroup = seesPeople || grouping.type !== "person";
  const [dragging, setDragging] = useState<Dragging | null>(null);
  const [said, setSaid] = useState("");
  const latest = useRef<Dragging | null>(null);
  const board = useRef<HTMLDivElement>(null);
  const shown = groups.filter((group) => !group.hidden);
  const hidden = groups.filter((group) => group.hidden);
  const preview = shownColumns(database, view).filter((column) => column.property.type !== "title");
  const sorted = view.sorts.length > 0;
  const rowsOf = (key: GroupKey) => answer.rows.filter((row) => (row.group ?? null) === key);

  /** Moves a card, shown at once, then saved: its group, and its place among the rows. */
  const commit = (row: Row, cardMove: CardMove) => {
    const regroups = cardMove.from !== cardMove.to;
    const column = rowsOf(cardMove.to).filter((entry) => entry.id !== row.id);
    const last = column.at(-1);
    const placement =
      sorted || !editsDatabase
        ? undefined
        : cardMove.before !== null
          ? { before: cardMove.before }
          : last !== undefined
            ? { after: last.id }
            : undefined;
    if (!regroups) {
      const order = rowsOf(cardMove.from).map((entry) => entry.id);
      const at = order.indexOf(row.id);
      const stays =
        cardMove.before === null ? at === order.length - 1 : order[at + 1] === cardMove.before;
      if (placement === undefined || stays) {
        return;
      }
    }
    queryClient.setQueryData<ViewRows>(
      viewRowsQuery(workspace.id, database.page.id, view.id).queryKey,
      (current) => current && movedCard(current, cardMove, grouping),
    );
    if (regroups) {
      setSaid(copy.board.moved(titleOf(row), groupName(grouping, cardMove.to, people)));
    }
    move.mutate(
      {
        rowId: row.id,
        ...(placement === undefined ? {} : { placement }),
        ...(regroups ? { group: { view_id: view.id, from: cardMove.from, to: cardMove.to } } : {}),
      },
      { onError: report },
    );
  };

  /**
   * Where a card dragged to a point would land: a column, and the card it
   * would go before. Its own column takes it only where it may be placed
   * among the cards, which a view that sorts, or a user who does not edit
   * the database, does not allow; nor does a column of people for a guest.
   */
  const targetAt = (point: Point, rowId: string, from: GroupKey): Target | null => {
    const column = underPoint(point, "boardColumn");
    const index = Number(column?.dataset["boardColumn"]);
    const group = shown[index];
    const own = (group?.key ?? null) === from;
    if (
      column === undefined ||
      group === undefined ||
      (own ? sorted || !editsDatabase : !mayRegroup)
    ) {
      return null;
    }
    const cards = [...column.querySelectorAll<HTMLElement>("[data-card]")].filter(
      (card) => card.dataset["card"] !== rowId,
    );
    const next = cards.find((card) => {
      const box = card.getBoundingClientRect();
      return point.y < box.top + box.height / 2;
    });
    return { key: group.key ?? null, before: next?.dataset["card"] ?? null };
  };

  const startCardDrag = (event: PointerEvent<HTMLElement>, row: Row, from: GroupKey) => {
    if (!edits(row.access) || (event.target as Element).closest("[data-no-drag]") !== null) {
      return;
    }
    const card = event.currentTarget;
    startDrag(event, {
      onStart: (origin) => {
        const box = card.getBoundingClientRect();
        latest.current = {
          row,
          from,
          offset: { x: origin.x - box.left, y: origin.y - box.top },
          width: box.width,
          point: origin,
          target: null,
        };
        setDragging(latest.current);
      },
      onMove: (point) => {
        if (latest.current === null) {
          return;
        }
        latest.current = { ...latest.current, point, target: targetAt(point, row.id, from) };
        setDragging(latest.current);
        scrollNearEdge(board.current, point);
      },
      onDrop: () => {
        const done = latest.current;
        latest.current = null;
        setDragging(null);
        if (done !== null && done.target !== null) {
          commit(done.row, {
            rowId: done.row.id,
            from: done.from,
            to: done.target.key,
            before: done.target.before,
          });
        }
      },
      onCancel: () => {
        latest.current = null;
        setDragging(null);
      },
    });
  };

  const setHidden = (key: GroupKey, hide: boolean) => {
    if (view.group === undefined || view.group === null) {
      return;
    }
    const current = (view.group.hidden ?? []).filter((entry) => entry !== key);
    onChangeView({ group: { ...view.group, hidden: hide ? [...current, key] : current } });
  };

  return (
    <div
      ref={board}
      className="relative flex items-start gap-3 overflow-x-auto overscroll-x-contain pb-3"
    >
      {shown.map((group, index) => {
        const key = group.key ?? null;
        const name = groupName(grouping, key, people);
        const hue = groupHue(grouping, key);
        const cards = rowsOf(key);
        const target = dragging?.target?.key === key ? dragging.target : null;
        return (
          <section
            key={key ?? "none"}
            aria-label={name}
            data-board-column={index}
            {...hueScope(hue)}
            className={cn(
              "grid w-[min(16rem,80vw)] shrink-0 content-start gap-2 rounded-lg p-2",
              hue === undefined ? "bg-muted/60" : "bg-neon-soft",
              target !== null && "outline-2 outline-primary",
            )}
          >
            <header className="flex min-h-(--tap-sm) items-center gap-2 px-1">
              <span className="flex min-w-0 flex-1 items-center gap-2">
                <GroupLabel property={grouping} groupKey={key} people={people} />
                <span className="text-xs text-muted-foreground">{group.count}</span>
              </span>
              {editsDatabase && (
                <GroupMenu
                  name={name}
                  first={index === 0}
                  last={index === shown.length - 1}
                  onHide={() => setHidden(key, true)}
                  onMove={(by) => {
                    const neighbour = shown[index + by];
                    if (neighbour !== undefined) {
                      const order = groups.map((entry) => entry.key ?? null);
                      reorderTo(order, key, neighbour.key ?? null, onChangeView, view);
                    }
                  }}
                />
              )}
            </header>
            <ol className="grid gap-2">
              {cards.map((row, at) => (
                <li key={row.id} className="grid gap-2">
                  {target?.before === row.id && <DropLine />}
                  <Card
                    row={row}
                    group={key}
                    preview={preview}
                    dragged={dragging?.row.id === row.id && dragging.from === key}
                    editingTitle={editingTitle === row.id}
                    onEditingTitle={onEditingTitle}
                    onPointerDown={(event) => startCardDrag(event, row, key)}
                    moves={{
                      groups: shown
                        .filter((other) => mayRegroup && (other.key ?? null) !== key)
                        .map((other) => ({
                          key: other.key ?? null,
                          name: groupName(grouping, other.key ?? null, people),
                        })),
                      up: sorted || at === 0 ? undefined : (cards[at - 1]?.id ?? undefined),
                      down:
                        sorted || at === cards.length - 1 ? undefined : (cards[at + 2]?.id ?? null),
                      onMove: (to, before) => commit(row, { rowId: row.id, from: key, to, before }),
                    }}
                  />
                </li>
              ))}
              {target !== null && target.before === null && (
                <li>
                  <DropLine />
                </li>
              )}
            </ol>
            {editsDatabase && (mayRegroup || key === null) && (
              <Button
                variant="ghost"
                size="sm"
                aria-label={copy.group.addCard(name)}
                onClick={() => onNewRow(key)}
                className="justify-start text-muted-foreground"
              >
                <Plus aria-hidden />
                {copy.toolbar.newRow}
              </Button>
            )}
          </section>
        );
      })}
      {hidden.length > 0 && (
        <section
          aria-label={copy.group.hidden}
          className="grid w-56 shrink-0 content-start gap-1 rounded-lg border border-dashed border-border p-2"
        >
          <h3 className="px-1 text-xs text-muted-foreground">{copy.group.hidden}</h3>
          <ul className="grid gap-0.5">
            {hidden.map((group) => {
              const key = group.key ?? null;
              return (
                <li key={key ?? "none"} className="flex min-h-(--tap-sm) items-center gap-2 px-1">
                  <span className="flex min-w-0 flex-1 items-center gap-2">
                    <GroupLabel property={grouping} groupKey={key} people={people} />
                    <span className="text-xs text-muted-foreground">{group.count}</span>
                  </span>
                  {editsDatabase && (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={copy.group.shown(groupName(grouping, key, people))}
                      onClick={() => setHidden(key, false)}
                    >
                      <Eye aria-hidden />
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        </section>
      )}
      {dragging !== null &&
        createPortal(
          <div
            aria-hidden
            className="pointer-events-none fixed z-50 rotate-1 rounded-md border border-primary bg-card px-3 py-2 text-sm font-medium shadow-glow"
            style={{
              left: dragging.point.x - dragging.offset.x,
              top: dragging.point.y - dragging.offset.y,
              width: dragging.width,
            }}
          >
            {titleOf(dragging.row)}
          </div>,
          document.body,
        )}
      <output className="sr-only" aria-live="polite">
        {dragging === null ? said : copy.board.dragging(titleOf(dragging.row))}
      </output>
    </div>
  );
}

/** The groups' order with one placed where its neighbour is, which a view keeps. */
function reorderTo(
  order: GroupKey[],
  key: GroupKey,
  neighbour: GroupKey,
  onChangeView: (change: ChangeView) => void,
  view: View,
) {
  if (view.group === undefined || view.group === null) {
    return;
  }
  const from = order.indexOf(key);
  const to = order.indexOf(neighbour);
  order.splice(from, 1);
  order.splice(to, 0, key);
  onChangeView({ group: { ...view.group, order } });
}

function DropLine() {
  return <span aria-hidden className="block h-0.5 rounded-full bg-primary shadow-glow" />;
}

type GroupMenuProps = {
  name: string;
  first: boolean;
  last: boolean;
  onHide: () => void;
  onMove: (by: -1 | 1) => void;
};

function GroupMenu({ name, first, last, onHide, onMove }: GroupMenuProps) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={copy.group.menu(name)}
          className="text-muted-foreground"
        >
          <MoreHorizontal aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {!first && (
          <DropdownMenuItem onSelect={() => onMove(-1)}>
            <ArrowLeft aria-hidden />
            {copy.group.moveLeft}
          </DropdownMenuItem>
        )}
        {!last && (
          <DropdownMenuItem onSelect={() => onMove(1)}>
            <ArrowRight aria-hidden />
            {copy.group.moveRight}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem onSelect={onHide}>
          <EyeOff aria-hidden />
          {copy.group.hide}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

type CardMoves = {
  groups: { key: GroupKey; name: string }[];
  /** The card above, to go before; absent when it is first or the view sorts. */
  up: string | undefined;
  /** The card two below, to go before, or null to go last; absent when it is last or the view sorts. */
  down: string | null | undefined;
  onMove: (to: GroupKey, before: string | null) => void;
};

type CardProps = {
  row: Row;
  group: GroupKey;
  preview: readonly Column[];
  dragged: boolean;
  editingTitle: boolean;
  onEditingTitle: (rowId: string | null) => void;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  moves: CardMoves;
};

/** A row as a card: its title, which opens it, the values the view shows, and its menu. */
function Card({
  row,
  group,
  preview,
  dragged,
  editingTitle,
  onEditingTitle,
  onPointerDown,
  moves,
}: CardProps) {
  const { people } = useDatabaseScope();
  const openRow = useOpenRow();
  const { target: titleButton, ended: titleEnded } = useRefocus<HTMLButtonElement>(editingTitle);
  const title = titleOf(row);
  const movable = edits(row.access);
  const movesOffered =
    movable && (moves.groups.length > 0 || moves.up !== undefined || moves.down !== undefined);
  return (
    <article
      data-card={row.id}
      data-row=""
      aria-label={title}
      onPointerDown={onPointerDown}
      className={cn(
        "grid gap-1.5 rounded-md border border-border bg-card p-2 text-sm transition-opacity",
        movable && "cursor-grab active:cursor-grabbing",
        dragged && "opacity-40",
      )}
    >
      <div className="flex items-start gap-1">
        {editingTitle ? (
          <TitleField
            row={row}
            onDone={(fromKeyboard) => {
              titleEnded(fromKeyboard);
              onEditingTitle(null);
            }}
          />
        ) : (
          <button
            ref={titleButton}
            type="button"
            onClick={() => openRow(row.id)}
            className="flex min-h-(--tap-sm) min-w-0 flex-1 items-center gap-1.5 rounded-sm px-1 text-left font-medium outline-offset-[-2px] hover:underline"
          >
            <RowIcon row={row} />
            <span className={cn("break-words", row.title === "" && "text-muted-foreground")}>
              {title}
            </span>
          </button>
        )}
        <span data-row-actions="" className="flex shrink-0 items-center" data-no-drag="">
          <RowMenu row={row}>
            {movesOffered ? (
              <>
                <DropdownMenuLabel>{copy.board.move(title)}</DropdownMenuLabel>
                {moves.groups.map((other) => (
                  <DropdownMenuItem
                    key={other.key ?? "none"}
                    onSelect={() => moves.onMove(other.key, null)}
                  >
                    <ArrowRight aria-hidden />
                    {copy.board.moveTo(other.name)}
                  </DropdownMenuItem>
                ))}
                {moves.up !== undefined && (
                  <DropdownMenuItem
                    onSelect={() => moves.up !== undefined && moves.onMove(group, moves.up)}
                  >
                    <ArrowUp aria-hidden />
                    {copy.board.moveUp}
                  </DropdownMenuItem>
                )}
                {moves.down !== undefined && (
                  <DropdownMenuItem
                    onSelect={() => moves.down !== undefined && moves.onMove(group, moves.down)}
                  >
                    <ArrowDown aria-hidden />
                    {copy.board.moveDown}
                  </DropdownMenuItem>
                )}
              </>
            ) : undefined}
          </RowMenu>
        </span>
      </div>
      {preview.map((column) => {
        const value = valueOf(row, column.property);
        return isEmpty(value) ? null : (
          <div key={column.property.id} className="flex min-w-0 flex-wrap items-center gap-1 px-1">
            <span className="sr-only">{column.property.name}: </span>
            <CellValue property={column.property} value={value} people={people} />
          </div>
        );
      })}
    </article>
  );
}
