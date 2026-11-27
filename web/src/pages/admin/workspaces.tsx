import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import {
  adminWorkspacesQuery,
  settingsQuery,
  useAdminDeleteWorkspace,
  useSetWorkspaceLimits,
  type AdminWorkspace,
  type SetLimits,
} from "@/api/admin";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { TypedConfirmDialog } from "@/components/feedback/typed-confirm-dialog";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { formatBytes, formatDate } from "@/lib/format";
import { strings } from "@/lib/strings";
import { countProblem } from "@/lib/validation";

const copy = strings.admin.workspaces;

const KINDS = ["members", "storage_bytes", "agents"] as const;
type Kind = (typeof KINDS)[number];

/** How a count of each kind reads: people and agents as numbers, storage in bytes. */
function amount(kind: Kind, value: number): string {
  return kind === "storage_bytes" ? formatBytes(value) : String(value);
}

/** A limit as it reads, or undefined for none: absent, or 0 for one set here. */
function limitOf(kind: Kind, limit: number | null | undefined): string | undefined {
  return limit === undefined || limit === null || limit === 0 ? undefined : amount(kind, limit);
}

/** A limit as it reads on its own, "no limit" for none. */
function limitText(kind: Kind, limit: number | null | undefined): string {
  return limitOf(kind, limit) ?? copy.unlimited;
}

/**
 * Every workspace on the instance, for instance admins: who owns it, how many
 * are in it, its limits and usage, and deleting it. Names are not links: being
 * an instance admin opens no workspace's content.
 */
export function WorkspacesAdminPage() {
  const workspaces = useInfiniteQuery(adminWorkspacesQuery);
  const [done, setDone] = useState<string>();
  const list = workspaces.data?.pages.flatMap((page) => page.workspaces) ?? [];

  return (
    <PageSection title={copy.title} hint={copy.hint}>
      <output className="block text-sm text-muted-foreground empty:hidden">{done ?? ""}</output>
      {workspaces.isPending && <Loading />}
      {workspaces.isError && (
        <ErrorNotice
          error={workspaces.error}
          retrying={workspaces.isFetching}
          onRetry={() => void workspaces.refetch()}
        />
      )}
      {workspaces.isSuccess && list.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.none}</p>
      )}
      {list.length > 0 && (
        <ul className="grid gap-3">
          {list.map((workspace) => (
            <WorkspaceCard key={workspace.id} workspace={workspace} onDone={setDone} />
          ))}
        </ul>
      )}
      {workspaces.hasNextPage && (
        <Button
          variant="outline"
          className="justify-self-center"
          disabled={workspaces.isFetchingNextPage}
          onClick={() => void workspaces.fetchNextPage()}
        >
          {copy.loadMore}
        </Button>
      )}
    </PageSection>
  );
}

function WorkspaceCard({
  workspace,
  onDone,
}: {
  workspace: AdminWorkspace;
  onDone: (message: string) => void;
}) {
  const remove = useAdminDeleteWorkspace();
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const { limits, usage } = workspace;

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        <p className="font-medium break-words">{workspace.name}</p>
        <p className="font-mono text-sm break-all text-muted-foreground">/w/{workspace.slug}</p>
        <ChipList>
          <Chip>{copy.members(workspace.member_count)}</Chip>
          <Chip>{copy.created(formatDate(workspace.created_at))}</Chip>
        </ChipList>
        <div className="grid gap-1 text-sm">
          <p className="text-muted-foreground">{copy.owners}</p>
          {workspace.owners.length === 0 ? (
            <p>{copy.noOwner}</p>
          ) : (
            <ul className="grid gap-0.5">
              {workspace.owners.map((owner) => (
                <li key={owner.id} className="flex flex-wrap gap-x-2">
                  <span>{owner.display_name}</span>
                  <span className="break-all text-muted-foreground">{owner.email}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
        <KeyValueList className="max-w-xl text-sm" aria-label={copy.limits}>
          {KINDS.map((kind) => (
            <KeyValueRow key={kind} label={copy.kinds[kind]}>
              <span className="grid">
                <span>
                  {copy.usage(amount(kind, usage[kind]), limitOf(kind, limits[kind].effective))}
                </span>
                <span className="text-xs text-muted-foreground">
                  {limits[kind].own === undefined || limits[kind].own === null
                    ? copy.byDefault
                    : copy.own}
                </span>
              </span>
            </KeyValueRow>
          ))}
        </KeyValueList>
        {editing && (
          <LimitsForm
            workspace={workspace}
            onClose={() => setEditing(false)}
            onSaved={() => {
              setEditing(false);
              onDone(copy.saved(workspace.name));
            }}
          />
        )}
        {remove.isError && <ErrorNotice error={remove.error} />}
      </div>
      <ItemActions>
        {!editing && (
          <Button
            variant="outline"
            size="sm"
            aria-label={copy.editLabel(workspace.name)}
            onClick={() => setEditing(true)}
          >
            {copy.edit}
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          disabled={remove.isPending}
          aria-label={copy.deleteLabel(workspace.name)}
          onClick={() => setConfirming(true)}
        >
          {copy.delete}
        </Button>
      </ItemActions>
      <TypedConfirmDialog
        open={confirming}
        title={copy.deleteTitle(workspace.name)}
        description={copy.deleteLede}
        name={workspace.name}
        confirm={copy.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          remove.mutate(workspace.id, { onSuccess: () => onDone(copy.deleted(workspace.name)) });
        }}
      />
    </ItemCard>
  );
}

const UNITS = { B: 1, MiB: 1024 ** 2, GiB: 1024 ** 3 } as const;
type Unit = keyof typeof UNITS;

/** The largest unit a stored size divides into exactly, so the form shows it as it is. */
function unitOf(bytes: number): Unit {
  if (bytes > 0 && bytes % UNITS.GiB === 0) {
    return "GiB";
  }
  if (bytes > 0 && bytes % UNITS.MiB === 0) {
    return "MiB";
  }
  return bytes === 0 ? "GiB" : "B";
}

/** What a typed limit sends: blank follows the default, as null; else the number, in its unit. */
function limitValue(typed: string, unit: number): number | null {
  return typed.trim() === "" ? null : Number(typed) * unit;
}

type Draft = Record<Kind, string>;
type Problems = Partial<Record<Kind, string | undefined>>;

/**
 * Each limit as a number for this workspace, 0 for none, or blank to follow
 * the instance's default, whose value the settings give.
 */
function LimitsForm({
  workspace,
  onClose,
  onSaved,
}: {
  workspace: AdminWorkspace;
  onClose: () => void;
  onSaved: () => void;
}) {
  const save = useSetWorkspaceLimits();
  const settings = useQuery(settingsQuery);
  const ownStorage = workspace.limits.storage_bytes.own ?? undefined;
  const [unit, setUnit] = useState<Unit>(() => unitOf(ownStorage ?? 0));
  const [draft, setDraft] = useState<Draft>(() => ({
    members: String(workspace.limits.members.own ?? ""),
    storage_bytes: ownStorage === undefined ? "" : String(ownStorage / UNITS[unitOf(ownStorage)]),
    agents: String(workspace.limits.agents.own ?? ""),
  }));
  const [problems, setProblems] = useState<Problems>({});

  const fallback = (kind: Kind): string | undefined => {
    const value = settings.data?.settings.find(
      (setting) => setting.key === `workspaces.limits.${kind}`,
    )?.value;
    return typeof value === "number" ? limitText(kind, value) : undefined;
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    save.reset();
    const found: Problems = Object.fromEntries(
      KINDS.map((kind) => [kind, countProblem(draft[kind])]),
    );
    setProblems(found);
    const invalid = KINDS.filter((kind) => found[kind]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const body: SetLimits = {
      members: limitValue(draft.members, 1),
      storage_bytes: limitValue(draft.storage_bytes, UNITS[unit]),
      agents: limitValue(draft.agents, 1),
    };
    save.mutate({ id: workspace.id, ...body }, { onSuccess: onSaved });
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.editTitle(workspace.name)}
      className="grid gap-4 rounded-lg border border-border bg-background p-4"
    >
      <p className="text-sm text-muted-foreground">{copy.editHint}</p>
      {save.isError && <ErrorNotice error={save.error} />}
      {KINDS.map((kind) => {
        const byDefault = fallback(kind);
        return (
          <Field
            key={kind}
            label={copy.kinds[kind]}
            hint={byDefault === undefined ? undefined : copy.followDefault(byDefault)}
            error={problems[kind]}
          >
            {(control) => (
              <span className="flex gap-2">
                <Input
                  {...control}
                  name={kind}
                  type="number"
                  inputMode="numeric"
                  min={0}
                  value={draft[kind]}
                  onChange={(event) => setDraft({ ...draft, [kind]: event.target.value })}
                />
                {kind === "storage_bytes" && (
                  <span className="w-28 shrink-0">
                    <NativeSelect
                      aria-label={copy.unit(copy.kinds.storage_bytes)}
                      value={unit}
                      onChange={(event) =>
                        setUnit(
                          (Object.keys(UNITS) as Unit[]).find(
                            (option) => option === event.target.value,
                          ) ?? "GiB",
                        )
                      }
                    >
                      {(Object.keys(UNITS) as Unit[]).map((option) => (
                        <option key={option} value={option}>
                          {copy.units[option]}
                        </option>
                      ))}
                    </NativeSelect>
                  </span>
                )}
              </span>
            )}
          </Field>
        );
      })}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? copy.saving : copy.save}
        </Button>
        <Button type="button" variant="ghost" onClick={onClose}>
          {copy.cancel}
        </Button>
      </div>
    </form>
  );
}
