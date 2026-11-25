import { Plus, Trash2, X } from "lucide-react";
import { useState } from "react";
import type { Condition, Filter, Operator, Property, StatusGroup, View } from "@/api/databases";
import { Button } from "@/components/ui/button";
import { fieldStyles, Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import {
  conditionCount,
  DAY_RANGES,
  draftKey,
  draftOf,
  filterOf,
  isDay,
  MAX_CONDITIONS,
  MAX_DEPTH,
  ME,
  operatorsFor,
  propertyOf,
  readNumber,
  RELATIVE_DAYS,
  startingValue,
  takesValue,
  today,
  type ConditionDraft,
  type FilterDraft,
  type GroupDraft,
} from "@/lib/database-rules";
import { useSettledSave } from "@/lib/debounce";
import { attribution } from "@/lib/people";
import { strings } from "@/lib/strings";
import { useDatabaseScope } from "./database-scope";

const copy = strings.databases.filter;

/** How long edits to a filter settle before the view saves it. */
export const FILTER_SAVE_MS = 400;

type FilterBuilderProps = {
  view: View;
  /** Saves the view's filter: null removes it. */
  onSave: (filter: Filter | null) => void;
  /** A property to start a new condition on, as the column menu's "Filter by it" asks. */
  start?: Property | undefined;
};

/**
 * A view's filter as groups of conditions: each condition a property, an
 * operator its type takes and a value fit for both; each group passing when
 * all of its items pass, or one does. Groups nest up to three deep. What is
 * built is saved once edits pause, without the conditions still missing a
 * value, and as the builder closes.
 */
export function FilterBuilder({ view, onSave, start }: FilterBuilderProps) {
  const { database } = useDatabaseScope();
  const [draft, setDraft] = useState<FilterDraft>(() => {
    const initial = draftOf(view.filter);
    if (start === undefined) {
      return initial;
    }
    return { ...initial, items: [...initial.items, conditionDraft(start)] };
  });
  const schedule = useSettledSave(view.filter ?? null, onSave, FILTER_SAVE_MS);

  const change = (next: FilterDraft) => {
    setDraft(next);
    schedule(filterOf(next));
  };

  const full = conditionCount(draft) >= MAX_CONDITIONS;
  const first = database.properties[0];

  return (
    <div className="grid gap-3">
      {draft.items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{copy.none}</p>
      ) : (
        <GroupEditor group={draft} depth={1} full={full} onChange={change} />
      )}
      {full && <p className="text-xs text-muted-foreground">{copy.limit}</p>}
      <div className="flex flex-wrap gap-2">
        {first !== undefined && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={full}
            onClick={() => change({ ...draft, items: [...draft.items, conditionDraft(first)] })}
          >
            <Plus aria-hidden />
            {copy.add}
          </Button>
        )}
        {first !== undefined && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={full}
            onClick={() => change({ ...draft, items: [...draft.items, groupDraft(first)] })}
          >
            <Plus aria-hidden />
            {copy.addGroup}
          </Button>
        )}
        {draft.items.length > 0 && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => change({ ...draft, items: [] })}
          >
            <Trash2 aria-hidden />
            {copy.clear}
          </Button>
        )}
      </div>
    </div>
  );
}

function conditionDraft(property: Property): ConditionDraft {
  const operator = operatorsFor(property.type)[0] ?? "is";
  return { key: draftKey(), kind: "condition", condition: condition(property, operator) };
}

function groupDraft(property: Property): GroupDraft {
  return { key: draftKey(), kind: "group", conjunction: "and", items: [conditionDraft(property)] };
}

function condition(property: Property, operator: Operator): Condition {
  const value = startingValue(property, operator);
  return { property: property.id, operator, ...(value === undefined ? {} : { value }) };
}

type GroupEditorProps = {
  group: GroupDraft;
  depth: number;
  full: boolean;
  onChange: (group: GroupDraft) => void;
};

function GroupEditor({ group, depth, full, onChange }: GroupEditorProps) {
  const { database } = useDatabaseScope();
  const first = database.properties[0];
  const replace = (key: string, next: GroupDraft | ConditionDraft | null) =>
    onChange({
      ...group,
      items:
        next === null
          ? group.items.filter((item) => item.key !== key)
          : group.items.map((item) => (item.key === key ? next : item)),
    });
  return (
    <ol className="grid gap-2">
      {group.items.map((item, index) => (
        <li key={item.key} className="grid grid-cols-[4.5rem_minmax(0,1fr)] items-start gap-2">
          <span className="flex min-h-(--tap) items-center text-sm text-muted-foreground">
            {index === 0 ? (
              copy.where
            ) : index === 1 ? (
              <NativeSelect
                aria-label={copy.conjunction}
                value={group.conjunction}
                onChange={(event) =>
                  onChange({ ...group, conjunction: event.target.value as "and" | "or" })
                }
                className="pr-7 pl-2"
              >
                <option value="and">{copy.conjunctions.and}</option>
                <option value="or">{copy.conjunctions.or}</option>
              </NativeSelect>
            ) : (
              copy.conjunctions[group.conjunction]
            )}
          </span>
          {item.kind === "condition" ? (
            <ConditionEditor
              condition={item.condition}
              onChange={(next) => replace(item.key, { ...item, condition: next })}
              onRemove={() => replace(item.key, null)}
            />
          ) : (
            <fieldset className="grid gap-2 rounded-md border border-border p-2">
              <legend className="sr-only">{copy.group}</legend>
              <GroupEditor
                group={item}
                depth={depth + 1}
                full={full}
                onChange={(next) => replace(item.key, next)}
              />
              <div className="flex flex-wrap gap-1">
                {first !== undefined && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={full}
                    onClick={() =>
                      replace(item.key, { ...item, items: [...item.items, conditionDraft(first)] })
                    }
                  >
                    <Plus aria-hidden />
                    {copy.add}
                  </Button>
                )}
                {first !== undefined && depth + 1 < MAX_DEPTH && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={full}
                    onClick={() =>
                      replace(item.key, { ...item, items: [...item.items, groupDraft(first)] })
                    }
                  >
                    <Plus aria-hidden />
                    {copy.addGroup}
                  </Button>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => replace(item.key, null)}
                >
                  <Trash2 aria-hidden />
                  {copy.removeGroup}
                </Button>
              </div>
            </fieldset>
          )}
        </li>
      ))}
    </ol>
  );
}

type ConditionEditorProps = {
  condition: Condition;
  onChange: (condition: Condition) => void;
  onRemove: () => void;
};

function ConditionEditor({ condition: current, onChange, onRemove }: ConditionEditorProps) {
  const { database } = useDatabaseScope();
  const property = propertyOf(database, current.property);
  if (property === undefined) {
    return null;
  }
  const operators = operatorsFor(property.type);
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="min-w-32 flex-1">
        <NativeSelect
          aria-label={copy.property}
          value={property.id}
          onChange={(event) => {
            const next = propertyOf(database, event.target.value);
            if (next !== undefined) {
              const operator = operatorsFor(next.type).includes(current.operator)
                ? current.operator
                : (operatorsFor(next.type)[0] ?? "is");
              onChange(condition(next, operator));
            }
          }}
        >
          {database.properties.map((entry) => (
            <option key={entry.id} value={entry.id}>
              {entry.name}
            </option>
          ))}
        </NativeSelect>
      </span>
      <span className="min-w-32 flex-1">
        <NativeSelect
          aria-label={copy.operator}
          value={current.operator}
          onChange={(event) => {
            const operator = event.target.value as Operator;
            const keeps =
              takesValue(operator) &&
              takesValue(current.operator) &&
              (operator === "within") === (current.operator === "within") &&
              (operator === "in_group") === (current.operator === "in_group");
            onChange(keeps ? { ...current, operator } : condition(property, operator));
          }}
        >
          {operators.map((operator) => (
            <option key={operator} value={operator}>
              {copy.operators[operator]}
            </option>
          ))}
        </NativeSelect>
      </span>
      {takesValue(current.operator) && (
        <span className="min-w-32 flex-1">
          <ValueInput
            key={property.id}
            property={property}
            condition={current}
            onChange={(value) => onChange({ ...current, value })}
          />
        </span>
      )}
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={copy.remove}
        onClick={onRemove}
      >
        <X aria-hidden />
      </Button>
    </div>
  );
}

const STATUS_GROUPS: readonly StatusGroup[] = ["to_do", "in_progress", "done"];

type ValueInputProps = {
  property: Property;
  condition: Condition;
  onChange: (value: unknown) => void;
};

/** What a condition compares with, as its property's type and its operator take it. */
function ValueInput({ property, condition: current, onChange }: ValueInputProps) {
  const { people, seesPeople } = useDatabaseScope();
  const value = current.value;
  switch (property.type) {
    case "number":
    case "unique_id":
      return <NumberValueInput value={value} onChange={onChange} />;
    case "select":
    case "multi_select":
    case "status":
      if (current.operator === "in_group") {
        return (
          <NativeSelect
            aria-label={copy.value}
            value={typeof value === "string" ? value : "to_do"}
            onChange={(event) => onChange(event.target.value)}
          >
            {STATUS_GROUPS.map((group) => (
              <option key={group} value={group}>
                {strings.databases.properties.statusGroups[group]}
              </option>
            ))}
          </NativeSelect>
        );
      }
      return (
        <NativeSelect
          aria-label={copy.value}
          value={typeof value === "string" ? value : ""}
          onChange={(event) => onChange(event.target.value)}
        >
          {(property.options ?? []).map((option) => (
            <option key={option.id} value={option.id}>
              {option.name}
            </option>
          ))}
        </NativeSelect>
      );
    case "date":
    case "created_time":
    case "last_edited_time":
      return <DateValueInput condition={current} onChange={onChange} />;
    case "person":
    case "created_by":
    case "last_edited_by":
      return (
        <NativeSelect
          aria-label={copy.value}
          value={typeof value === "string" ? value : ME}
          onChange={(event) => onChange(event.target.value)}
        >
          <option value={ME}>{copy.me}</option>
          {seesPeople &&
            [...people.values()].map((person) => (
              <option key={person.id} value={person.id}>
                {attribution(person)}
              </option>
            ))}
        </NativeSelect>
      );
    case "checkbox":
      return (
        <NativeSelect
          aria-label={copy.value}
          value={value === false ? "false" : "true"}
          onChange={(event) => onChange(event.target.value === "true")}
        >
          <option value="true">{copy.checked}</option>
          <option value="false">{copy.unchecked}</option>
        </NativeSelect>
      );
    default:
      return (
        <Input
          aria-label={copy.value}
          maxLength={1000}
          value={typeof value === "string" ? value : ""}
          onChange={(event) => onChange(event.target.value)}
        />
      );
  }
}

/**
 * A number to compare with, kept as it is typed, so "-" and "2." stay on the
 * way to -3 and 2.5: the condition takes it whenever it reads as a number,
 * and is left out of the filter while it does not.
 */
function NumberValueInput({
  value,
  onChange,
}: {
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const [text, setText] = useState(typeof value === "number" ? String(value) : "");
  return (
    <Input
      aria-label={copy.value}
      inputMode="decimal"
      value={text}
      onChange={(event) => {
        setText(event.target.value);
        onChange(readNumber(event.target.value) ?? "");
      }}
    />
  );
}

const EXACT = "exact";

/** A day relative to today, or an exact one with a date field; a range of days for `within`. */
function DateValueInput({ condition: current, onChange }: Omit<ValueInputProps, "property">) {
  const value = typeof current.value === "string" ? current.value : "";
  if (current.operator === "within") {
    return (
      <NativeSelect
        aria-label={copy.value}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        {DAY_RANGES.map((range) => (
          <option key={range} value={range}>
            {copy.ranges[range]}
          </option>
        ))}
      </NativeSelect>
    );
  }
  const exact = isDay(value);
  return (
    <span className="flex flex-wrap gap-1.5">
      <span className="min-w-32 flex-1">
        <NativeSelect
          aria-label={copy.value}
          value={exact ? EXACT : value}
          onChange={(event) =>
            onChange(event.target.value === EXACT ? today() : event.target.value)
          }
        >
          {RELATIVE_DAYS.map((day) => (
            <option key={day} value={day}>
              {copy.relative[day]}
            </option>
          ))}
          <option value={EXACT}>{copy.exact}</option>
        </NativeSelect>
      </span>
      {exact && (
        <input
          type="date"
          aria-label={copy.day}
          value={value}
          onChange={(event) => event.target.value !== "" && onChange(event.target.value)}
          className={`${fieldStyles} h-(--tap) min-w-32 flex-1`}
        />
      )}
    </span>
  );
}
