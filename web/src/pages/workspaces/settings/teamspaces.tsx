import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ChevronDown, ChevronRight, Lock, Plus } from "lucide-react";
import { useId, useState, type FormEvent, type ReactNode } from "react";
import {
  teamspacesQuery,
  useCreateTeamspace,
  useDeleteTeamspace,
  useJoinTeamspace,
  useUpdateTeamspace,
  type Teamspace,
  type TeamspaceAccess,
  type TeamspaceName,
} from "@/api/teamspaces";
import { workspaceQuery } from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field, focusFirstInvalid } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { EntityMark } from "@/components/nodes/entity-mark";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AccessPicker, choiceOf } from "@/components/workspaces/access-picker";
import { HueControl } from "@/components/workspaces/hue-control";
import { TeamspaceEntry } from "@/components/workspaces/teamspace-entry";
import { TeamspaceMembers } from "@/components/workspaces/teamspace-members";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { iconProblem, nameProblem } from "@/lib/validation";
import { managesTeamspace, mayCreateTeamspace, seesTeamspaces } from "@/lib/workspace-rules";
import { NotAllowedPage } from "@/pages/not-allowed";

const copy = strings.teamspaces.settings;

/**
 * The workspace's teamspaces: creating one, where the role allows, and
 * changing, staffing and deleting those the user manages. Owners and admins
 * also get the private teamspaces they are not in, by name, to manage them.
 */
export function TeamspacesSettingsPage() {
  const workspace = useWorkspace();
  const details = useQuery(workspaceQuery(workspace.id));
  const teamspaces = useQuery(teamspacesQuery(workspace.id));
  const [done, setDone] = useState<string>();

  if (!seesTeamspaces(workspace.role)) {
    return <NotAllowedPage />;
  }
  const canCreate =
    details.data !== undefined &&
    mayCreateTeamspace(workspace.role, details.data.members_can_create_teamspaces);

  return (
    <div className="grid gap-10">
      <PageSection
        title={copy.create}
        hint={details.data === undefined || canCreate ? copy.createHint : copy.createOff}
      >
        {canCreate && <CreateTeamspaceForm onCreated={(name) => setDone(copy.created(name))} />}
      </PageSection>

      <PageSection title={copy.list} hint={copy.listHint}>
        <output className="block text-sm text-muted-foreground empty:hidden">{done ?? ""}</output>
        {teamspaces.isPending && <Loading />}
        {teamspaces.isError && (
          <ErrorNotice
            error={teamspaces.error}
            retrying={teamspaces.isFetching}
            onRetry={() => void teamspaces.refetch()}
          />
        )}
        {teamspaces.data && (
          <ul className="grid gap-3">
            {teamspaces.data.teamspaces.map((teamspace) => (
              <TeamspaceCard key={teamspace.id} teamspace={teamspace} onDone={setDone} />
            ))}
            {teamspaces.data.hidden.map((teamspace) => (
              <HiddenTeamspaceCard key={teamspace.id} teamspace={teamspace} onDone={setDone} />
            ))}
          </ul>
        )}
      </PageSection>
    </div>
  );
}

type Problems = { name?: string | undefined; icon?: string | undefined };

function CreateTeamspaceForm({ onCreated }: { onCreated: (name: string) => void }) {
  const workspace = useWorkspace();
  const accessId = useId();
  const create = useCreateTeamspace(workspace.id);
  const [name, setName] = useState("");
  const [icon, setIcon] = useState("");
  const [access, setAccess] = useState<"open" | "closed" | "private">("open");
  const [problems, setProblems] = useState<Problems>({});

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    create.reset();
    const found: Problems = { name: nameProblem(name), icon: iconProblem(icon) };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const typedIcon = icon.trim();
    create.mutate(
      { name: name.trim(), access, ...(typedIcon === "" ? {} : { icon: typedIcon }) },
      {
        onSuccess: (teamspace) => {
          setName("");
          setIcon("");
          setAccess("open");
          onCreated(teamspace.name);
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.create}
      className="grid gap-4 rounded-lg border border-border bg-card p-4"
    >
      {create.isError && <ErrorNotice error={create.error} />}
      <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_8rem]">
        <Field label={copy.name} error={problems.name}>
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
        <Field label={copy.icon} hint={copy.iconHint} error={problems.icon}>
          {(control) => (
            <Input
              {...control}
              name="icon"
              autoComplete="off"
              value={icon}
              onChange={(event) => setIcon(event.target.value)}
            />
          )}
        </Field>
      </div>
      <div className="grid gap-1.5">
        <p id={accessId} className="text-sm font-medium">
          {copy.access}
        </p>
        <AccessPicker value={access} onChange={setAccess} labelledBy={accessId} />
      </div>
      <div>
        <Button type="submit" disabled={create.isPending}>
          <Plus aria-hidden />
          {create.isPending ? copy.pending : copy.submit}
        </Button>
      </div>
    </form>
  );
}

type CardProps = { onDone: (message: string) => void };

/** A teamspace the user sees: its settings and members for those who manage it. */
function TeamspaceCard({ teamspace, onDone }: CardProps & { teamspace: Teamspace }) {
  const workspace = useWorkspace();
  const [editing, setEditing] = useState(false);
  const canManage = managesTeamspace(workspace.role, teamspace.role);
  return (
    <ManagedCard
      id={teamspace.id}
      name={teamspace.name}
      access={teamspace.access}
      canManage={canManage}
      onDone={onDone}
      editing={editing}
      onEdit={() => setEditing(true)}
      heading={
        <>
          <p className="flex min-w-0 items-center gap-2.5 font-medium">
            <EntityMark name={teamspace.name} icon={teamspace.icon} hue={teamspace.hue} />
            <span className="break-words">{teamspace.name}</span>
            {teamspace.access === "private" && (
              <Lock aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
            )}
          </p>
          <ChipList>
            <Chip tone="strong" hue={teamspace.hue}>
              {strings.access[teamspace.access]}
            </Chip>
            <Chip>
              {teamspace.role
                ? strings.teamspaces.yourRole(strings.teamspaceRoles[teamspace.role])
                : strings.teamspaces.notIn}
            </Chip>
            <Chip>{strings.teamspaces.count(teamspace.member_count)}</Chip>
          </ChipList>
          {!canManage && <TeamspaceEntry teamspace={teamspace} />}
        </>
      }
      editor={
        <EditTeamspaceForm
          teamspace={teamspace}
          onClose={() => setEditing(false)}
          onDone={onDone}
        />
      }
      open={
        <Button asChild variant="outline" size="sm">
          <Link
            to="/w/$slug/teamspaces/$teamspaceId"
            params={{ slug: workspace.slug, teamspaceId: teamspace.id }}
            aria-label={strings.teamspaces.browse.openLabel(teamspace.name)}
          >
            {strings.teamspaces.browse.open}
          </Link>
        </Button>
      }
    />
  );
}

/** A private teamspace an owner or admin is not in: its name, to manage it. */
function HiddenTeamspaceCard({ teamspace, onDone }: CardProps & { teamspace: TeamspaceName }) {
  const workspace = useWorkspace();
  const join = useJoinTeamspace(workspace.id);
  const [editing, setEditing] = useState(false);
  return (
    <ManagedCard
      id={teamspace.id}
      name={teamspace.name}
      access="private"
      canManage
      onDone={onDone}
      editing={editing}
      onEdit={() => setEditing(true)}
      heading={
        <>
          <p className="flex min-w-0 items-center gap-2.5 font-medium">
            <Lock aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            <span className="break-words">{teamspace.name}</span>
          </p>
          <ChipList>
            <Chip>{strings.access.private}</Chip>
            <Chip>{strings.teamspaces.notIn}</Chip>
          </ChipList>
          <p className="text-sm text-muted-foreground">{strings.teamspaces.hiddenNote}</p>
          {join.isError && <ErrorNotice error={join.error} />}
        </>
      }
      editor={
        <EditHiddenForm teamspace={teamspace} onClose={() => setEditing(false)} onDone={onDone} />
      }
      open={
        <Button
          variant="outline"
          size="sm"
          disabled={join.isPending}
          aria-label={copy.joinLabel(teamspace.name)}
          onClick={() => join.mutate(teamspace.id)}
        >
          {copy.join}
        </Button>
      }
    />
  );
}

type ManagedCardProps = CardProps & {
  id: string;
  name: string;
  access: TeamspaceAccess;
  canManage: boolean;
  heading: ReactNode;
  /** The edit form, shown in place while `editing`. */
  editor: ReactNode;
  editing: boolean;
  onEdit: () => void;
  open: ReactNode;
};

/**
 * The parts every teamspace card shares: its edit form and its members, each
 * opened in place, and deleting it, which General never is.
 */
function ManagedCard({
  id,
  name,
  access,
  canManage,
  heading,
  editor,
  editing,
  onEdit,
  open,
  onDone,
}: ManagedCardProps) {
  const workspace = useWorkspace();
  const remove = useDeleteTeamspace(workspace.id);
  const membersId = useId();
  const [showMembers, setShowMembers] = useState(false);
  const [confirming, setConfirming] = useState(false);

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        {heading}
        {remove.isError && <ErrorNotice error={remove.error} />}
        {editing && editor}
        {canManage && (
          <div className="grid gap-2">
            <Button
              variant="ghost"
              size="sm"
              className="justify-self-start"
              aria-expanded={showMembers}
              aria-controls={membersId}
              aria-label={copy.membersLabel(name)}
              onClick={() => setShowMembers((shown) => !shown)}
            >
              {showMembers ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
              {copy.members}
            </Button>
            <div id={membersId} hidden={!showMembers}>
              {showMembers && (
                <TeamspaceMembers teamspaceId={id} name={name} access={access} canManage />
              )}
            </div>
          </div>
        )}
      </div>
      <ItemActions>
        {open}
        {canManage && !editing && (
          <Button variant="outline" size="sm" aria-label={copy.editLabel(name)} onClick={onEdit}>
            {copy.edit}
          </Button>
        )}
        {canManage && access !== "default" && (
          <Button
            variant="outline"
            size="sm"
            disabled={remove.isPending}
            aria-label={copy.deleteLabel(name)}
            onClick={() => setConfirming(true)}
          >
            {copy.delete}
          </Button>
        )}
      </ItemActions>
      <ConfirmDialog
        open={confirming}
        title={copy.deleteTitle(name)}
        description={copy.deleteLede}
        confirm={copy.delete}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          remove.mutate(id, { onSuccess: () => onDone(copy.deleted(name)) });
        }}
      />
    </ItemCard>
  );
}

type EditProps = CardProps & { onClose: () => void };

/** Name, icon, access and hue; only what changed is sent. */
function EditTeamspaceForm({ teamspace, onClose, onDone }: EditProps & { teamspace: Teamspace }) {
  const workspace = useWorkspace();
  const accessId = useId();
  const update = useUpdateTeamspace(workspace.id);
  const [name, setName] = useState(teamspace.name);
  const [icon, setIcon] = useState(teamspace.icon ?? "");
  const [access, setAccess] = useState(choiceOf(teamspace.access));
  const [pinned, setPinned] = useState<number | null>(teamspace.hue_override ?? null);
  const [problems, setProblems] = useState<Problems>({});

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    update.reset();
    const found: Problems = { name: nameProblem(name), icon: iconProblem(icon) };
    setProblems(found);
    const invalid = (Object.keys(found) as (keyof Problems)[]).filter((key) => found[key]);
    if (invalid.length > 0) {
      focusFirstInvalid(event.currentTarget, invalid);
      return;
    }
    const typedName = name.trim();
    const typedIcon = icon.trim();
    const was = { icon: teamspace.icon ?? "", pinned: teamspace.hue_override ?? null };
    update.mutate(
      {
        id: teamspace.id,
        ...(typedName === teamspace.name ? {} : { name: typedName }),
        ...(typedIcon === was.icon ? {} : { icon: typedIcon === "" ? null : typedIcon }),
        ...(access === undefined || access === teamspace.access ? {} : { access }),
        ...(pinned === was.pinned ? {} : { hue_override: pinned }),
      },
      {
        onSuccess: (saved) => {
          onDone(copy.saved(saved.name));
          onClose();
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.editLabel(teamspace.name)}
      className="grid gap-4 rounded-lg border border-border bg-background p-4"
    >
      {update.isError && <ErrorNotice error={update.error} />}
      <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_8rem]">
        <Field label={copy.name} error={problems.name}>
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
        <Field label={copy.icon} hint={copy.iconHint} error={problems.icon}>
          {(control) => (
            <Input
              {...control}
              name="icon"
              autoComplete="off"
              value={icon}
              onChange={(event) => setIcon(event.target.value)}
            />
          )}
        </Field>
      </div>
      <div className="grid gap-1.5">
        <p id={accessId} className="text-sm font-medium">
          {copy.access}
        </p>
        {access === undefined ? (
          <p className="text-sm text-muted-foreground">{copy.defaultAccess}</p>
        ) : (
          <AccessPicker value={access} onChange={setAccess} labelledBy={accessId} />
        )}
      </div>
      <div className="grid gap-1.5">
        <p className="text-sm font-medium">{strings.hue.title}</p>
        <HueControl
          id={teamspace.id}
          name={name.trim() === "" ? teamspace.name : name.trim()}
          icon={icon}
          value={pinned}
          onChange={setPinned}
        />
      </div>
      <FormButtons pending={update.isPending} onClose={onClose} />
    </form>
  );
}

/** For a private teamspace the user is not in, only its name and access are known. */
function EditHiddenForm({ teamspace, onClose, onDone }: EditProps & { teamspace: TeamspaceName }) {
  const workspace = useWorkspace();
  const accessId = useId();
  const update = useUpdateTeamspace(workspace.id);
  const [name, setName] = useState(teamspace.name);
  const [access, setAccess] = useState<"open" | "closed" | "private">("private");
  const [problem, setProblem] = useState<string>();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    update.reset();
    const found = nameProblem(name);
    setProblem(found);
    if (found !== undefined) {
      event.currentTarget.querySelector("input")?.focus();
      return;
    }
    const typedName = name.trim();
    update.mutate(
      {
        id: teamspace.id,
        ...(typedName === teamspace.name ? {} : { name: typedName }),
        ...(access === "private" ? {} : { access }),
      },
      {
        onSuccess: (saved) => {
          onDone(copy.saved(saved.name));
          onClose();
        },
      },
    );
  };

  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.editLabel(teamspace.name)}
      className="grid gap-4 rounded-lg border border-border bg-background p-4"
    >
      {update.isError && <ErrorNotice error={update.error} />}
      <Field label={copy.name} error={problem}>
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
      <div className="grid gap-1.5">
        <p id={accessId} className="text-sm font-medium">
          {copy.access}
        </p>
        <AccessPicker value={access} onChange={setAccess} labelledBy={accessId} />
      </div>
      <FormButtons pending={update.isPending} onClose={onClose} />
    </form>
  );
}

function FormButtons({ pending, onClose }: { pending: boolean; onClose: () => void }) {
  return (
    <div className="flex flex-wrap gap-2">
      <Button type="submit" disabled={pending}>
        {pending ? copy.saving : copy.save}
      </Button>
      <Button type="button" variant="ghost" onClick={onClose}>
        {copy.cancel}
      </Button>
    </div>
  );
}
