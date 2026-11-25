/**
 * What the server's database rules let a view, a filter or a value be, so the
 * app offers only what can work: the operators each type takes, the types a
 * board groups by, how values read. The server checks everything again.
 */
import type {
  Condition,
  Database,
  Filter,
  GroupKey,
  NumberFormat,
  Operator,
  OptionChange,
  Property,
  PropertyKind,
  PropertyOption,
  Row,
  View,
  ViewProperty,
  ViewRows,
} from "@/api/databases";

/** Types whose values the server works out from the row, which nobody sets. */
const COMPUTED: ReadonlySet<PropertyKind> = new Set([
  "unique_id",
  "created_time",
  "created_by",
  "last_edited_time",
  "last_edited_by",
]);

export function isComputed(kind: PropertyKind): boolean {
  return COMPUTED.has(kind);
}

/** The types a board, or any view, groups by. */
export function groups(kind: PropertyKind): boolean {
  return kind === "select" || kind === "status" || kind === "person";
}

/** The types whose values are options of the property. */
export function hasOptions(kind: PropertyKind): boolean {
  return kind === "select" || kind === "multi_select" || kind === "status";
}

/** The types whose values name people, which guests neither see nor set. */
export function namesPeople(kind: PropertyKind): boolean {
  return kind === "person" || kind === "created_by" || kind === "last_edited_by";
}

/**
 * Whether the user may give a property another type: never the title, and,
 * for a guest, who knows no other member, not a person, whose values would
 * become people's names.
 */
export function retypes(property: Property, seesPeople: boolean): boolean {
  return property.type !== "title" && (seesPeople || property.type !== "person");
}

/** Every type a property can be given, in the order the type picker lists them. */
export const PROPERTY_KINDS: readonly PropertyKind[] = [
  "text",
  "number",
  "select",
  "multi_select",
  "status",
  "date",
  "person",
  "checkbox",
  "url",
  "unique_id",
  "created_time",
  "created_by",
  "last_edited_time",
  "last_edited_by",
];

/** The types a property may change to: any but the title, and a unique id only while it has none. */
export function kindsFor(database: Database, property?: Property): PropertyKind[] {
  if (property?.type === "title") {
    return ["title"];
  }
  const hasUniqueId = database.properties.some(
    (entry) => entry.type === "unique_id" && entry.id !== property?.id,
  );
  return PROPERTY_KINDS.filter((kind) => kind !== "unique_id" || !hasUniqueId);
}

const TEXT_OPERATORS: readonly Operator[] = [
  "is",
  "is_not",
  "contains",
  "does_not_contain",
  "starts_with",
  "ends_with",
];
const NUMBER_OPERATORS: readonly Operator[] = [
  "equals",
  "not_equals",
  "greater_than",
  "greater_than_or_equal",
  "less_than",
  "less_than_or_equal",
];
const DATE_OPERATORS: readonly Operator[] = [
  "is",
  "before",
  "after",
  "on_or_before",
  "on_or_after",
  "within",
];
const EMPTY_OPERATORS: readonly Operator[] = ["is_empty", "is_not_empty"];

/** The operators a condition on a property of this type takes, as the server has them. */
export function operatorsFor(kind: PropertyKind): Operator[] {
  switch (kind) {
    case "title":
    case "text":
    case "url":
      return [...TEXT_OPERATORS, ...EMPTY_OPERATORS];
    case "number":
    case "unique_id":
      return [...NUMBER_OPERATORS, ...EMPTY_OPERATORS];
    case "select":
      return ["is", "is_not", ...EMPTY_OPERATORS];
    case "status":
      return ["is", "is_not", "in_group", ...EMPTY_OPERATORS];
    case "multi_select":
      return ["contains", "does_not_contain", ...EMPTY_OPERATORS];
    case "date":
    case "created_time":
    case "last_edited_time":
      return [...DATE_OPERATORS, ...EMPTY_OPERATORS];
    case "person":
    case "created_by":
    case "last_edited_by":
      return ["contains", "does_not_contain", ...EMPTY_OPERATORS];
    case "checkbox":
      return ["is"];
  }
}

/** Whether a condition with this operator compares with a value. */
export function takesValue(operator: Operator): boolean {
  return operator !== "is_empty" && operator !== "is_not_empty";
}

/** Days relative to today a date condition compares with. */
export const RELATIVE_DAYS = [
  "today",
  "tomorrow",
  "yesterday",
  "one_week_ago",
  "one_week_from_now",
  "one_month_ago",
  "one_month_from_now",
] as const;

/** Ranges of days counted from today, for `within`. */
export const DAY_RANGES = [
  "past_week",
  "past_month",
  "past_year",
  "next_week",
  "next_month",
  "next_year",
] as const;

/** What a condition on a person compares with to mean whoever looks at the view. */
export const ME = "me";

/** The value a new condition starts with, for its property and operator. */
export function startingValue(property: Property, operator: Operator): unknown {
  if (!takesValue(operator)) {
    return undefined;
  }
  switch (property.type) {
    case "checkbox":
      return true;
    case "date":
    case "created_time":
    case "last_edited_time":
      return operator === "within" ? "past_week" : "today";
    case "person":
    case "created_by":
    case "last_edited_by":
      return ME;
    case "status":
      return operator === "in_group" ? "to_do" : property.options?.[0]?.id;
    case "select":
    case "multi_select":
      return property.options?.[0]?.id;
    case "number":
    case "unique_id":
      return 0;
    default:
      return "";
  }
}

/** A condition is ready to save once it has the value its operator needs. */
export function conditionReady(condition: Condition): boolean {
  if (!takesValue(condition.operator)) {
    return true;
  }
  const value = condition.value;
  if (typeof value === "string") {
    return value.trim() !== "";
  }
  if (typeof value === "number") {
    return Number.isFinite(value);
  }
  return typeof value === "boolean";
}

/** A filter as its builder holds it: groups and conditions, each with a key for the list. */
export type FilterDraft = GroupDraft;
export type GroupDraft = {
  key: string;
  kind: "group";
  conjunction: "and" | "or";
  items: (GroupDraft | ConditionDraft)[];
};
export type ConditionDraft = { key: string; kind: "condition"; condition: Condition };

/** Groups nest at most this deep, as the server has them. */
export const MAX_DEPTH = 3;
/** A filter holds at most this many conditions. */
export const MAX_CONDITIONS = 50;

let draftKeys = 0;
/** A key for a new part of a draft, unique while the app runs. */
export function draftKey(): string {
  draftKeys += 1;
  return `filter-${draftKeys}`;
}

/** The builder's draft of a view's filter: always a group at the top, as the builder shows one. */
export function draftOf(filter: Filter | null | undefined): FilterDraft {
  const part = (entry: Filter): GroupDraft | ConditionDraft => {
    if ("and" in entry) {
      return { key: draftKey(), kind: "group", conjunction: "and", items: entry.and.map(part) };
    }
    if ("or" in entry) {
      return { key: draftKey(), kind: "group", conjunction: "or", items: entry.or.map(part) };
    }
    return { key: draftKey(), kind: "condition", condition: entry };
  };
  if (filter === null || filter === undefined) {
    return { key: draftKey(), kind: "group", conjunction: "and", items: [] };
  }
  const top = part(filter);
  return top.kind === "group"
    ? top
    : { key: draftKey(), kind: "group", conjunction: "and", items: [top] };
}

/**
 * The filter a draft stands for, leaving out empty groups and conditions not
 * ready yet; null when nothing is left, which removes the view's filter.
 */
export function filterOf(draft: FilterDraft): Filter | null {
  const part = (entry: GroupDraft | ConditionDraft): Filter | null => {
    if (entry.kind === "condition") {
      return conditionReady(entry.condition) ? entry.condition : null;
    }
    const items = entry.items.map(part).filter((item): item is Filter => item !== null);
    if (items.length === 0) {
      return null;
    }
    return entry.conjunction === "and" ? { and: items } : { or: items };
  };
  return part(draft);
}

/** How many conditions a draft holds, ready or not. */
export function conditionCount(draft: GroupDraft | ConditionDraft): number {
  return draft.kind === "condition"
    ? 1
    : draft.items.reduce((sum, item) => sum + conditionCount(item), 0);
}

/** How many conditions a saved filter holds, as its button says. */
export function filterSize(filter: Filter | null | undefined): number {
  if (filter === null || filter === undefined) {
    return 0;
  }
  if ("and" in filter) {
    return filter.and.reduce((sum, entry) => sum + filterSize(entry), 0);
  }
  if ("or" in filter) {
    return filter.or.reduce((sum, entry) => sum + filterSize(entry), 0);
  }
  return 1;
}

/** A property of the database by its id. */
export function propertyOf(database: Database, id: string): Property | undefined {
  return database.properties.find((property) => property.id === id);
}

/** The database's title property, which every database has exactly one of. */
export function titleProperty(database: Database): Property | undefined {
  return database.properties.find((property) => property.type === "title");
}

export function optionOf(property: Property, id: unknown): PropertyOption | undefined {
  return typeof id === "string" ? property.options?.find((option) => option.id === id) : undefined;
}

/** A column of a view: a property with what the view keeps of it. */
export type Column = { property: Property; visible: boolean; width?: number | null };

/**
 * Every property in the view's order with what the view keeps of it: those
 * the view lists first, then any it does not, as the schema orders them,
 * shown in a table and hidden elsewhere but the title.
 */
export function columnsOf(database: Database, view: View): Column[] {
  const listed = new Map(view.properties.map((entry) => [entry.property, entry]));
  const ordered: Column[] = [];
  for (const entry of view.properties) {
    const property = propertyOf(database, entry.property);
    if (property !== undefined) {
      ordered.push({ property, visible: entry.visible, width: entry.width ?? null });
    }
  }
  for (const property of database.properties) {
    if (!listed.has(property.id)) {
      ordered.push({
        property,
        visible: view.layout === "table" || property.type === "title",
      });
    }
  }
  return ordered;
}

/** The columns a view shows: the title always, first, then the others it shows, in its order. */
export function shownColumns(database: Database, view: View): Column[] {
  const all = columnsOf(database, view);
  const title = all.find((column) => column.property.type === "title");
  const rest = all.filter((column) => column.property.type !== "title" && column.visible);
  return title === undefined ? rest : [{ ...title, visible: true }, ...rest];
}

/** The view's properties as a change sends them, every one in its order. */
export function viewProperties(columns: readonly Column[]): ViewProperty[] {
  return columns.map((column) => ({
    property: column.property.id,
    visible: column.visible,
    ...(column.width === null || column.width === undefined ? {} : { width: column.width }),
  }));
}

/** The columns with one changed, the rest as they are. */
export function withColumn(
  columns: readonly Column[],
  propertyId: string,
  change: Partial<Omit<Column, "property">>,
): Column[] {
  return columns.map((column) =>
    column.property.id === propertyId ? { ...column, ...change } : column,
  );
}

/** The columns with one moved before or after another. */
export function movedColumn(
  columns: readonly Column[],
  propertyId: string,
  target: string,
  side: "before" | "after",
): Column[] {
  const moving = columns.find((column) => column.property.id === propertyId);
  if (moving === undefined || propertyId === target) {
    return [...columns];
  }
  const rest = columns.filter((column) => column.property.id !== propertyId);
  const at = rest.findIndex((column) => column.property.id === target);
  if (at < 0) {
    return [...columns];
  }
  const index = side === "before" ? at : at + 1;
  return [...rest.slice(0, index), moving, ...rest.slice(index)];
}

/** Column widths, in pixels, within what the server keeps. */
export const MIN_WIDTH = 80;
export const MAX_WIDTH = 2000;
export const TITLE_WIDTH = 260;
export const COLUMN_WIDTH = 180;

export function widthOf(column: Column): number {
  return column.width ?? (column.property.type === "title" ? TITLE_WIDTH : COLUMN_WIDTH);
}

export function clampWidth(width: number): number {
  return Math.round(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, width)));
}

/** Hues offered for options: twelve steps around the wheel; no hue is grey. */
export const OPTION_HUES = [0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330] as const;

/** A hue for a new option: the first of the offered ones the property does not use yet. */
export function nextHue(options: readonly { hue?: number | null }[]): number {
  const used = new Set(options.map((option) => option.hue));
  return OPTION_HUES.find((hue) => !used.has(hue)) ?? OPTION_HUES[options.length % 12] ?? 0;
}

/** An option as a change keeps it: by its id, with its name, hue and group. */
export function keptOption(option: PropertyOption): OptionChange {
  return {
    id: option.id,
    name: option.name,
    ...(option.hue === undefined || option.hue === null ? {} : { hue: option.hue }),
    ...(option.group === undefined || option.group === null ? {} : { group: option.group }),
  };
}

/**
 * The options to send for a property whose options were edited from those
 * whose ids are `known`, against the options the server has now, `live`: a
 * change sends every option, and the server clears the values of any left
 * out. So the edited ones go, but for those deleted elsewhere meanwhile, and
 * then those added elsewhere meanwhile, which the edit never had.
 */
export function mergedOptions(
  edited: readonly OptionChange[],
  known: ReadonlySet<string>,
  live: readonly PropertyOption[],
): OptionChange[] {
  const there = new Set(live.map((option) => option.id));
  const kept = edited.filter((option) => {
    const id = option.id ?? null;
    return id === null || there.has(id);
  });
  const added = live.filter((option) => !known.has(option.id)).map(keptOption);
  return [...kept, ...added];
}

/** A name for a new property that no other has, ignoring case: "Property", "Property 2". */
export function freshName(database: Database, base: string): string {
  const taken = new Set(database.properties.map((property) => property.name.toLowerCase()));
  if (!taken.has(base.toLowerCase())) {
    return base;
  }
  for (let n = 2; ; n += 1) {
    const name = `${base} ${n}`;
    if (!taken.has(name.toLowerCase())) {
      return name;
    }
  }
}

const numberFormats = {
  plain: new Intl.NumberFormat(undefined, { maximumFractionDigits: 10 }),
  integer: new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }),
  percent: new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 2 }),
  decimals: new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }),
};

/**
 * A number as its property's format shows it: plain, rounded to a whole
 * number, as a percentage of 1, or with two decimals in its currency.
 */
export function formatNumber(
  value: number,
  format: NumberFormat | null | undefined,
  currency?: string | null,
): string {
  switch (format ?? "plain") {
    case "integer":
      return numberFormats.integer.format(value);
    case "percent":
      return numberFormats.percent.format(value);
    case "currency":
      if (currency) {
        try {
          return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(value);
        } catch {
          return `${numberFormats.decimals.format(value)} ${currency}`;
        }
      }
      return numberFormats.decimals.format(value);
    case "plain":
      return numberFormats.plain.format(value);
  }
}

/** A date value as rows hold it: a day or a moment, and maybe an end. */
export type DateValue = { start: string; end?: string | null };

export function isDateValue(value: unknown): value is DateValue {
  return (
    typeof value === "object" &&
    value !== null &&
    "start" in value &&
    typeof value.start === "string"
  );
}

const DAY = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Whether a date is a whole day, `2026-11-22`, rather than a moment. */
export function isDay(text: string): boolean {
  return DAY.test(text);
}

/** A day as the local midnight of it, so it never shifts with the time zone. */
export function localDay(day: string): Date {
  const [, year, month, date] = DAY.exec(day) ?? [];
  return new Date(Number(year), Number(month) - 1, Number(date));
}

/** How a date property's days show, which each reader picks in their browser. */
export type DateFormat = "medium" | "long" | "numeric";

export const DATE_FORMATS: readonly DateFormat[] = ["medium", "long", "numeric"];

const dayFormats = {
  medium: new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }),
  long: new Intl.DateTimeFormat(undefined, { dateStyle: "full" }),
};
const timeFormat = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });

/** A day, as its local midnight, in one of the formats: numeric is `2026-11-22`. */
export function formatDay(day: Date, format: DateFormat): string {
  return format === "numeric" ? today(day) : dayFormats[format].format(day);
}

/** One end of a date, a day or a moment, in the reader's time zone and the format. */
export function formatDatePart(text: string, format: DateFormat = "medium"): string {
  if (isDay(text)) {
    return formatDay(localDay(text), format);
  }
  const moment = new Date(text);
  if (Number.isNaN(moment.getTime())) {
    return text;
  }
  return `${formatDay(moment, format)} ${timeFormat.format(moment)}`;
}

/** A date value as it reads: its start, and its end after an arrow. */
export function formatDate(value: DateValue, format: DateFormat = "medium"): string {
  const end = value.end ?? null;
  return end === null
    ? formatDatePart(value.start, format)
    : `${formatDatePart(value.start, format)} → ${formatDatePart(end, format)}`;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** Today in the reader's time zone, as a day. */
export function today(now = new Date()): string {
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** A moment as a datetime-local field takes it, in the reader's time zone. */
export function localInput(moment: string): string {
  const at = new Date(moment);
  if (Number.isNaN(at.getTime())) {
    return "";
  }
  return `${today(at)}T${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** What a datetime-local field holds, as an RFC 3339 moment in UTC. */
export function fromLocalInput(local: string): string | undefined {
  const at = new Date(local);
  return Number.isNaN(at.getTime()) ? undefined : at.toISOString();
}

/** The day a date part falls on, for a date field: itself, or the moment's local day. */
export function dayOf(text: string): string {
  return isDay(text) ? text : localInput(text).slice(0, 10);
}

/** A row's value of a property, computed ones from the row itself. */
export function valueOf(row: Row, property: Property): unknown {
  switch (property.type) {
    case "title":
      return row.title === "" ? undefined : row.title;
    case "unique_id":
      return row.unique_id;
    case "created_time":
      return row.created_at;
    case "last_edited_time":
      return row.last_edited_at;
    case "created_by":
      return row.created_by ?? undefined;
    case "last_edited_by":
      return row.last_edited_by ?? undefined;
    default:
      return row.values[property.id];
  }
}

/** The ids a list value holds: a multi-select's options, a person's people. */
export function idsOf(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.filter((entry): entry is string => typeof entry === "string");
  }
  return typeof value === "string" ? [value] : [];
}

/** Whether a value is empty, as a card leaves such a property out. */
export function isEmpty(value: unknown): boolean {
  return (
    value === undefined ||
    value === null ||
    value === "" ||
    value === false ||
    (Array.isArray(value) && value.length === 0)
  );
}

/** The group a row is listed in, in a grouped view's rows. */
const groupOf = (row: Row): GroupKey => row.group ?? null;

/** A card moved on a board: from one group to another, or within one, before a card or last. */
export type CardMove = { rowId: string; from: GroupKey; to: GroupKey; before: string | null };

/**
 * A view's rows with a card moved, as the server will have them once it
 * takes the move: the row's value of the grouping property set, the card in
 * its new place, and the groups' counts. On a board of people, the person of
 * the group it leaves makes room for the one of the group it joins.
 */
export function movedCard(answer: ViewRows, move: CardMove, grouping: Property): ViewRows {
  const entry = answer.rows.find((row) => row.id === move.rowId && groupOf(row) === move.from);
  if (entry === undefined) {
    return answer;
  }
  const values = { ...entry.values };
  if (grouping.type === "person") {
    const people = idsOf(values[grouping.id]).filter((id) => id !== move.from);
    if (move.to !== null && !people.includes(move.to)) {
      people.push(move.to);
    }
    if (people.length === 0) {
      delete values[grouping.id];
    } else {
      values[grouping.id] = people;
    }
  } else if (move.to === null) {
    delete values[grouping.id];
  } else {
    values[grouping.id] = move.to;
  }
  const regroups = move.from !== move.to;
  // A row naming two people is on the board twice: joining a group it is in already merges them.
  const there =
    regroups && answer.rows.some((row) => row.id === move.rowId && groupOf(row) === move.to);
  const rows = answer.rows
    .filter((row) => !(row.id === move.rowId && groupOf(row) === move.from))
    .map((row) => (row.id === move.rowId ? Object.assign({}, row, { values }) : row));
  if (!there) {
    const moved = { ...entry, values, group: move.to };
    const before =
      move.before === null
        ? -1
        : rows.findIndex((row) => row.id === move.before && groupOf(row) === move.to);
    const last = rows.findLastIndex((row) => groupOf(row) === move.to);
    rows.splice(before >= 0 ? before : last >= 0 ? last + 1 : rows.length, 0, moved);
  }
  return {
    rows,
    total: there ? answer.total - 1 : answer.total,
    groups:
      answer.groups?.map((group) => {
        const key = group.key ?? null;
        const left = regroups && key === move.from ? 1 : 0;
        const joined = regroups && key === move.to && !there ? 1 : 0;
        return Object.assign({}, group, { count: group.count - left + joined });
      }) ?? null,
  };
}
