import { Check, ExternalLink, Plus, X } from "lucide-react";
import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { useChangeProperty, type Property, type PropertyOption } from "@/api/databases";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { fieldStyles, Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import {
  dayOf,
  fromLocalInput,
  idsOf,
  isDateValue,
  isDay,
  keptOption,
  localInput,
  nextHue,
  readNumber,
  today,
  URL_SCHEMES,
  type DateValue,
} from "@/lib/database-rules";
import { assignable, attribution, type Someone } from "@/lib/people";
import { strings } from "@/lib/strings";
import { PersonName } from "./cell-value";
import { useDatabaseScope } from "./database-scope";
import { OptionChip } from "./option-chip";

const copy = strings.databases.cells;

/**
 * What an editor in a cell's popover is given: the value, the way to save a
 * new one (null clears it), and the way to close. `commitRef` is filled with
 * what saves a value still being typed, which the popover runs as it closes
 * by a click outside it. The popover puts the focus in its first field.
 */
export type EditorProps = {
  property: Property;
  value: unknown;
  label: string;
  onSave: (value: unknown) => void;
  onClose: () => void;
  commitRef: RefObject<(() => void) | null>;
};

/** Fills `commit` with what saves the value being typed, as it is after each render. */
function useCommit(commitRef: RefObject<(() => void) | null>, save: () => void) {
  useLayoutEffect(() => {
    commitRef.current = save;
    return () => {
      commitRef.current = null;
    };
  });
}

/** Enter saves and Escape closes without saving, in one-line fields. */
function fieldKeys(event: KeyboardEvent, save: () => void, close: () => void) {
  if (event.key === "Enter" && !event.nativeEvent.isComposing) {
    event.preventDefault();
    save();
  } else if (event.key === "Escape") {
    event.preventDefault();
    close();
  }
}

/**
 * A text or a title: Enter saves, Shift+Enter starts a new line in a text, and
 * leaving the popover saves too. An empty text clears the value.
 */
export function TextEditor({ property, value, label, onSave, onClose, commitRef }: EditorProps) {
  const start = typeof value === "string" ? value : "";
  const [text, setText] = useState(start);
  const sent = useRef(false);
  const save = () => {
    if (sent.current) {
      return;
    }
    sent.current = true;
    const next = property.type === "title" ? text.replaceAll(/[\r\n]+/g, " ") : text;
    if (next !== start) {
      onSave(next === "" && property.type !== "title" ? null : next);
    }
  };
  useCommit(commitRef, save);
  const close = () => {
    sent.current = true;
    onClose();
  };
  if (property.type === "title") {
    return (
      <Input
        aria-label={label}
        maxLength={500}
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) =>
          fieldKeys(
            event,
            () => {
              save();
              onClose();
            },
            close,
          )
        }
      />
    );
  }
  return (
    <Textarea
      aria-label={label}
      rows={4}
      maxLength={10000}
      value={text}
      onChange={(event) => setText(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
          event.preventDefault();
          save();
          onClose();
        } else if (event.key === "Escape") {
          event.preventDefault();
          close();
        }
      }}
    />
  );
}

/**
 * A number, typed as text so any locale's keyboard works, read with this
 * browser's decimal separator; empty clears it.
 */
export function NumberEditor({ value, label, onSave, onClose, commitRef }: EditorProps) {
  const start = typeof value === "number" ? String(value) : "";
  const [text, setText] = useState(start);
  const [invalid, setInvalid] = useState(false);
  const errorId = useId();
  const sent = useRef(false);
  /** Saves what is typed; false when it is not a number, which stays open to fix. */
  const save = (): boolean => {
    if (sent.current) {
      return true;
    }
    if (text.trim() === "") {
      sent.current = true;
      if (start !== "") {
        onSave(null);
      }
      return true;
    }
    const number = readNumber(text);
    if (number === undefined) {
      setInvalid(true);
      return false;
    }
    sent.current = true;
    if (String(number) !== start) {
      onSave(number);
    }
    return true;
  };
  useCommit(commitRef, () => void save());
  return (
    <div className="grid gap-1.5">
      <Input
        aria-label={label}
        inputMode="decimal"
        value={text}
        aria-invalid={invalid}
        {...(invalid ? { "aria-describedby": errorId } : {})}
        onChange={(event) => {
          setText(event.target.value);
          setInvalid(false);
        }}
        onKeyDown={(event) =>
          fieldKeys(
            event,
            () => {
              if (save()) {
                onClose();
              }
            },
            () => {
              sent.current = true;
              onClose();
            },
          )
        }
      />
      {invalid && (
        <p id={errorId} className="text-xs text-bad">
          {copy.numberInvalid}
        </p>
      )}
    </div>
  );
}

/** A link, with the way to open it; empty clears it. */
export function UrlEditor({ value, label, onSave, onClose, commitRef }: EditorProps) {
  const start = typeof value === "string" ? value : "";
  const [text, setText] = useState(start);
  const [invalid, setInvalid] = useState(false);
  const errorId = useId();
  const sent = useRef(false);
  const save = (): boolean => {
    if (sent.current) {
      return true;
    }
    const next = text.trim();
    if (next !== "" && !URL_SCHEMES.test(next)) {
      setInvalid(true);
      return false;
    }
    sent.current = true;
    if (next !== start) {
      onSave(next === "" ? null : next);
    }
    return true;
  };
  useCommit(commitRef, () => void save());
  return (
    <div className="grid gap-1.5">
      <Input
        type="url"
        aria-label={label}
        value={text}
        aria-invalid={invalid}
        {...(invalid ? { "aria-describedby": errorId } : {})}
        onChange={(event) => {
          setText(event.target.value);
          setInvalid(false);
        }}
        onKeyDown={(event) =>
          fieldKeys(
            event,
            () => {
              if (save()) {
                onClose();
              }
            },
            () => {
              sent.current = true;
              onClose();
            },
          )
        }
      />
      {invalid && (
        <p id={errorId} className="text-xs text-bad">
          {copy.urlInvalid}
        </p>
      )}
      {start !== "" && URL_SCHEMES.test(start) && (
        <a
          href={start}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex min-h-(--tap-sm) items-center gap-1.5 text-sm text-primary underline-offset-4 hover:underline"
        >
          <ExternalLink aria-hidden className="size-4" />
          {copy.openLink(start)}
        </a>
      )}
    </div>
  );
}

/**
 * The options of a select, a status or a multi-select to pick from, found by
 * typing. A select or a status takes one and closes; a multi-select toggles
 * each. Those who edit the database make a new option of what they typed.
 */
export function OptionEditor({ property, value, label, onSave, onClose }: EditorProps) {
  const workspace = useWorkspace();
  const { database, editsDatabase, report } = useDatabaseScope();
  const change = useChangeProperty(workspace.id, database.page.id);
  const [query, setQuery] = useState("");
  const listId = useId();
  const multiple = property.type === "multi_select";
  const options = property.options ?? [];
  const chosen = idsOf(value);
  const needle = query.trim().toLowerCase();
  const shown =
    needle === ""
      ? options
      : options.filter((option) => option.name.toLowerCase().includes(needle));
  const exact = options.some((option) => option.name.toLowerCase() === needle);
  const creates = editsDatabase && property.type !== "status" && needle !== "" && !exact;

  const pick = (id: string) => {
    if (multiple) {
      const next = chosen.includes(id) ? chosen.filter((entry) => entry !== id) : [...chosen, id];
      onSave(next.length === 0 ? null : next);
    } else {
      if (chosen[0] !== id) {
        onSave(id);
      }
      onClose();
    }
  };

  /**
   * Adds an option of what was typed to those the server has as it is sent,
   * so none added elsewhere meanwhile is left out; one of that name made
   * elsewhere meanwhile is picked as it is.
   */
  const create = () => {
    const name = query.trim();
    const withNew = (live: readonly PropertyOption[]) =>
      live.some((option) => option.name.toLowerCase() === name.toLowerCase())
        ? live.map(keptOption)
        : [...live.map(keptOption), { name, hue: nextHue(live) }];
    change.mutate(
      { propertyId: property.id, options: withNew },
      {
        onSuccess: (changed) => {
          const made = changed.options?.find(
            (option) => option.name.toLowerCase() === name.toLowerCase(),
          );
          setQuery("");
          if (made !== undefined) {
            pick(made.id);
          }
        },
        onError: report,
      },
    );
  };

  return (
    <div className="grid gap-2">
      <Input
        aria-label={editsDatabase && property.type !== "status" ? copy.search : copy.searchOnly}
        value={query}
        aria-controls={listId}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            const only = shown.length === 1 ? shown[0] : undefined;
            if (only !== undefined) {
              pick(only.id);
            } else if (creates) {
              create();
            }
          }
        }}
      />
      <ul id={listId} aria-label={label} className="grid max-h-64 gap-0.5 overflow-y-auto">
        {shown.map((option) => {
          const selected = chosen.includes(option.id);
          return (
            <li key={option.id}>
              <button
                type="button"
                aria-pressed={selected}
                onClick={() => pick(option.id)}
                className="flex min-h-(--tap-sm) w-full items-center gap-2 rounded-md px-2 text-left hover:bg-accent"
              >
                <OptionChip option={option} />
                {selected && <Check aria-hidden className="ml-auto size-4 text-primary" />}
              </button>
            </li>
          );
        })}
      </ul>
      {options.length === 0 && !creates && (
        <p className="text-xs text-muted-foreground">{copy.noOptions}</p>
      )}
      {options.length > 0 && shown.length === 0 && !creates && (
        <p className="text-xs text-muted-foreground">{copy.noMatch}</p>
      )}
      <div className="flex flex-wrap gap-1">
        {creates && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={change.isPending}
            onClick={create}
          >
            <Plus aria-hidden />
            {copy.create(query.trim())}
          </Button>
        )}
        {chosen.length > 0 && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              onSave(null);
              onClose();
            }}
          >
            <X aria-hidden />
            {copy.clear}
          </Button>
        )}
      </div>
    </div>
  );
}

/** A day as the start of a date that has a time: its local midnight. */
const atMidnight = (day: string) => fromLocalInput(`${day}T00:00`) ?? day;

/**
 * A date, with the browser's own date fields, which every platform makes
 * accessible and phones open their native picker for: a day, or a moment with
 * a time, and an end when asked for. It is saved once it is entered, with
 * Enter or as the popover closes, not as each part is typed: a field fills a
 * year a digit at a time, each a date of its own.
 */
export function DateEditor({ value, label, onSave, onClose, commitRef }: EditorProps) {
  const current: DateValue | null = isDateValue(value) ? value : null;
  const [start, setStart] = useState(current?.start ?? "");
  const [end, setEnd] = useState<string | null>(current?.end ?? null);
  const [withTime, setWithTime] = useState(current !== null && !isDay(current.start));
  const ids = { start: useId(), end: useId(), time: useId(), range: useId(), error: useId() };
  const sent = useRef(false);
  const backwards = end !== null && end !== "" && start !== "" && new Date(end) < new Date(start);

  /** Saves the date as the fields hold it, once it has a start and its end is not before it. */
  const save = (): boolean => {
    if (sent.current) {
      return true;
    }
    if (start === "" || backwards) {
      return false;
    }
    sent.current = true;
    const next = end === null || end === "" ? { start } : { start, end };
    if (
      current === null ||
      current.start !== next.start ||
      (current.end ?? null) !== (next.end ?? null)
    ) {
      onSave(next);
    }
    return true;
  };
  useCommit(commitRef, () => void save());

  const read = (text: string) => (withTime ? (fromLocalInput(text) ?? "") : text);
  const show = (text: string) => (text === "" ? "" : withTime ? localInput(text) : dayOf(text));
  const kind = withTime ? "datetime-local" : "date";
  const keys = (event: KeyboardEvent) =>
    fieldKeys(
      event,
      () => {
        if (save()) {
          onClose();
        }
      },
      () => {
        sent.current = true;
        onClose();
      },
    );

  const switchTime = (on: boolean) => {
    setWithTime(on);
    const convert = (text: string) =>
      text === "" ? "" : on ? atMidnight(dayOf(text)) : dayOf(text);
    setStart(convert(start));
    setEnd(end === null ? null : convert(end));
  };

  return (
    <fieldset aria-label={label} className="grid gap-3">
      <div className="grid gap-1">
        <label htmlFor={ids.start} className="text-xs text-muted-foreground">
          {copy.start}
        </label>
        <input
          id={ids.start}
          type={kind}
          value={show(start)}
          onChange={(event) => setStart(read(event.target.value))}
          onKeyDown={keys}
          className={`${fieldStyles} h-(--tap)`}
        />
      </div>
      {end !== null && (
        <div className="grid gap-1">
          <label htmlFor={ids.end} className="text-xs text-muted-foreground">
            {copy.end}
          </label>
          <input
            id={ids.end}
            type={kind}
            value={show(end)}
            min={show(start)}
            aria-invalid={backwards}
            {...(backwards ? { "aria-describedby": ids.error } : {})}
            onChange={(event) => setEnd(read(event.target.value))}
            onKeyDown={keys}
            className={`${fieldStyles} h-(--tap)`}
          />
          {backwards && (
            <p id={ids.error} className="text-xs text-bad">
              {copy.endBeforeStart}
            </p>
          )}
        </div>
      )}
      <div className="grid gap-1">
        <label className="flex min-h-(--tap-sm) cursor-pointer items-center gap-2 text-sm">
          <Checkbox
            id={ids.range}
            checked={end !== null}
            onCheckedChange={(checked) => setEnd(checked === true ? start || today() : null)}
          />
          {copy.withEnd}
        </label>
        <label className="flex min-h-(--tap-sm) cursor-pointer items-center gap-2 text-sm">
          <Checkbox
            id={ids.time}
            checked={withTime}
            onCheckedChange={(checked) => switchTime(checked === true)}
          />
          {copy.withTime}
        </label>
      </div>
      <div className="flex flex-wrap gap-1">
        <Button
          type="button"
          size="sm"
          disabled={start === "" || backwards}
          onClick={() => {
            if (save()) {
              onClose();
            }
          }}
        >
          <Check aria-hidden />
          {copy.setDate}
        </Button>
        {current !== null && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              sent.current = true;
              onSave(null);
              onClose();
            }}
          >
            <X aria-hidden />
            {copy.clear}
          </Button>
        )}
      </div>
    </fieldset>
  );
}

/**
 * The workspace's people and agents to pick from, found by typing, an agent
 * by its name or its owner's; each toggles, and saves at once. Integrations
 * are given no work, and the deleted none either.
 */
export function PersonEditor({ value, label, onSave, onClose }: EditorProps) {
  const { people } = useDatabaseScope();
  const [query, setQuery] = useState("");
  const listId = useId();
  const chosen = idsOf(value);
  const needle = query.trim().toLowerCase();
  const everyone: Someone[] = [...people.values()]
    .filter(assignable)
    .toSorted((a, b) => a.display_name.localeCompare(b.display_name));
  // Those chosen once who can no longer be, a deleted agent or someone who
  // left, come first, to be taken off.
  const offered = new Set(everyone.map((person) => person.id));
  const former = chosen
    .filter((id) => !offered.has(id))
    .map((id) => ({ id, person: people.get(id) }));
  const matches = (person: Someone | undefined) =>
    needle === "" ||
    (person !== undefined &&
      `${attribution(person)} ${person.email ?? ""}`.toLowerCase().includes(needle));
  const shown = [
    ...former.filter((entry) => matches(entry.person)),
    ...everyone.filter(matches).map((person) => ({ id: person.id, person })),
  ];
  const toggle = (id: string) => {
    const next = chosen.includes(id) ? chosen.filter((entry) => entry !== id) : [...chosen, id];
    onSave(next.length === 0 ? null : next);
  };
  return (
    <div className="grid gap-2">
      <Input
        aria-label={copy.findPerson}
        value={query}
        aria-controls={listId}
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && shown.length === 1 && shown[0] !== undefined) {
            event.preventDefault();
            toggle(shown[0].id);
          } else if (event.key === "Escape") {
            onClose();
          }
        }}
      />
      <ul id={listId} aria-label={label} className="grid max-h-64 gap-0.5 overflow-y-auto">
        {shown.map(({ id, person }) => {
          const selected = chosen.includes(id);
          return (
            <li key={id}>
              <button
                type="button"
                aria-pressed={selected}
                onClick={() => toggle(id)}
                className="flex min-h-(--tap-sm) w-full items-center gap-2 rounded-md px-2 text-left hover:bg-accent"
              >
                <PersonName person={person} />
                {selected && <Check aria-hidden className="ml-auto size-4 text-primary" />}
              </button>
            </li>
          );
        })}
      </ul>
      {shown.length === 0 && <p className="text-xs text-muted-foreground">{copy.noPeople}</p>}
    </div>
  );
}
