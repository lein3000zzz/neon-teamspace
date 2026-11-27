import { cn } from "cn";
import {
  ArrowDownWideNarrow,
  ArrowLeft,
  ArrowRight,
  ArrowUpNarrowWide,
  BetweenHorizontalStart,
  EyeOff,
  ListFilter,
  PanelRightOpen,
  Pencil,
  Plus,
  Shapes,
  Trash2,
} from "lucide-react";
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  useDeleteProperty,
  type ChangeView,
  type Property,
  type Row,
  type View,
} from "@/api/databases";
import type { Placement } from "@/api/pages";
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
import {
  clampWidth,
  columnsOf,
  movedColumn,
  propertyOf,
  retypes,
  shownColumns,
  viewProperties,
  widthOf,
  withColumn,
  MAX_WIDTH,
  MIN_WIDTH,
  type Column,
} from "@/lib/database-rules";
import { edits } from "@/lib/page-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { Cell } from "./cell";
import { useDatabaseScope, useOpenRow } from "./database-scope";
import { startDrag, underPoint } from "./drag";
import { GroupLabel } from "./group-label";
import { PropertyDialog } from "./property-dialog";
import { PropertyIcon } from "./property-icon";
import { RowIcon, RowMenu, TitleField, useRefocus } from "./row-parts";
import { entriesOf, entryKey, entryOfRow, type ViewBodyProps } from "./view-body";
import { useWindowed } from "./windowed";

const copy = strings.databases;

/**
 * The column after the last, which holds the way to add a property, and takes
 * what is left of a table narrower than its place.
 */
const ADD_WIDTH = 48;

/** A table's row before it is measured: a cell's height and its border. */
const ROW_ESTIMATE = 37;

/**
 * A view as a table: a column per property it shows, as wide as the view
 * keeps it, the title first and held in place as the rest scroll sideways;
 * each cell edits in place, and a value cut short tells itself in full on
 * hover. Columns reorder by dragging their heading, or from its menu, and
 * resize from their edge with the pointer or the arrow keys. Only the rows
 * near the screen are drawn, so thousands stay quick.
 */
export function TableView({
  view,
  answer,
  editingTitle,
  onEditingTitle,
  onChangeView,
}: ViewBodyProps) {
  const { database, editsDatabase, people } = useDatabaseScope();
  const columns = useMemo(() => shownColumns(database, view), [database, view]);
  const [widths, setWidths] = useState<Record<string, number>>({});
  const [moving, setMoving] = useState<{
    id: string;
    target: string;
    side: "before" | "after";
  } | null>(null);
  const grouping = view.group ? propertyOf(database, view.group.property) : undefined;
  const width = (column: Column) => widths[column.property.id] ?? widthOf(column);
  const entries = useMemo(() => entriesOf(answer, grouping !== undefined), [answer, grouping]);
  const keys = useMemo(() => entries.map(entryKey), [entries]);
  const { list, items, before, after, measure, scrollTo } = useWindowed<HTMLTableSectionElement>(
    keys,
    ROW_ESTIMATE,
  );
  const span = columns.length + 1;
  // Laid out fixed, the columns are as wide as they say whatever they hold.
  const total = columns.reduce((sum, column) => sum + width(column), ADD_WIDTH);

  // A row whose title is to be typed, as a new one, is drawn wherever it is.
  useEffect(() => {
    if (editingTitle !== null) {
      const at = entryOfRow(entries, editingTitle);
      if (at >= 0) {
        scrollTo(at);
      }
    }
  }, [editingTitle, entries, scrollTo]);

  const saveColumns = (next: Column[]) => onChangeView({ properties: viewProperties(next) });

  const resized = (propertyId: string, next: number | undefined) =>
    setWidths((current) => {
      const copied = { ...current };
      if (next === undefined) {
        delete copied[propertyId];
      } else {
        copied[propertyId] = next;
      }
      return copied;
    });

  return (
    <div className="relative overflow-x-auto overscroll-x-contain">
      <table
        aria-label={view.name}
        aria-rowcount={entries.length + 1}
        style={{ width: `max(100%, ${total}px)` }}
        className="table-fixed border-separate border-spacing-0 text-sm"
      >
        <colgroup>
          {columns.map((column) => (
            <col key={column.property.id} style={{ width: width(column) }} />
          ))}
          <col />
        </colgroup>
        <thead>
          <tr aria-rowindex={1}>
            {columns.map((column, index) => (
              <ColumnHeader
                key={column.property.id}
                view={view}
                column={column}
                index={index}
                columns={columns}
                width={width(column)}
                drop={moving?.target === column.property.id ? moving.side : undefined}
                onMoving={setMoving}
                onResize={(next) => resized(column.property.id, next)}
                onSaveWidth={(next) => {
                  saveColumns(
                    withColumn(columnsOf(database, view), column.property.id, { width: next }),
                  );
                  resized(column.property.id, undefined);
                }}
                onReorder={(target, side) =>
                  saveColumns(
                    movedColumn(columnsOf(database, view), column.property.id, target, side),
                  )
                }
                onChangeView={onChangeView}
              />
            ))}
            {editsDatabase ? (
              <th scope="col" className="border-b border-border bg-background p-0 text-left">
                <AddPropertyButton view={view} onChangeView={onChangeView} />
              </th>
            ) : (
              <td aria-hidden className="border-b border-border bg-background" />
            )}
          </tr>
        </thead>
        <tbody ref={list}>
          {before > 0 && <Space height={before} span={span} />}
          {items.map((item) => {
            const entry = entries[item.index];
            if (entry === undefined) {
              return null;
            }
            return entry.kind === "group" ? (
              <tr
                key={item.key}
                ref={measure}
                data-index={item.index}
                aria-rowindex={item.index + 2}
              >
                <th
                  scope="colgroup"
                  colSpan={span}
                  className="border-b border-border bg-background px-2 pt-4 pb-1.5 text-left font-normal"
                >
                  {grouping !== undefined && (
                    <span className="sticky left-2 inline-flex items-center gap-2">
                      <GroupLabel property={grouping} groupKey={entry.key} people={people} />
                      <span className="text-xs text-muted-foreground">
                        {copy.group.count(entry.count)}
                      </span>
                    </span>
                  )}
                </th>
              </tr>
            ) : (
              <TableRow
                key={item.key}
                index={item.index}
                measure={measure}
                row={entry.row}
                columns={columns}
                editingTitle={editingTitle === entry.row.id}
                onEditingTitle={onEditingTitle}
              />
            );
          })}
          {after > 0 && <Space height={after} span={span} />}
        </tbody>
      </table>
    </div>
  );
}

/** The height of rows not drawn, above or below those that are. */
function Space({ height, span }: { height: number; span: number }) {
  return (
    <tr aria-hidden>
      <td aria-hidden colSpan={span} style={{ height, padding: 0, border: 0 }} />
    </tr>
  );
}

type TableRowProps = {
  /** Its place among the lines of the table's body. */
  index: number;
  /** Measures it as it is drawn. */
  measure: (element: Element | null) => void;
  row: Row;
  columns: readonly Column[];
  editingTitle: boolean;
  onEditingTitle: (rowId: string | null) => void;
};

/** A row of the table, drawn again only when it, or what the table shows of it, changes. */
const TableRow = memo(function TableRow({
  index,
  measure,
  row,
  columns,
  editingTitle,
  onEditingTitle,
}: TableRowProps) {
  const { database } = useDatabaseScope();
  const openRow = useOpenRow();
  const { target: renameButton, ended: titleEnded } = useRefocus<HTMLButtonElement>(editingTitle);
  const title = titleOf(row);
  const trashed = (database.page.trashed_at ?? null) !== null;
  return (
    <tr
      ref={measure}
      data-index={index}
      aria-rowindex={index + 2}
      data-row=""
      className="group/row"
    >
      {columns.map((column, at) =>
        at === 0 ? (
          <th
            key={column.property.id}
            scope="row"
            className="sticky left-0 z-[1] border-r border-b border-border bg-background p-0 text-left font-medium"
          >
            {editingTitle ? (
              <TitleField
                row={row}
                onDone={(fromKeyboard) => {
                  titleEnded(fromKeyboard);
                  onEditingTitle(null);
                }}
              />
            ) : (
              <span className="flex min-h-(--tap-sm) min-w-0 items-center gap-0.5 pr-0.5">
                <button
                  ref={renameButton}
                  type="button"
                  disabled={!edits(row.access) || trashed}
                  onClick={() => onEditingTitle(row.id)}
                  aria-label={copy.rows.rename(title)}
                  className="flex min-h-(--tap-sm) min-w-0 flex-1 items-center gap-1.5 rounded-sm px-2 text-left outline-offset-[-2px] hover:bg-accent disabled:cursor-default disabled:hover:bg-transparent"
                >
                  <RowIcon row={row} />
                  <span
                    title={title}
                    className={cn("truncate", row.title === "" && "text-muted-foreground")}
                  >
                    {title}
                  </span>
                </button>
                <span data-row-actions="" className="flex shrink-0 items-center">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={copy.rows.open(title)}
                    onClick={() => openRow(row.id)}
                  >
                    <PanelRightOpen aria-hidden />
                  </Button>
                  <RowMenu row={row} />
                </span>
              </span>
            )}
          </th>
        ) : (
          <td key={column.property.id} className="border-r border-b border-border p-0 align-middle">
            <Cell property={column.property} row={row} variant="table" />
          </td>
        ),
      )}
      <td aria-hidden className="border-b border-border" />
    </tr>
  );
});

type ColumnHeaderProps = {
  view: View;
  column: Column;
  index: number;
  columns: readonly Column[];
  width: number;
  /** Which side of it a dragged column would land on. */
  drop: "before" | "after" | undefined;
  onMoving: (moving: { id: string; target: string; side: "before" | "after" } | null) => void;
  onResize: (width: number | undefined) => void;
  onSaveWidth: (width: number) => void;
  onReorder: (target: string, side: "before" | "after") => void;
  onChangeView: (change: ChangeView) => void;
};

/**
 * A column's heading: its property's name and type, which opens its menu,
 * drags to reorder the columns, and has an edge that resizes it.
 */
function ColumnHeader({
  view,
  column,
  index,
  columns,
  width,
  drop,
  onMoving,
  onResize,
  onSaveWidth,
  onReorder,
  onChangeView,
}: ColumnHeaderProps) {
  const { editsDatabase } = useDatabaseScope();
  const { property } = column;
  const [open, setOpen] = useState(false);
  // A press on the heading while its menu is open closes the menu before the click lands.
  const openAtPress = useRef(false);
  const target = useRef<{ id: string; side: "before" | "after" } | null>(null);
  const sticky = index === 0;
  const name = (
    <>
      <PropertyIcon kind={property.type} className="text-muted-foreground" />
      <span title={property.name} className="truncate">
        {property.name}
      </span>
    </>
  );
  return (
    <th
      scope="col"
      data-column={property.id}
      className={cn(
        "relative border-r border-b border-border bg-background p-0 text-left font-normal text-muted-foreground",
        sticky && "sticky left-0 z-[2]",
        drop === "before" && "shadow-[inset_2px_0_0_var(--primary)]",
        drop === "after" && "shadow-[inset_-2px_0_0_var(--primary)]",
      )}
    >
      {editsDatabase ? (
        <ColumnMenu
          view={view}
          column={column}
          index={index}
          columns={columns}
          open={open}
          onOpenChange={setOpen}
          onChangeView={onChangeView}
          trigger={
            <button
              type="button"
              aria-label={copy.properties.menu(property.name)}
              onPointerDown={(event) => {
                openAtPress.current = open;
                if (sticky) {
                  return;
                }
                startDrag(event, {
                  onMove: (point) => {
                    const over = underPoint(point, "column");
                    const id = over?.dataset["column"];
                    if (
                      over === undefined ||
                      id === undefined ||
                      id === property.id ||
                      id === columns[0]?.property.id
                    ) {
                      target.current = null;
                      onMoving(null);
                      return;
                    }
                    const box = over.getBoundingClientRect();
                    const side = point.x < box.left + box.width / 2 ? "before" : "after";
                    target.current = { id, side };
                    onMoving({ id: property.id, target: id, side });
                  },
                  onDrop: () => {
                    onMoving(null);
                    if (target.current !== null) {
                      onReorder(target.current.id, target.current.side);
                    }
                    target.current = null;
                  },
                  onCancel: () => {
                    target.current = null;
                    onMoving(null);
                  },
                });
              }}
              onClick={() => setOpen(!openAtPress.current)}
              className="flex h-(--tap-sm) w-full min-w-0 items-center gap-1.5 px-2 text-left outline-offset-[-2px] hover:bg-accent aria-expanded:bg-accent"
            >
              {name}
            </button>
          }
        />
      ) : (
        <span className="flex h-(--tap-sm) min-w-0 items-center gap-1.5 px-2">{name}</span>
      )}
      {editsDatabase && (
        <Resizer name={property.name} width={width} onResize={onResize} onSave={onSaveWidth} />
      )}
    </th>
  );
}

type ResizerProps = {
  name: string;
  width: number;
  onResize: (width: number | undefined) => void;
  onSave: (width: number) => void;
};

/** How far a column's width steps with the arrow keys: the widths it starts at are steps. */
const WIDTH_STEP = 20;

/**
 * How wide a column is, at its edge: dragged with the pointer, or, once
 * focused, a slider stepped with the arrow keys. Saved once it settles.
 */
function Resizer({ name, width, onResize, onSave }: ResizerProps) {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  return (
    <input
      type="range"
      aria-label={copy.properties.resize(name)}
      min={MIN_WIDTH}
      max={MAX_WIDTH}
      step={WIDTH_STEP}
      value={width}
      onChange={(event) => {
        const next = clampWidth(Number(event.target.value));
        onResize(next);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => onSave(next), 400);
      }}
      onPointerDown={(event) => {
        const start = width;
        let latest = start;
        startDrag(
          event,
          {
            onMove: (point, origin) => {
              latest = clampWidth(start + point.x - origin.x);
              onResize(latest);
            },
            onDrop: () => onSave(latest),
            onCancel: () => onResize(undefined),
          },
          1,
        );
      }}
      className="absolute inset-y-0 -right-1 z-[3] m-0 h-full w-2 cursor-col-resize appearance-none bg-transparent opacity-0 hover:bg-primary/40 hover:opacity-100 focus-visible:bg-primary focus-visible:opacity-100"
    />
  );
}

type ColumnMenuProps = {
  view: View;
  column: Column;
  index: number;
  columns: readonly Column[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChangeView: (change: ChangeView) => void;
  trigger: ReactNode;
};

type Dialog =
  | { kind: "edit"; focus: "name" | "type" }
  | { kind: "insert"; side: "before" | "after" }
  | { kind: "delete" }
  | null;

/**
 * Everything to do with a column: rename it or change its type, sort or
 * filter the view by it, hide it, add a property beside it, move it, or
 * delete its property. The title can be neither hidden nor deleted.
 */
function ColumnMenu({
  view,
  column,
  index,
  columns,
  open,
  onOpenChange,
  onChangeView,
  trigger,
}: ColumnMenuProps) {
  const workspace = useWorkspace();
  const { database, seesPeople, report, startFilter } = useDatabaseScope();
  const remove = useDeleteProperty(workspace.id, database.page.id);
  const [dialog, setDialog] = useState<Dialog>(null);
  const { property } = column;
  const title = property.type === "title";
  const before = columns[index - 1];
  const after = columns[index + 1];
  const all = columnsOf(database, view);

  const sortBy = (direction: "ascending" | "descending") =>
    onChangeView({
      sorts: [
        { property: property.id, direction },
        ...view.sorts.filter((sort) => sort.property !== property.id),
      ].slice(0, 10),
    });

  const inserted = (made: Property, side: "before" | "after") => {
    const next = movedColumn(
      [...all, { property: made, visible: true }],
      made.id,
      property.id,
      side,
    );
    onChangeView({ properties: viewProperties(next) });
  };
  const placement = (side: "before" | "after"): Placement =>
    side === "before" ? { before: property.id } : { after: property.id };

  return (
    <>
      <DropdownMenu open={open} onOpenChange={onOpenChange} modal={false}>
        <DropdownMenuTrigger asChild onPointerDown={(event) => event.preventDefault()}>
          {trigger}
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => setDialog({ kind: "edit", focus: "name" })}>
            <Pencil aria-hidden />
            {copy.properties.rename}
          </DropdownMenuItem>
          {retypes(property, seesPeople) && (
            <DropdownMenuItem onSelect={() => setDialog({ kind: "edit", focus: "type" })}>
              <Shapes aria-hidden />
              {copy.properties.changeType}
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => sortBy("ascending")}>
            <ArrowUpNarrowWide aria-hidden />
            {copy.properties.sortAscending}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => sortBy("descending")}>
            <ArrowDownWideNarrow aria-hidden />
            {copy.properties.sortDescending}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => startFilter(property)}>
            <ListFilter aria-hidden />
            {copy.properties.filter}
          </DropdownMenuItem>
          {!title && (
            <DropdownMenuItem
              onSelect={() =>
                onChangeView({
                  properties: viewProperties(withColumn(all, property.id, { visible: false })),
                })
              }
            >
              <EyeOff aria-hidden />
              {copy.properties.hide}
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          {!title && (
            <DropdownMenuItem onSelect={() => setDialog({ kind: "insert", side: "before" })}>
              <BetweenHorizontalStart aria-hidden className="rotate-90" />
              {copy.properties.insertLeft}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => setDialog({ kind: "insert", side: "after" })}>
            <BetweenHorizontalStart aria-hidden className="-rotate-90" />
            {copy.properties.insertRight}
          </DropdownMenuItem>
          {!title && before !== undefined && index > 1 && (
            <DropdownMenuItem
              onSelect={() =>
                onChangeView({
                  properties: viewProperties(
                    movedColumn(all, property.id, before.property.id, "before"),
                  ),
                })
              }
            >
              <ArrowLeft aria-hidden />
              {copy.properties.moveLeft}
            </DropdownMenuItem>
          )}
          {!title && after !== undefined && (
            <DropdownMenuItem
              onSelect={() =>
                onChangeView({
                  properties: viewProperties(
                    movedColumn(all, property.id, after.property.id, "after"),
                  ),
                })
              }
            >
              <ArrowRight aria-hidden />
              {copy.properties.moveRight}
            </DropdownMenuItem>
          )}
          {!title && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => setDialog({ kind: "delete" })}>
                <Trash2 aria-hidden />
                {copy.properties.delete}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {dialog?.kind === "edit" && (
        <PropertyDialog
          open
          onOpenChange={(next) => !next && setDialog(null)}
          property={property}
          focus={dialog.focus}
        />
      )}
      {dialog?.kind === "insert" && (
        <PropertyDialog
          open
          onOpenChange={(next) => !next && setDialog(null)}
          placement={placement(dialog.side)}
          onAdded={(made) => inserted(made, dialog.side)}
        />
      )}
      <ConfirmDialog
        open={dialog?.kind === "delete"}
        title={copy.properties.deleteTitle(property.name)}
        description={copy.properties.deleteLede}
        confirm={copy.properties.delete}
        onCancel={() => setDialog(null)}
        onConfirm={() => {
          setDialog(null);
          remove.mutate(property.id, { onError: report });
        }}
      />
    </>
  );
}

/** The way to add a property, at the end of the headings, shown in this view where it is made. */
function AddPropertyButton({
  view,
  onChangeView,
}: {
  view: View;
  onChangeView: (change: ChangeView) => void;
}) {
  const { database } = useDatabaseScope();
  const [adding, setAdding] = useState(false);
  return (
    <>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={copy.properties.add}
        onClick={() => setAdding(true)}
        className="text-muted-foreground"
      >
        <Plus aria-hidden />
      </Button>
      {adding && (
        <PropertyDialog
          open
          onOpenChange={setAdding}
          onAdded={(made) =>
            onChangeView({
              properties: viewProperties([
                ...columnsOf(database, view),
                { property: made, visible: true },
              ]),
            })
          }
        />
      )}
    </>
  );
}
