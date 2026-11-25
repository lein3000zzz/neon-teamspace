import { ArrowDown, ArrowUp, Plus, Trash2, X } from "lucide-react";
import type { Sort, View } from "@/api/databases";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { strings } from "@/lib/strings";
import { useDatabaseScope } from "./database-scope";

const copy = strings.databases.sort;

/** A view sorts by at most this many properties. */
const MAX_SORTS = 10;

type SortBuilderProps = {
  view: View;
  onSave: (sorts: Sort[]) => void;
};

/**
 * A view's sorts, the first deciding first: each a property, once, and its
 * direction. Every change saves at once.
 */
export function SortBuilder({ view, onSave }: SortBuilderProps) {
  const { database } = useDatabaseScope();
  const sorts = view.sorts;
  const used = new Set(sorts.map((sort) => sort.property));
  const unused = database.properties.find((property) => !used.has(property.id));
  const set = (index: number, sort: Sort) =>
    onSave(sorts.map((entry, at) => (at === index ? sort : entry)));
  const move = (index: number, by: -1 | 1) => {
    const next = [...sorts];
    const [moved] = next.splice(index, 1);
    if (moved !== undefined) {
      next.splice(index + by, 0, moved);
    }
    onSave(next);
  };

  return (
    <div className="grid gap-3">
      {sorts.length === 0 ? (
        <p className="text-sm text-muted-foreground">{copy.none}</p>
      ) : (
        <ol className="grid gap-2">
          {sorts.map((sort, index) => {
            const n = index + 1;
            return (
              <li key={sort.property} className="flex flex-wrap items-center gap-1.5">
                <span className="min-w-32 flex-1">
                  <NativeSelect
                    aria-label={copy.property(n)}
                    value={sort.property}
                    onChange={(event) => set(index, { ...sort, property: event.target.value })}
                  >
                    {database.properties
                      .filter((property) => property.id === sort.property || !used.has(property.id))
                      .map((property) => (
                        <option key={property.id} value={property.id}>
                          {property.name}
                        </option>
                      ))}
                  </NativeSelect>
                </span>
                <span className="min-w-32 flex-1">
                  <NativeSelect
                    aria-label={copy.direction(n)}
                    value={sort.direction}
                    onChange={(event) =>
                      set(index, { ...sort, direction: event.target.value as Sort["direction"] })
                    }
                  >
                    <option value="ascending">{copy.directions.ascending}</option>
                    <option value="descending">{copy.directions.descending}</option>
                  </NativeSelect>
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={copy.moveUp(n)}
                  disabled={index === 0}
                  onClick={() => move(index, -1)}
                >
                  <ArrowUp aria-hidden />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={copy.moveDown(n)}
                  disabled={index === sorts.length - 1}
                  onClick={() => move(index, 1)}
                >
                  <ArrowDown aria-hidden />
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={copy.remove(n)}
                  onClick={() => onSave(sorts.filter((_, at) => at !== index))}
                >
                  <X aria-hidden />
                </Button>
              </li>
            );
          })}
        </ol>
      )}
      <div className="flex flex-wrap gap-2">
        {unused !== undefined && sorts.length < MAX_SORTS && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => onSave([...sorts, { property: unused.id, direction: "ascending" }])}
          >
            <Plus aria-hidden />
            {copy.add}
          </Button>
        )}
        {sorts.length > 0 && (
          <Button type="button" variant="ghost" size="sm" onClick={() => onSave([])}>
            <Trash2 aria-hidden />
            {copy.clear}
          </Button>
        )}
      </div>
    </div>
  );
}
