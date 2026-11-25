import { cn } from "cn";
import { useRef, useState, type ReactNode } from "react";
import { useChangeRow, type Property, type Row } from "@/api/databases";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { isComputed, namesPeople, valueOf } from "@/lib/database-rules";
import { edits } from "@/lib/page-rules";
import { titleOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import {
  DateEditor,
  NumberEditor,
  OptionEditor,
  PersonEditor,
  TextEditor,
  UrlEditor,
  type EditorProps,
} from "./cell-editors";
import { CellValue } from "./cell-value";
import { useDatabaseScope } from "./database-scope";

const copy = strings.databases.cells;

type CellProps = {
  property: Property;
  row: Row;
  /** How it sits: a table's cell, on one line, or a row's properties, wrapping. */
  variant: "table" | "panel";
};

/**
 * A row's value of a property, and, for whoever may change the row, the way
 * to: a checkbox toggles in place, any other value opens its editor in a
 * popover over the cell. Computed values, and people for guests, only show.
 */
export function Cell({ property, row, variant }: CellProps) {
  const workspace = useWorkspace();
  const { database, people, seesPeople, report } = useDatabaseScope();
  const change = useChangeRow(workspace.id, database);
  const value = valueOf(row, property);
  const label = copy.edit(property.name, titleOf(row));
  const trashed = (database.page.trashed_at ?? null) !== null;
  const editable =
    edits(row.access) &&
    !trashed &&
    !isComputed(property.type) &&
    (seesPeople || !namesPeople(property.type));
  const save = (next: unknown) =>
    change.mutate({ rowId: row.id, values: { [property.id]: next } }, { onError: report });
  const inline = variant === "table";
  const shown = <CellValue property={property} value={value} people={people} inline={inline} />;

  if (property.type === "checkbox") {
    return (
      <span
        className={cn("flex items-center", inline ? "min-h-(--tap-sm) px-2" : "min-h-(--tap-sm)")}
      >
        <Checkbox
          aria-label={label}
          checked={value === true}
          disabled={!editable}
          onCheckedChange={(checked) => save(checked === true ? true : null)}
        />
      </span>
    );
  }
  if (!editable) {
    return (
      <span
        className={cn(
          "flex min-h-(--tap-sm) min-w-0 items-center text-sm",
          inline ? "px-2" : "py-1",
          isComputed(property.type) && "text-muted-foreground",
        )}
      >
        {shown}
      </span>
    );
  }
  return (
    <EditableCell property={property} value={value} label={label} inline={inline} onSave={save}>
      {shown}
    </EditableCell>
  );
}

type EditableCellProps = {
  property: Property;
  value: unknown;
  label: string;
  inline: boolean;
  onSave: (value: unknown) => void;
  children: ReactNode;
};

function EditableCell({ property, value, label, inline, onSave, children }: EditableCellProps) {
  const [open, setOpen] = useState(false);
  const commit = useRef<(() => void) | null>(null);
  const editor: EditorProps = {
    property,
    value,
    label,
    onSave,
    onClose: () => setOpen(false),
    commitRef: commit,
  };
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          commit.current?.();
        }
        setOpen(next);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex min-h-(--tap-sm) w-full min-w-0 items-center rounded-sm text-left text-sm outline-offset-[-2px] hover:bg-accent aria-expanded:bg-accent",
            inline ? "px-2" : "px-1.5 py-1",
          )}
        >
          <span className="sr-only">{`${label}: `}</span>
          {children}
        </button>
      </PopoverTrigger>
      <PopoverContent
        aria-label={label}
        // Escape leaves the value as it was: nothing is saved as the popover closes.
        onEscapeKeyDown={() => {
          commit.current = null;
        }}
        className="w-[min(20rem,calc(100vw-1rem))]"
      >
        {editorFor(editor)}
      </PopoverContent>
    </Popover>
  );
}

/** The editor for a value of the property's type. */
function editorFor(props: EditorProps): ReactNode {
  switch (props.property.type) {
    case "number":
      return <NumberEditor {...props} />;
    case "url":
      return <UrlEditor {...props} />;
    case "select":
    case "status":
    case "multi_select":
      return <OptionEditor {...props} />;
    case "date":
      return <DateEditor {...props} />;
    case "person":
      return <PersonEditor {...props} />;
    default:
      return <TextEditor {...props} />;
  }
}
