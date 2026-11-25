import type { ChangeView, GroupKey, Row, View, ViewRows } from "@/api/databases";

/** What each layout of a view is given: the view, its rows, and the ways to change them. */
export type ViewBodyProps = {
  view: View;
  answer: ViewRows;
  /** The row whose title is being typed in place, as a new row's is. */
  editingTitle: string | null;
  onEditingTitle: (rowId: string | null) => void;
  /** Changes the view, shown at once and saved. */
  onChangeView: (change: ChangeView) => void;
  /** Adds a row, in a group of the view when one is named. */
  onNewRow: (group?: GroupKey) => void;
};

/** A line of a table's body or of a list: a group's heading, in a view that groups, or a row. */
export type Entry = { kind: "group"; key: GroupKey; count: number } | { kind: "row"; row: Row };

/** The lines of a table's body or a list: each group shown with its rows under it, or the rows alone. */
export function entriesOf(answer: ViewRows, grouped: boolean): Entry[] {
  if (!grouped || answer.groups === null) {
    return answer.rows.map((row) => ({ kind: "row", row }));
  }
  const byGroup = new Map<GroupKey, Row[]>();
  for (const row of answer.rows) {
    const key = row.group ?? null;
    const rows = byGroup.get(key);
    if (rows === undefined) {
      byGroup.set(key, [row]);
    } else {
      rows.push(row);
    }
  }
  const entries: Entry[] = [];
  for (const group of answer.groups) {
    if (!group.hidden) {
      const key = group.key ?? null;
      entries.push({ kind: "group", key, count: group.count });
      for (const row of byGroup.get(key) ?? []) {
        entries.push({ kind: "row", row });
      }
    }
  }
  return entries;
}

/** A line's key: a row is listed once in each of its groups. */
export const entryKey = (entry: Entry) =>
  entry.kind === "group" ? `group:${entry.key ?? ""}` : `${entry.row.id}:${entry.row.group ?? ""}`;

/** Where a row is among the lines, for a row whose title is to be typed, to draw it. */
export const entryOfRow = (entries: readonly Entry[], rowId: string) =>
  entries.findIndex((entry) => entry.kind === "row" && entry.row.id === rowId);
