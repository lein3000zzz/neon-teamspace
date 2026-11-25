import type { GroupKey, Property } from "@/api/databases";
import { optionOf } from "@/lib/database-rules";
import { attribution, type Someone } from "@/lib/people";
import { strings } from "@/lib/strings";
import { PersonName } from "./cell-value";
import { OptionChip } from "./option-chip";

/**
 * What a group is called: its option, its person, an agent with whom it acts
 * for, or the rows without a value.
 */
export function groupName(
  property: Property,
  key: GroupKey,
  people: ReadonlyMap<string, Someone>,
): string {
  if (key === null) {
    return strings.databases.group.noValue(property.name);
  }
  if (property.type === "person") {
    const person = people.get(key);
    return person === undefined ? strings.databases.cells.someone : attribution(person);
  }
  return optionOf(property, key)?.name ?? key;
}

/** The hue a group is drawn in: its option's; none for people and the rows without a value. */
export function groupHue(property: Property, key: GroupKey): number | undefined {
  return key === null || property.type === "person"
    ? undefined
    : (optionOf(property, key)?.hue ?? undefined);
}

type GroupLabelProps = {
  property: Property;
  groupKey: GroupKey;
  people: ReadonlyMap<string, Someone>;
};

/** A group's name as its heading shows it: an option's pill, a person in their hue, or plain words. */
export function GroupLabel({ property, groupKey, people }: GroupLabelProps) {
  if (groupKey === null) {
    return (
      <span className="truncate text-sm text-muted-foreground">
        {groupName(property, groupKey, people)}
      </span>
    );
  }
  if (property.type === "person") {
    return <PersonName person={people.get(groupKey)} />;
  }
  const option = optionOf(property, groupKey);
  return option === undefined ? null : <OptionChip option={option} />;
}
