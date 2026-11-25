import { ArrowDown, ArrowUp, Pencil, Plus } from "lucide-react";
import { useState } from "react";
import type { Property, View, ViewProperty } from "@/api/databases";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { columnsOf, viewProperties, withColumn, type Column } from "@/lib/database-rules";
import { strings } from "@/lib/strings";
import { useDatabaseScope } from "./database-scope";
import { PropertyDialog } from "./property-dialog";
import { PropertyIcon } from "./property-icon";

const copy = strings.databases.properties;

type PropertiesPickerProps = {
  view: View;
  onSave: (properties: ViewProperty[]) => void;
};

/**
 * Which properties a view shows, in its order: each shown or hidden, moved up
 * or down, or opened to edit; and a new one, shown here. The title always
 * shows.
 */
export function PropertiesPicker({ view, onSave }: PropertiesPickerProps) {
  const { database } = useDatabaseScope();
  const [editing, setEditing] = useState<Property | null>(null);
  const [adding, setAdding] = useState(false);
  const columns = columnsOf(database, view);
  const save = (next: Column[]) => onSave(viewProperties(next));
  const move = (index: number, by: -1 | 1) => {
    const next = [...columns];
    const [moved] = next.splice(index, 1);
    if (moved !== undefined) {
      next.splice(index + by, 0, moved);
    }
    save(next);
  };

  return (
    <div className="grid gap-3">
      <p className="text-xs text-muted-foreground">{copy.pickerLede}</p>
      <ol className="grid gap-0.5">
        {columns.map((column, index) => {
          const { property } = column;
          const title = property.type === "title";
          return (
            <li key={property.id} className="flex min-h-(--tap) items-center gap-1">
              <label className="flex min-h-(--tap) min-w-0 flex-1 cursor-pointer items-center gap-2 text-sm">
                <Checkbox
                  aria-label={copy.shown(property.name)}
                  checked={title || column.visible}
                  disabled={title}
                  onCheckedChange={(checked) =>
                    save(withColumn(columns, property.id, { visible: checked === true }))
                  }
                />
                <PropertyIcon kind={property.type} className="text-muted-foreground" />
                <span className="truncate">{property.name}</span>
              </label>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={copy.moveUp(property.name)}
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                <ArrowUp aria-hidden />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={copy.moveDown(property.name)}
                disabled={index === columns.length - 1}
                onClick={() => move(index, 1)}
              >
                <ArrowDown aria-hidden />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={copy.edit(property.name)}
                onClick={() => setEditing(property)}
              >
                <Pencil aria-hidden />
              </Button>
            </li>
          );
        })}
      </ol>
      <div>
        <Button type="button" variant="outline" size="sm" onClick={() => setAdding(true)}>
          <Plus aria-hidden />
          {copy.add}
        </Button>
      </div>
      {editing !== null && (
        <PropertyDialog
          key={editing.id}
          open
          onOpenChange={(open) => !open && setEditing(null)}
          property={editing}
        />
      )}
      {adding && (
        <PropertyDialog
          open
          onOpenChange={setAdding}
          onAdded={(made) => save([...columns, { property: made, visible: true }])}
        />
      )}
    </div>
  );
}
