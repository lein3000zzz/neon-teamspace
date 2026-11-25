import type { ChangeView, GroupKey, View, ViewRows } from "@/api/databases";

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
