import { ArrowDown, ArrowUp, Plus, Trash2, X } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";
import {
  useAddProperty,
  useChangeProperty,
  useDeleteProperty,
  type AddProperty,
  type NumberFormat,
  type OptionChange,
  type OptionsFrom,
  type Property,
  type PropertyChange,
  type PropertyKind,
  type StatusGroup,
} from "@/api/databases";
import { hasCode } from "@/api/errors";
import type { Placement } from "@/api/pages";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, errorText } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import {
  DATE_FORMATS,
  freshName,
  hasOptions,
  isComputed,
  kindsFor,
  mergedOptions,
  nextHue,
  retypes,
  type DateFormat,
} from "@/lib/database-rules";
import { strings } from "@/lib/strings";
import { rememberDateFormat, useDateFormat } from "./date-format";
import { useDatabaseScope } from "./database-scope";
import { HuePicker } from "./hue-picker";

const copy = strings.databases.properties;

const NUMBER_FORMATS: readonly NumberFormat[] = ["plain", "integer", "percent", "currency"];
const STATUS_GROUPS: readonly StatusGroup[] = ["to_do", "in_progress", "done"];

/** An option as the dialog edits it: one that exists has its id. */
type OptionDraft = {
  key: string;
  id?: string;
  name: string;
  hue: number | null;
  group: StatusGroup | null;
};

let draftKeys = 0;
const optionKey = () => {
  draftKeys += 1;
  return `option-${draftKeys}`;
};

/** The three a new status starts with, as the server gives one without options. */
function statusDefaults(): OptionDraft[] {
  return [
    { key: optionKey(), name: "Not started", hue: null, group: "to_do" },
    { key: optionKey(), name: "In progress", hue: 240, group: "in_progress" },
    { key: optionKey(), name: "Done", hue: 150, group: "done" },
  ];
}

function draftsOf(property: Property | undefined): OptionDraft[] {
  return (property?.options ?? []).map((option) => ({
    key: option.id,
    id: option.id,
    name: option.name,
    hue: option.hue ?? null,
    group: option.group ?? null,
  }));
}

function changesOf(drafts: readonly OptionDraft[], kind: PropertyKind): OptionChange[] {
  return drafts.map((draft) => ({
    ...(draft.id === undefined ? {} : { id: draft.id }),
    name: draft.name.trim(),
    ...(draft.hue === null ? {} : { hue: draft.hue }),
    ...(kind === "status" ? { group: draft.group ?? "to_do" } : {}),
  }));
}

type PropertyDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The property to edit; a new one is made without it. */
  property?: Property | undefined;
  /** Where a new one goes among the properties. */
  placement?: Placement | undefined;
  /** The field to start in. */
  focus?: "name" | "type";
  /** Told of a new property once it is made, as a view shows it where it was asked for. */
  onAdded?: (property: Property) => void;
};

/**
 * A property's name, type and settings: a number's format and currency, a
 * unique id's prefix, how dates show, and the options of a select, a
 * multi-select or a status, each with its colour and, for a status, its
 * group. A change of type that loses values asks first. The same dialog makes
 * a new property.
 */
export function PropertyDialog({
  open,
  onOpenChange,
  property,
  placement,
  focus = "name",
  onAdded,
}: PropertyDialogProps) {
  const workspace = useWorkspace();
  const { database, seesPeople } = useDatabaseScope();
  const add = useAddProperty(workspace.id, database.page.id);
  const change = useChangeProperty(workspace.id, database.page.id);
  const remove = useDeleteProperty(workspace.id, database.page.id);
  const [name, setName] = useState(property?.name ?? freshName(database, copy.types.text));
  const [kind, setKind] = useState<PropertyKind>(property?.type ?? "text");
  const [format, setFormat] = useState<NumberFormat>(property?.number_format ?? "plain");
  const [currency, setCurrency] = useState(property?.currency ?? "EUR");
  const [prefix, setPrefix] = useState(property?.prefix ?? "");
  const [options, setOptions] = useState<OptionDraft[]>(() => draftsOf(property));
  // The options the dialog opened with, and whether they were edited: only an edit sends them.
  const [known] = useState(() => new Set((property?.options ?? []).map((option) => option.id)));
  const [optionsEdited, setOptionsEdited] = useState(false);
  const [lossy, setLossy] = useState<{
    body: Omit<PropertyChange, "propertyId">;
    message: string;
  } | null>(null);
  const [deleting, setDeleting] = useState(false);
  const shownFormat = useDateFormat(property?.id ?? "");
  const [dateFormat, setDateFormat] = useState<DateFormat>(shownFormat);
  const typeField = useRef<HTMLSelectElement>(null);
  const nameField = useRef<HTMLInputElement>(null);
  const editing = property !== undefined;
  const isTitle = property?.type === "title";
  const typeFixed = property !== undefined && !retypes(property, seesPeople);
  const retyped = editing && kind !== property.type;
  const pending = add.isPending || change.isPending || remove.isPending;
  const failure = add.error ?? change.error ?? remove.error;

  const close = () => onOpenChange(false);

  const pickKind = (next: PropertyKind) => {
    setKind(next);
    if (!editing && name === freshName(database, copy.types[kind])) {
      setName(freshName(database, copy.types[next]));
    }
    if (!editing && next === "status" && options.length === 0) {
      setOptions(statusDefaults());
    }
  };

  /** The settings that come with a type, which the server takes only beside it. */
  const settings = () => ({
    ...(kind === "number"
      ? { number_format: format, ...(format === "currency" ? { currency: currency.trim() } : {}) }
      : {}),
    ...(kind === "unique_id" && prefix.trim() !== "" ? { prefix: prefix.trim() } : {}),
  });

  const settingsChanged = () =>
    (kind === "number" &&
      (format !== (property?.number_format ?? "plain") ||
        (format === "currency" && currency.trim() !== (property?.currency ?? "")))) ||
    (kind === "unique_id" && prefix.trim() !== (property?.prefix ?? ""));

  const sendChange = (body: Omit<PropertyChange, "propertyId">) => {
    if (property === undefined) {
      return;
    }
    change.mutate(
      { propertyId: property.id, ...body },
      {
        onSuccess: close,
        onError: (error) => {
          if (hasCode(error, "lossy_change")) {
            change.reset();
            setLossy({ body, message: errorText(error) });
          }
        },
      },
    );
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (!editing) {
      const body: AddProperty = {
        name: trimmed,
        type: kind,
        ...settings(),
        ...(hasOptions(kind) ? { options: changesOf(options, kind) } : {}),
        ...(placement === undefined ? {} : { placement }),
      };
      add.mutate(body, {
        onSuccess: (made) => {
          if (kind === "date") {
            rememberDateFormat(made.id, dateFormat);
          }
          onAdded?.(made);
          close();
        },
      });
      return;
    }
    if (kind === "date") {
      rememberDateFormat(property.id, dateFormat);
    }
    const edited = changesOf(options, kind);
    const merged: OptionsFrom = (live) => mergedOptions(edited, known, live);
    const body: Omit<PropertyChange, "propertyId"> = {
      ...(trimmed !== property.name ? { name: trimmed } : {}),
      ...(retyped || settingsChanged() ? { type: kind, ...settings() } : {}),
      ...(!retyped && hasOptions(kind) && optionsEdited ? { options: merged } : {}),
    };
    if (Object.keys(body).length === 0) {
      close();
      return;
    }
    sendChange(body);
  };

  /** Changes the options as drafted, which the change then sends. */
  const editOptions = (edit: (current: OptionDraft[]) => OptionDraft[]) => {
    setOptionsEdited(true);
    setOptions(edit);
  };
  const editOption = (key: string, patch: Partial<OptionDraft>) =>
    editOptions((current) =>
      current.map((option) => (option.key === key ? { ...option, ...patch } : option)),
    );
  const moveOption = (index: number, by: -1 | 1) =>
    editOptions((current) => {
      const next = [...current];
      const [moved] = next.splice(index, 1);
      if (moved !== undefined) {
        next.splice(index + by, 0, moved);
      }
      return next;
    });

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent
          closeLabel={copy.cancel}
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            (focus === "type" ? typeField : nameField).current?.focus();
          }}
        >
          <DialogTitle>{editing ? copy.editTitle(property.name) : copy.addTitle}</DialogTitle>
          <DialogDescription className="sr-only">
            {editing ? copy.editTitle(property.name) : copy.addTitle}
          </DialogDescription>
          <form onSubmit={submit} className="grid gap-4">
            <Field label={copy.name}>
              {(control) => (
                <Input
                  {...control}
                  ref={nameField}
                  required
                  maxLength={100}
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              )}
            </Field>
            <Field
              label={copy.type}
              hint={
                isTitle
                  ? copy.titleType
                  : typeFixed
                    ? copy.personType
                    : isComputed(kind)
                      ? copy.computed
                      : undefined
              }
            >
              {(control) => (
                <NativeSelect
                  {...control}
                  ref={typeField}
                  value={kind}
                  disabled={typeFixed}
                  onChange={(event) => pickKind(event.target.value as PropertyKind)}
                >
                  {kindsFor(database, property).map((entry) => (
                    <option key={entry} value={entry}>
                      {copy.types[entry]}
                    </option>
                  ))}
                </NativeSelect>
              )}
            </Field>

            {kind === "number" && (
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.numberFormat}>
                  {(control) => (
                    <NativeSelect
                      {...control}
                      value={format}
                      onChange={(event) => setFormat(event.target.value as NumberFormat)}
                    >
                      {NUMBER_FORMATS.map((entry) => (
                        <option key={entry} value={entry}>
                          {copy.formats[entry]}
                        </option>
                      ))}
                    </NativeSelect>
                  )}
                </Field>
                {format === "currency" && (
                  <Field label={copy.currency} hint={copy.currencyHint}>
                    {(control) => (
                      <Input
                        {...control}
                        required
                        pattern="[A-Za-z]{3}"
                        maxLength={3}
                        value={currency}
                        onChange={(event) => setCurrency(event.target.value.toUpperCase())}
                      />
                    )}
                  </Field>
                )}
              </div>
            )}

            {kind === "unique_id" && (
              <Field label={copy.prefix} hint={copy.prefixHint}>
                {(control) => (
                  <Input
                    {...control}
                    pattern="[A-Za-z][A-Za-z0-9]{0,9}"
                    maxLength={10}
                    value={prefix}
                    onChange={(event) => setPrefix(event.target.value.toUpperCase())}
                  />
                )}
              </Field>
            )}

            {kind === "date" && (
              <Field label={copy.dateFormat} hint={copy.dateFormatHint}>
                {(control) => (
                  <NativeSelect
                    {...control}
                    value={dateFormat}
                    onChange={(event) => setDateFormat(event.target.value as DateFormat)}
                  >
                    {DATE_FORMATS.map((entry) => (
                      <option key={entry} value={entry}>
                        {copy.dateFormats[entry]}
                      </option>
                    ))}
                  </NativeSelect>
                )}
              </Field>
            )}

            {hasOptions(kind) && !retyped && (
              <fieldset className="grid gap-2">
                <legend className="mb-1.5 text-sm font-medium">{copy.options}</legend>
                {options.length === 0 && (
                  <p className="text-xs text-muted-foreground">{copy.noOptions}</p>
                )}
                <ol className="grid gap-1">
                  {options.map((option, index) => {
                    const label =
                      option.name.trim() === "" ? copy.newOption(index + 1) : option.name;
                    return (
                      <li key={option.key} className="flex flex-wrap items-center gap-1">
                        <HuePicker
                          name={label}
                          hue={option.hue}
                          onPick={(hue) => editOption(option.key, { hue })}
                        />
                        <Input
                          aria-label={copy.optionName(index + 1)}
                          required
                          maxLength={100}
                          value={option.name}
                          onChange={(event) => editOption(option.key, { name: event.target.value })}
                          className="w-auto min-w-0 flex-1"
                        />
                        {kind === "status" && (
                          <span className="w-36">
                            <NativeSelect
                              aria-label={copy.statusGroup(label)}
                              value={option.group ?? "to_do"}
                              onChange={(event) =>
                                editOption(option.key, { group: event.target.value as StatusGroup })
                              }
                            >
                              {STATUS_GROUPS.map((group) => (
                                <option key={group} value={group}>
                                  {copy.statusGroups[group]}
                                </option>
                              ))}
                            </NativeSelect>
                          </span>
                        )}
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          aria-label={copy.moveOptionUp(label)}
                          disabled={index === 0}
                          onClick={() => moveOption(index, -1)}
                        >
                          <ArrowUp aria-hidden />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          aria-label={copy.moveOptionDown(label)}
                          disabled={index === options.length - 1}
                          onClick={() => moveOption(index, 1)}
                        >
                          <ArrowDown aria-hidden />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-sm"
                          aria-label={copy.removeOption(label)}
                          onClick={() =>
                            editOptions((current) =>
                              current.filter((entry) => entry.key !== option.key),
                            )
                          }
                        >
                          <X aria-hidden />
                        </Button>
                      </li>
                    );
                  })}
                </ol>
                <div>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={options.length >= 200}
                    onClick={() =>
                      editOptions((current) => [
                        ...current,
                        {
                          key: optionKey(),
                          name: "",
                          hue: nextHue(current),
                          group: kind === "status" ? "to_do" : null,
                        },
                      ])
                    }
                  >
                    <Plus aria-hidden />
                    {copy.addOption}
                  </Button>
                </div>
              </fieldset>
            )}

            {failure !== null && <ErrorNotice error={failure} />}
            <DialogFooter className="sm:justify-between">
              {editing && !isTitle ? (
                <Button
                  type="button"
                  variant="ghost"
                  disabled={pending}
                  onClick={() => setDeleting(true)}
                >
                  <Trash2 aria-hidden />
                  {copy.delete}
                </Button>
              ) : (
                <span />
              )}
              <div className="flex flex-col-reverse gap-2 sm:flex-row">
                <Button type="button" variant="outline" onClick={close}>
                  {copy.cancel}
                </Button>
                <Button type="submit" disabled={pending}>
                  {pending ? copy.saving : copy.save}
                </Button>
              </div>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      <ConfirmDialog
        open={lossy !== null}
        title={copy.lossyTitle}
        description={lossy?.message ?? ""}
        confirm={copy.lossyConfirm}
        onCancel={() => setLossy(null)}
        onConfirm={() => {
          const body = lossy?.body;
          setLossy(null);
          if (body !== undefined) {
            sendChange({ ...body, confirm: true });
          }
        }}
      />
      {property !== undefined && (
        <ConfirmDialog
          open={deleting}
          title={copy.deleteTitle(property.name)}
          description={copy.deleteLede}
          confirm={copy.delete}
          onCancel={() => setDeleting(false)}
          onConfirm={() => {
            setDeleting(false);
            remove.mutate(property.id, { onSuccess: close });
          }}
        />
      )}
    </>
  );
}
