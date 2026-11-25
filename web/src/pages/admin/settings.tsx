import { useQuery } from "@tanstack/react-query";
import { Lock, RotateCcw } from "lucide-react";
import { useState } from "react";
import { settingsQuery, useUpdateSettings, type Setting } from "@/api/admin";
import { ApiError, hasCode } from "@/api/errors";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field, type FieldControl } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { PageSection } from "@/components/settings/page-section";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { strings } from "@/lib/strings";
import { MailStatusPanel } from "./mail-status";

const copy = strings.admin.settings;

/** A change waiting to be saved: a new value, or null to go back to the default. */
type Drafts = Readonly<Record<string, unknown>>;

/** A group's heading: its own words, or the prefix with a capital, such as Instance. */
function groupTitle(prefix: string): string {
  return copy.groups[prefix] ?? prefix.charAt(0).toUpperCase() + prefix.slice(1);
}

/** The settings in groups by the part of their key before the first dot, in key order. */
function groupByPrefix(settings: readonly Setting[]): [string, Setting[]][] {
  const groups = new Map<string, Setting[]>();
  for (const setting of settings) {
    const prefix = setting.key.split(".")[0] ?? setting.key;
    groups.set(prefix, [...(groups.get(prefix) ?? []), setting]);
  }
  return [...groups];
}

/**
 * Every runtime setting, where its value comes from, and a control for those
 * not pinned by the environment or the config file. Changes collect until
 * saved together; the server takes all of them or none, and names each one
 * it refused.
 */
export function SettingsPage() {
  const settings = useQuery(settingsQuery);
  const save = useUpdateSettings();
  const [drafts, setDrafts] = useState<Drafts>({});
  const [localProblems, setLocalProblems] = useState<Record<string, string>>({});
  const pending = Object.keys(drafts).length;

  const setDraft = (setting: Setting, value: unknown) => {
    save.reset();
    setLocalProblems((problems) => without(problems, setting.key));
    setDrafts((current) =>
      value !== null && sameValue(setting, value)
        ? without(current, setting.key)
        : { ...current, [setting.key]: value },
    );
  };

  const submit = () => {
    const changes: Record<string, unknown> = {};
    const problems: Record<string, string> = {};
    for (const setting of settings.data?.settings ?? []) {
      if (!(setting.key in drafts)) {
        continue;
      }
      const draft = drafts[setting.key];
      if (setting.kind === "integer" && draft !== null) {
        const text = String(draft).trim();
        if (!/^-?\d+$/.test(text)) {
          problems[setting.key] = strings.validation.integer;
          continue;
        }
        changes[setting.key] = Number(text);
      } else {
        changes[setting.key] = draft;
      }
    }
    setLocalProblems(problems);
    if (Object.keys(problems).length > 0) {
      return;
    }
    save.mutate(changes, { onSuccess: () => setDrafts({}) });
  };

  const refused = hasCode(save.error, "invalid_settings");
  const problemFor = (key: string) =>
    localProblems[key] ?? (save.error instanceof ApiError ? save.error.problemFor(key) : undefined);

  return (
    <div className="grid gap-10">
      <p className="max-w-[62ch] text-sm text-muted-foreground">{copy.hint}</p>

      {settings.isPending && <Loading />}
      {settings.isError && (
        <ErrorNotice
          error={settings.error}
          retrying={settings.isFetching}
          onRetry={() => void settings.refetch()}
        />
      )}

      {settings.data &&
        groupByPrefix(settings.data.settings).map(([prefix, group]) => (
          <PageSection key={prefix} title={groupTitle(prefix)}>
            {prefix === "mail" && <MailStatusPanel mail={settings.data.mail} />}
            <ul className="grid gap-6">
              {group.map((setting) => (
                <SettingRow
                  key={setting.key}
                  setting={setting}
                  hasDraft={setting.key in drafts}
                  draft={drafts[setting.key]}
                  problem={problemFor(setting.key)}
                  onChange={(value) => setDraft(setting, value)}
                />
              ))}
            </ul>
          </PageSection>
        ))}

      {(pending > 0 || save.isError || save.isSuccess) && (
        <div className="sticky bottom-0 -mx-2 grid gap-3 border-t border-border bg-background/95 px-2 pt-3 safe-bottom-3">
          {refused && (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.invalid}</p>
            </Alert>
          )}
          {save.isError && !refused && <ErrorNotice error={save.error} />}
          <div className="flex flex-wrap items-center gap-3">
            <output className="mr-auto block text-sm text-muted-foreground">
              {pending > 0 ? copy.unsaved(pending) : save.isSuccess ? copy.saved : ""}
            </output>
            {pending > 0 && (
              <>
                <Button
                  variant="ghost"
                  onClick={() => {
                    save.reset();
                    setLocalProblems({});
                    setDrafts({});
                  }}
                >
                  {copy.discard}
                </Button>
                <Button disabled={save.isPending} onClick={submit}>
                  {save.isPending ? copy.saving : copy.save}
                </Button>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function without<T>(record: Readonly<Record<string, T>>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([name]) => name !== key));
}

/** Whether a typed value is what the setting holds already, so it is no change. */
function sameValue(setting: Setting, value: unknown): boolean {
  if (setting.kind === "secret") {
    return value === "";
  }
  if (setting.kind === "integer") {
    return String(value).trim() === String(setting.value);
  }
  if (setting.kind === "set") {
    return picked(value).join(",") === picked(setting.value).join(",");
  }
  return value === setting.value;
}

/** A set's options as a value holds them; nothing for a value that is not a list. */
function picked(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

/** A value as the settings reference writes it, for the default's line. */
function shown(value: unknown): string {
  if (typeof value === "boolean") {
    return value ? copy.on : copy.off;
  }
  if (Array.isArray(value)) {
    return value.length === 0 ? copy.none : picked(value).join(", ");
  }
  return value === "" || value === null || value === undefined ? "—" : String(value);
}

type SettingRowProps = {
  setting: Setting;
  hasDraft: boolean;
  draft: unknown;
  problem: string | undefined;
  onChange: (value: unknown) => void;
};

function SettingRow({ setting, hasDraft, draft, problem, onChange }: SettingRowProps) {
  const pinned = setting.pinned;
  const resetting = hasDraft && draft === null;
  const current = hasDraft && draft !== null ? draft : setting.value;
  const canReset = !pinned && !resetting && setting.source === "database";

  const hint = (
    <>
      {setting.description} {setting.kind === "duration" && `${copy.durationHint} `}
      {setting.kind === "secret" ? copy.secretHint : copy.defaultValue(shown(setting.default))}
      {resetting && ` ${copy.willReset}`}
    </>
  );

  return (
    <li className="grid gap-2">
      <Field
        label={<span className="font-mono text-sm">{setting.key}</span>}
        aside={
          <ChipList>
            <Chip>{copy.sources[setting.source]}</Chip>
            {pinned && (
              <Chip tone="strong">
                <Lock aria-hidden className="size-3" />
                {copy.pinned}
              </Chip>
            )}
          </ChipList>
        }
        hint={hint}
        error={problem}
        className="max-w-xl"
      >
        {(control, labelId) => (
          <SettingControl
            setting={setting}
            control={control}
            labelId={labelId}
            value={current}
            disabled={pinned}
            onChange={onChange}
          />
        )}
      </Field>
      {pinned && (
        <p className="text-xs text-muted-foreground">
          {setting.source === "yaml" ? copy.pinnedYaml : copy.pinnedBy(setting.env_var)}
        </p>
      )}
      {canReset && (
        <Button
          variant="ghost"
          size="sm"
          className="justify-self-start"
          aria-label={copy.resetLabel(setting.key)}
          onClick={() => onChange(null)}
        >
          <RotateCcw aria-hidden />
          {copy.reset}
        </Button>
      )}
    </li>
  );
}

type SettingControlProps = {
  setting: Setting;
  control: FieldControl;
  labelId: string;
  value: unknown;
  disabled: boolean;
  onChange: (value: unknown) => void;
};

/** The control for a setting's kind; a secret's value never comes back, so its field starts empty. */
function SettingControl({
  setting,
  control,
  labelId,
  value,
  disabled,
  onChange,
}: SettingControlProps) {
  switch (setting.kind) {
    case "bool":
      return (
        <ToggleGroup
          type="single"
          aria-labelledby={labelId}
          {...(control["aria-describedby"] === undefined
            ? {}
            : { "aria-describedby": control["aria-describedby"] })}
          disabled={disabled}
          value={value === true ? "on" : "off"}
          onValueChange={(next) => {
            if (next === "on" || next === "off") {
              onChange(next === "on");
            }
          }}
        >
          <ToggleGroupItem value="on">{copy.on}</ToggleGroupItem>
          <ToggleGroupItem value="off">{copy.off}</ToggleGroupItem>
        </ToggleGroup>
      );
    case "set": {
      const options = setting.options ?? [];
      return (
        <ToggleGroup
          type="multiple"
          aria-labelledby={labelId}
          {...(control["aria-describedby"] === undefined
            ? {}
            : { "aria-describedby": control["aria-describedby"] })}
          disabled={disabled}
          value={picked(value)}
          onValueChange={(next) => onChange(options.filter((option) => next.includes(option)))}
        >
          {options.map((option) => (
            <ToggleGroupItem key={option} value={option}>
              {option}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      );
    }
    case "enum":
      return (
        <NativeSelect
          {...control}
          name={setting.key}
          disabled={disabled}
          value={String(value)}
          onChange={(event) => onChange(event.target.value)}
        >
          {(setting.options ?? []).map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </NativeSelect>
      );
    case "secret":
      return (
        <Input
          {...control}
          name={setting.key}
          type="password"
          autoComplete="off"
          disabled={disabled}
          placeholder={setting.value === "" ? copy.secretUnset : copy.secretSet}
          value={value === setting.value ? "" : String(value ?? "")}
          onChange={(event) => onChange(event.target.value)}
        />
      );
    default:
      return (
        <Input
          {...control}
          name={setting.key}
          {...(setting.kind === "integer" ? { inputMode: "numeric" as const } : {})}
          autoComplete="off"
          spellCheck={false}
          disabled={disabled}
          {...(setting.kind === "string" ? {} : { className: "font-mono" })}
          value={String(value ?? "")}
          onChange={(event) => onChange(event.target.value)}
        />
      );
  }
}
