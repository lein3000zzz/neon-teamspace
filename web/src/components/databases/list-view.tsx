import { cn } from "cn";
import { memo, useEffect, useMemo } from "react";
import type { Row } from "@/api/databases";
import { isEmpty, propertyOf, shownColumns, valueOf, type Column } from "@/lib/database-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { CellValue } from "./cell-value";
import { useDatabaseScope, useOpenRow } from "./database-scope";
import { GroupLabel } from "./group-label";
import { RowIcon, RowMenu, TitleField, useRefocus } from "./row-parts";
import { entriesOf, entryKey, entryOfRow, type ViewBodyProps } from "./view-body";
import { useWindowed } from "./windowed";

const copy = strings.databases;

/** A line of a list before it is measured: one line of text. */
const LINE_ESTIMATE = 45;

/**
 * A view as a list: a line per row, its title, which opens it, and beside it
 * the values the view shows; under each group's heading, when the view
 * groups. Only the lines near the screen are drawn, so thousands stay quick.
 */
export function ListView({ view, answer, editingTitle, onEditingTitle }: ViewBodyProps) {
  const { database, people } = useDatabaseScope();
  const shown = useMemo(
    () => shownColumns(database, view).filter((column) => column.property.type !== "title"),
    [database, view],
  );
  const grouping = view.group ? propertyOf(database, view.group.property) : undefined;
  const entries = useMemo(() => entriesOf(answer, grouping !== undefined), [answer, grouping]);
  const keys = useMemo(() => entries.map(entryKey), [entries]);
  const { list, items, before, after, measure, scrollTo } = useWindowed<HTMLUListElement>(
    keys,
    LINE_ESTIMATE,
  );

  // A row whose title is to be typed, as a new one, is drawn wherever it is.
  useEffect(() => {
    if (editingTitle !== null) {
      const at = entryOfRow(entries, editingTitle);
      if (at >= 0) {
        scrollTo(at);
      }
    }
  }, [editingTitle, entries, scrollTo]);

  return (
    <ul ref={list} className="grid divide-y divide-border border-y border-border">
      {before > 0 && <li aria-hidden style={{ height: before, border: 0 }} />}
      {items.map((item) => {
        const entry = entries[item.index];
        if (entry === undefined) {
          return null;
        }
        return entry.kind === "group" ? (
          <li key={item.key} ref={measure} data-index={item.index} className="px-1 pt-4 pb-1">
            <h3 className="flex items-center gap-2">
              {grouping !== undefined && (
                <GroupLabel property={grouping} groupKey={entry.key} people={people} />
              )}
              <span className="text-xs font-normal text-muted-foreground">
                {copy.group.count(entry.count)}
              </span>
            </h3>
          </li>
        ) : (
          <ListLine
            key={item.key}
            index={item.index}
            measure={measure}
            row={entry.row}
            columns={shown}
            editingTitle={editingTitle === entry.row.id}
            onEditingTitle={onEditingTitle}
          />
        );
      })}
      {after > 0 && <li aria-hidden style={{ height: after, border: 0 }} />}
    </ul>
  );
}

type ListLineProps = {
  /** Its place among the list's lines. */
  index: number;
  /** Measures it as it is drawn. */
  measure: (element: Element | null) => void;
  row: Row;
  columns: readonly Column[];
  editingTitle: boolean;
  onEditingTitle: (rowId: string | null) => void;
};

/** A row of the list, drawn again only when it, or what the list shows of it, changes. */
const ListLine = memo(function ListLine({
  index,
  measure,
  row,
  columns,
  editingTitle,
  onEditingTitle,
}: ListLineProps) {
  const { people } = useDatabaseScope();
  const openRow = useOpenRow();
  const { target: titleButton, ended: titleEnded } = useRefocus<HTMLButtonElement>(editingTitle);
  const title = titleOf(row);
  return (
    <li
      ref={measure}
      data-index={index}
      data-row=""
      className="flex min-h-(--tap) flex-wrap items-center gap-x-3 gap-y-1 py-1"
    >
      {editingTitle ? (
        <span className="min-w-40 flex-1">
          <TitleField
            row={row}
            onDone={(fromKeyboard) => {
              titleEnded(fromKeyboard);
              onEditingTitle(null);
            }}
          />
        </span>
      ) : (
        <button
          ref={titleButton}
          type="button"
          onClick={() => openRow(row.id)}
          className="flex min-h-(--tap-sm) min-w-0 flex-1 items-center gap-2 rounded-sm px-1 text-left text-sm font-medium outline-offset-[-2px] hover:underline"
        >
          <RowIcon row={row} />
          <span className={cn("truncate", row.title === "" && "text-muted-foreground")}>
            {title}
          </span>
        </button>
      )}
      <span className="flex min-w-0 flex-wrap items-center justify-end gap-2 text-sm">
        {columns.map((column) => {
          const value = valueOf(row, column.property);
          return isEmpty(value) ? null : (
            <span key={column.property.id} className="flex max-w-56 min-w-0 items-center">
              <span className="sr-only">{column.property.name}: </span>
              <CellValue property={column.property} value={value} people={people} inline />
            </span>
          );
        })}
      </span>
      <span data-row-actions="" className="flex shrink-0 items-center">
        <RowMenu row={row} />
      </span>
    </li>
  );
});
