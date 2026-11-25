import { cn } from "cn";
import type { Row } from "@/api/databases";
import { isEmpty, propertyOf, shownColumns, valueOf, type Column } from "@/lib/database-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { CellValue } from "./cell-value";
import { useDatabaseScope, useOpenRow } from "./database-scope";
import { GroupLabel } from "./group-label";
import { RowIcon, RowMenu, TitleField } from "./row-parts";
import type { ViewBodyProps } from "./view-body";

const copy = strings.databases;

/**
 * A view as a list: a line per row, its title, which opens it, and beside it
 * the values the view shows; in sections, when the view groups.
 */
export function ListView({ view, answer, editingTitle, onEditingTitle }: ViewBodyProps) {
  const { database, people } = useDatabaseScope();
  const shown = shownColumns(database, view).filter((column) => column.property.type !== "title");
  const grouping = view.group ? propertyOf(database, view.group.property) : undefined;
  const line = (row: Row) => (
    <ListLine
      key={`${row.id}:${row.group ?? ""}`}
      row={row}
      columns={shown}
      editingTitle={editingTitle === row.id}
      onEditingTitle={onEditingTitle}
    />
  );
  if (grouping === undefined || answer.groups === null) {
    return (
      <ul className="grid divide-y divide-border border-y border-border">
        {answer.rows.map(line)}
      </ul>
    );
  }
  return (
    <div className="grid gap-4">
      {answer.groups
        .filter((group) => !group.hidden)
        .map((group) => {
          const key = group.key ?? null;
          const rows = answer.rows.filter((row) => (row.group ?? null) === key);
          return (
            <section key={key ?? "none"} className="grid gap-1">
              <h3 className="flex items-center gap-2 px-1">
                <GroupLabel property={grouping} groupKey={key} people={people} />
                <span className="text-xs font-normal text-muted-foreground">
                  {copy.group.count(group.count)}
                </span>
              </h3>
              <ul className="grid divide-y divide-border border-y border-border">
                {rows.map(line)}
              </ul>
            </section>
          );
        })}
    </div>
  );
}

type ListLineProps = {
  row: Row;
  columns: readonly Column[];
  editingTitle: boolean;
  onEditingTitle: (rowId: string | null) => void;
};

function ListLine({ row, columns, editingTitle, onEditingTitle }: ListLineProps) {
  const { people } = useDatabaseScope();
  const openRow = useOpenRow();
  const title = titleOf(row);
  return (
    <li data-row="" className="flex min-h-(--tap) flex-wrap items-center gap-x-3 gap-y-1 py-1">
      {editingTitle ? (
        <span className="min-w-40 flex-1">
          <TitleField row={row} onDone={() => onEditingTitle(null)} />
        </span>
      ) : (
        <button
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
}
