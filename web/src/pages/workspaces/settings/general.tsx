import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { LogOut, Trash2 } from "lucide-react";
import { useState, type FormEvent } from "react";
import { hasCode } from "@/api/errors";
import {
  useDeleteWorkspace,
  useLeaveWorkspace,
  useUpdateWorkspace,
  workspaceQuery,
  type AgentCreation,
  type Workspace,
} from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { TypedConfirmDialog } from "@/components/feedback/typed-confirm-dialog";
import { CheckboxField, Field, focusFirstInvalid } from "@/components/forms/field";
import { EntityMark } from "@/components/nodes/entity-mark";
import { KeyValueList, KeyValueRow } from "@/components/nodes/key-value";
import { PageSection } from "@/components/settings/page-section";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { HueControl } from "@/components/workspaces/hue-control";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatBytes, formatDate } from "@/lib/format";
import { strings } from "@/lib/strings";
import { iconProblem, nameProblem, slugProblem } from "@/lib/validation";
import { manages } from "@/lib/workspace-rules";

const copy = strings.workspaces.general;

/**
 * The workspace itself: its name, icon, address and colour for its owners and
 * admins to change, what holds it in, leaving it, and, for owners, deleting it.
 */
export function GeneralSettingsPage() {
  const workspace = useWorkspace();
  const details = useQuery(workspaceQuery(workspace.id));

  if (details.isPending) {
    return <Loading />;
  }
  if (details.isError) {
    return (
      <ErrorNotice
        error={details.error}
        retrying={details.isFetching}
        onRetry={() => void details.refetch()}
      />
    );
  }
  const canEdit = manages(details.data.role);
  return (
    <div className="grid gap-10">
      {canEdit ? (
        <WorkspaceForm workspace={details.data} />
      ) : (
        <WorkspaceFacts workspace={details.data} />
      )}
      {canEdit && <HueSection workspace={details.data} />}
      {canEdit && <AgentsSection workspace={details.data} />}
      <LimitsSection workspace={details.data} />
      <LeaveSection workspace={details.data} />
      {details.data.role === "owner" && <DangerSection workspace={details.data} />}
    </div>
  );
}

/** For members and guests: what the workspace is, without the means to change it. */
function WorkspaceFacts({ workspace }: { workspace: Workspace }) {
  return (
    <PageSection title={copy.title} hint={copy.readOnlyHint}>
      <KeyValueList className="max-w-xl text-sm">
        <KeyValueRow label={copy.name}>
          <span className="flex items-center gap-2">
            <EntityMark name={workspace.name} icon={workspace.icon} hue={workspace.hue} size="sm" />
            {workspace.name}
          </span>
        </KeyValueRow>
        <KeyValueRow label={copy.slug}>
          <span className="font-mono">/w/{workspace.slug}</span>
        </KeyValueRow>
        <KeyValueRow label={copy.role}>{strings.roles[workspace.role]}</KeyValueRow>
        <PeopleRow count={workspace.member_count} />
        <KeyValueRow label={copy.created}>{formatDate(workspace.created_at)}</KeyValueRow>
      </KeyValueList>
    </PageSection>
  );
}

/** How many are in the workspace, which the server tells everyone but guests. */
function PeopleRow({ count }: { count: number | null | undefined }) {
  if (count === undefined || count === null) {
    return null;
  }
  return <KeyValueRow label={copy.members}>{count}</KeyValueRow>;
}

type Problems = {
  name?: string | undefined;
  icon?: string | undefined;
  slug?: string | undefined;
};

function WorkspaceForm({ workspace }: { workspace: Workspace }) {
  const navigate = useNavigate();
  const update = useUpdateWorkspace(workspace.id, (saved) => {
    if (saved.slug !== workspace.slug) {
      void navigate({ to: "/w/$slug/settings", params: { slug: saved.slug }, replace: true });
    }
  });
  const [name, setName] = useState(workspace.name);
  const [icon, setIcon] = useState(workspace.icon ?? "");
  const [slug, setSlug] = useState(workspace.slug);
  const [membersCreate, setMembersCreate] = useState(workspace.members_can_create_teamspaces);
  const [problems, setProblems] = useState<Problems>({});

  const error = update.error;
  const refusedSlug = hasCode(error, "slug_taken") ? copy.slugTaken : undefined;
  const shown = { ...problems, ...(refusedSlug === undefined ? {} : { slug: refusedSlug }) };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    update.reset();
    const found: Problems = {
      name: nameProblem(name),
      icon: iconProblem(icon),
      slug: slug.trim() === "" ? strings.validation.required : slugProblem(slug),
    };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const typedIcon = icon.trim();
    const typedSlug = slug.trim().toLowerCase();
    update.mutate(
      {
        name: name.trim(),
        icon: typedIcon === "" ? null : typedIcon,
        members_can_create_teamspaces: membersCreate,
        ...(typedSlug === workspace.slug ? {} : { slug: typedSlug }),
      },
      {
        onSuccess: (saved) => {
          setName(saved.name);
          setIcon(saved.icon ?? "");
          setSlug(saved.slug);
        },
      },
    );
  };

  return (
    <PageSection title={copy.title} hint={copy.hint}>
      <form noValidate onSubmit={submit} aria-label={copy.title} className="grid max-w-md gap-4">
        {update.isError && refusedSlug === undefined && <ErrorNotice error={error} />}
        <Field label={copy.name} error={shown.name}>
          {(control) => (
            <Input
              {...control}
              name="name"
              autoComplete="off"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          )}
        </Field>
        <Field label={copy.icon} hint={copy.iconHint} error={shown.icon}>
          {(control) => (
            <Input
              {...control}
              name="icon"
              autoComplete="off"
              className="max-w-32"
              value={icon}
              onChange={(event) => setIcon(event.target.value)}
            />
          )}
        </Field>
        <Field label={copy.slug} hint={copy.slugHint(window.location.origin)} error={shown.slug}>
          {(control) => (
            <Input
              {...control}
              name="slug"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              className="font-mono"
              value={slug}
              onChange={(event) => setSlug(event.target.value)}
            />
          )}
        </Field>
        <CheckboxField
          label={copy.membersCreate}
          hint={copy.membersCreateHint}
          checked={membersCreate}
          onCheckedChange={setMembersCreate}
        />
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={update.isPending}>
            {update.isPending ? copy.saving : copy.save}
          </Button>
          <output className="block text-sm text-muted-foreground">
            {update.isSuccess ? copy.saved : ""}
          </output>
        </div>
      </form>
      <KeyValueList className="max-w-xl text-sm">
        <KeyValueRow label={copy.role}>{strings.roles[workspace.role]}</KeyValueRow>
        <PeopleRow count={workspace.member_count} />
        <KeyValueRow label={copy.created}>{formatDate(workspace.created_at)}</KeyValueRow>
      </KeyValueList>
    </PageSection>
  );
}

/** Its hue: the one its id hashes to, or one pinned, saved on its own. */
function HueSection({ workspace }: { workspace: Workspace }) {
  const update = useUpdateWorkspace(workspace.id);
  const [pinned, setPinned] = useState<number | null>(workspace.hue_override ?? null);
  const hue = strings.hue;
  return (
    <PageSection title={hue.title} hint={hue.hint}>
      <HueControl
        id={workspace.id}
        name={workspace.name}
        icon={workspace.icon}
        value={pinned}
        onChange={(next) => {
          update.reset();
          setPinned(next);
        }}
      />
      {update.isError && <ErrorNotice error={update.error} />}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          disabled={update.isPending}
          onClick={() => update.mutate({ hue_override: pinned })}
        >
          {update.isPending ? hue.saving : hue.save}
        </Button>
        <output className="block text-sm text-muted-foreground">
          {update.isSuccess ? hue.saved : ""}
        </output>
      </div>
    </PageSection>
  );
}

/**
 * Whether the workspace's agents work, and who makes them. Off, none is made
 * and every agent's tokens are refused; the instance may have them off anyway.
 */
function AgentsSection({ workspace }: { workspace: Workspace }) {
  const update = useUpdateWorkspace(workspace.id);
  const [enabled, setEnabled] = useState(workspace.agents.enabled);
  const [creation, setCreation] = useState<AgentCreation>(workspace.agents.creation);
  const agents = strings.workspaces.agents;
  return (
    <PageSection title={agents.settings} hint={agents.settingsHint}>
      {!workspace.agents.available && (
        <p className="text-sm text-muted-foreground">{agents.unavailable}</p>
      )}
      <form
        noValidate
        aria-label={agents.settings}
        className="grid max-w-md gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          update.mutate({ agents_enabled: enabled, agent_creation: creation });
        }}
      >
        {update.isError && <ErrorNotice error={update.error} />}
        <CheckboxField
          label={agents.enabled}
          hint={agents.enabledHint}
          checked={enabled}
          onCheckedChange={(on) => {
            update.reset();
            setEnabled(on);
          }}
        />
        <Field label={agents.creation}>
          {(control) => (
            <NativeSelect
              {...control}
              name="agent_creation"
              value={creation}
              onChange={(event) => {
                update.reset();
                setCreation(event.target.value === "admins" ? "admins" : "members");
              }}
            >
              {(["members", "admins"] as const).map((option) => (
                <option key={option} value={option}>
                  {agents.creations[option]}
                </option>
              ))}
            </NativeSelect>
          )}
        </Field>
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" variant="outline" disabled={update.isPending}>
            {update.isPending ? copy.saving : copy.save}
          </Button>
          <output className="block text-sm text-muted-foreground">
            {update.isSuccess ? copy.saved : ""}
          </output>
        </div>
      </form>
    </PageSection>
  );
}

/**
 * What the instance admins set, read only here. A guest learns the limit but
 * not how many are in the workspace, which the server keeps from them.
 */
function LimitsSection({ workspace }: { workspace: Workspace }) {
  const max = workspace.limits.members;
  const limit = max === undefined || max === null ? undefined : String(max);
  const count = workspace.member_count;
  const maxBytes = workspace.limits.storage_bytes;
  const bytesLimit =
    maxBytes === undefined || maxBytes === null ? undefined : formatBytes(maxBytes);
  const used = workspace.storage_bytes;
  return (
    <PageSection title={copy.limits} hint={copy.limitsHint}>
      <KeyValueList className="max-w-xl text-sm">
        <KeyValueRow label={strings.admin.workspaces.kinds.members}>
          {count === undefined || count === null
            ? (limit ?? copy.noLimit)
            : copy.usage(String(count), limit)}
        </KeyValueRow>
        <KeyValueRow label={copy.storage}>
          {used === undefined || used === null
            ? (bytesLimit ?? copy.noLimit)
            : copy.usage(formatBytes(used), bytesLimit)}
        </KeyValueRow>
      </KeyValueList>
    </PageSection>
  );
}

function LeaveSection({ workspace }: { workspace: Workspace }) {
  const navigate = useNavigate();
  const leave = useLeaveWorkspace(workspace.id, () => void navigate({ to: "/", replace: true }));
  const [confirming, setConfirming] = useState(false);
  return (
    <PageSection title={copy.leave} hint={copy.leaveHint}>
      {leave.isError &&
        (hasCode(leave.error, "last_owner", "last_teamspace_owner") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">
              {hasCode(leave.error, "last_owner") ? copy.lastOwner : copy.lastTeamspaceOwner}
            </p>
          </Alert>
        ) : (
          <ErrorNotice error={leave.error} />
        ))}
      <div>
        <Button variant="outline" disabled={leave.isPending} onClick={() => setConfirming(true)}>
          <LogOut aria-hidden />
          {copy.leave}
        </Button>
      </div>
      <ConfirmDialog
        open={confirming}
        title={copy.leaveTitle(workspace.name)}
        description={copy.leaveLede}
        confirm={copy.leave}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          leave.mutate();
        }}
      />
    </PageSection>
  );
}

function DangerSection({ workspace }: { workspace: Workspace }) {
  const navigate = useNavigate();
  const remove = useDeleteWorkspace(workspace.id, () => void navigate({ to: "/", replace: true }));
  const [confirming, setConfirming] = useState(false);
  return (
    <PageSection title={copy.danger} hint={copy.dangerHint}>
      {remove.isError &&
        (hasCode(remove.error, "confirmation_mismatch") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.mismatch}</p>
          </Alert>
        ) : (
          <ErrorNotice error={remove.error} />
        ))}
      <div>
        <Button
          variant="outline"
          className="border-destructive"
          disabled={remove.isPending}
          onClick={() => setConfirming(true)}
        >
          <Trash2 aria-hidden />
          {copy.delete}
        </Button>
      </div>
      <TypedConfirmDialog
        open={confirming}
        title={copy.deleteTitle(workspace.name)}
        description={copy.deleteLede}
        name={workspace.name}
        confirm={copy.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={(typed) => {
          setConfirming(false);
          remove.mutate(typed);
        }}
      />
    </PageSection>
  );
}
