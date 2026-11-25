import { cn } from "cn";
import { Check } from "lucide-react";
import type { Property } from "@/api/databases";
import type { Person } from "@/api/workspaces";
import { HueDot } from "@/components/nodes/hue-dot";
import {
  formatDate,
  formatNumber,
  idsOf,
  isDateValue,
  optionOf,
  type DateValue,
} from "@/lib/database-rules";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { useDateFormat } from "./date-format";
import { OptionChip } from "./option-chip";

const copy = strings.databases.cells;

type CellValueProps = {
  property: Property;
  value: unknown;
  people: ReadonlyMap<string, Person>;
  /** On one line, cut short, as a table cell shows it; else wrapping, as a card does. */
  inline?: boolean;
};

/**
 * A value as it reads, by its property's type: text, a number in its format,
 * options as pills in their hues, dates in the reader's format, people by
 * name in their hues, a check, a link's address. Nothing for no value.
 */
export function CellValue({ property, value, people, inline = false }: CellValueProps) {
  if (value === undefined || value === null) {
    return null;
  }
  const wrap = inline ? "flex min-w-0 items-center gap-1 overflow-hidden" : "flex flex-wrap gap-1";
  switch (property.type) {
    case "title":
    case "text":
      return typeof value === "string" ? (
        <span className={cn("min-w-0", inline ? "truncate" : "break-words whitespace-pre-wrap")}>
          {value}
        </span>
      ) : null;
    case "url":
      return typeof value === "string" ? (
        <span
          className={cn("min-w-0 text-primary underline underline-offset-2", inline && "truncate")}
        >
          {value}
        </span>
      ) : null;
    case "number":
      return typeof value === "number" ? (
        <span className="tabular-nums">
          {formatNumber(value, property.number_format, property.currency)}
        </span>
      ) : null;
    case "select":
    case "status": {
      const option = optionOf(property, value);
      return option ? <OptionChip option={option} /> : null;
    }
    case "multi_select": {
      const options = idsOf(value)
        .map((id) => optionOf(property, id))
        .filter((option) => option !== undefined);
      return (
        <span className={wrap}>
          {options.map((option) => (
            <OptionChip key={option.id} option={option} />
          ))}
        </span>
      );
    }
    case "date":
      return isDateValue(value) ? <DateText propertyId={property.id} value={value} /> : null;
    case "created_time":
    case "last_edited_time":
      return typeof value === "string" ? <span>{formatDateTime(value)}</span> : null;
    case "person":
    case "created_by":
    case "last_edited_by":
      return (
        <span className={wrap}>
          {idsOf(value).map((id) => (
            <PersonName key={id} person={people.get(id)} />
          ))}
        </span>
      );
    case "checkbox":
      return value === true ? (
        <Check aria-label={strings.databases.filter.checked} className="size-4" />
      ) : null;
    case "unique_id":
      return typeof value === "string" ? (
        <span className="font-mono text-xs text-muted-foreground">{value}</span>
      ) : null;
  }
}

function DateText({ propertyId, value }: { propertyId: string; value: DateValue }) {
  return <span>{formatDate(value, useDateFormat(propertyId))}</span>;
}

/** A person a value names, in their hue; one the user does not know is someone. */
export function PersonName({ person }: { person: Person | undefined }) {
  return (
    <span className="inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-full border border-border px-2 py-px text-xs leading-5">
      <HueDot {...(person === undefined ? {} : { hue: person.hue })} className="size-1.5" />
      <span className="truncate">{person?.display_name ?? copy.someone}</span>
    </span>
  );
}
