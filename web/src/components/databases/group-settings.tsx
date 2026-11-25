import { useId } from "react";
import type { Grouping, GroupKey, QueryGroup, View } from "@/api/databases";
import { CheckboxField } from "@/components/forms/field";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { groups as groupsBy, propertyOf } from "@/lib/database-rules";
import { strings } from "@/lib/strings";
import { useDatabaseScope } from "./database-scope";
import { groupName } from "./group-label";

const copy = strings.databases.group;

const NONE = "";

type GroupSettingsProps = {
  view: View;
  /** The view's groups, as its rows came, hidden ones too. */
  groups: readonly QueryGroup[] | null;
  onSave: (group: Grouping | null) => void;
};

/**
 * What a view groups by, a select, status or person property, and which of
 * its groups it shows. A board always groups; a table or a list may not.
 */
export function GroupSettings({ view, groups, onSave }: GroupSettingsProps) {
  const { database, people } = useDatabaseScope();
  const selectId = useId();
  const choices = database.properties.filter((property) => groupsBy(property.type));
  const current = view.group ? propertyOf(database, view.group.property) : undefined;
  const board = view.layout === "board";

  if (choices.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.noGroupable}</p>;
  }
  const hidden = new Set<GroupKey>(view.group?.hidden ?? []);
  const toggle = (key: GroupKey, shown: boolean) => {
    if (view.group === undefined || view.group === null) {
      return;
    }
    const next = shown ? [...hidden].filter((entry) => entry !== key) : [...hidden, key];
    onSave({ ...view.group, hidden: next });
  };

  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <Label htmlFor={selectId}>{copy.by}</Label>
        <NativeSelect
          id={selectId}
          value={current?.id ?? NONE}
          onChange={(event) =>
            onSave(event.target.value === NONE ? null : { property: event.target.value })
          }
        >
          {!board && <option value={NONE}>{copy.none}</option>}
          {choices.map((property) => (
            <option key={property.id} value={property.id}>
              {property.name}
            </option>
          ))}
        </NativeSelect>
        {board && <p className="text-xs text-muted-foreground">{copy.boardNeeds}</p>}
      </div>
      {current !== undefined && groups !== null && groups.length > 0 && (
        <fieldset className="grid gap-0.5">
          <legend className="mb-1 text-sm font-medium">{copy.groups}</legend>
          {groups.map((group) => {
            const name = groupName(current, group.key ?? null, people);
            return (
              <CheckboxField
                key={group.key ?? NONE}
                label={`${name} · ${copy.count(group.count)}`}
                checked={!group.hidden}
                onCheckedChange={(shown) => toggle(group.key ?? null, shown)}
              />
            );
          })}
        </fieldset>
      )}
    </div>
  );
}
