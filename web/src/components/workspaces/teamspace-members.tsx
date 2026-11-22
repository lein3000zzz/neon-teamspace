import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { hasCode } from "@/api/errors";
import {
  teamspaceMembersQuery,
  useAddTeamspaceMember,
  useChangeTeamspaceRole,
  useRemoveTeamspaceMember,
  type TeamspaceAccess,
  type TeamspaceMember,
  type TeamspaceRole,
} from "@/api/teamspaces";
import { membersQuery } from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { HueDot } from "@/components/nodes/hue-dot";
import { ItemActions, ItemCard } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDate } from "@/lib/format";
import { strings } from "@/lib/strings";
import { TEAMSPACE_ROLES } from "@/lib/workspace-rules";

const copy = strings.teamspaces.members;

type TeamspaceMembersProps = {
  teamspaceId: string;
  name: string;
  access: TeamspaceAccess;
  /** Whether the user may add and remove members and change their roles. */
  canManage: boolean;
};

/**
 * Who is in a teamspace, owners first, and, for those who manage it, adding
 * people from the workspace, changing their teamspace role and removing them.
 * The default teamspace takes everyone but guests, so nobody is added to it
 * or removed from it here.
 */
export function TeamspaceMembers({ teamspaceId, name, access, canManage }: TeamspaceMembersProps) {
  const workspace = useWorkspace();
  const members = useQuery(teamspaceMembersQuery(workspace.id, teamspaceId));
  const [done, setDone] = useState<string>();
  const fixed = access === "default";

  return (
    <div className="grid gap-4">
      {fixed && <p className="text-sm text-muted-foreground">{copy.defaultNote}</p>}
      <output className="block text-sm text-muted-foreground empty:hidden">{done ?? ""}</output>
      {canManage && !fixed && members.data && (
        <AddMemberForm
          teamspaceId={teamspaceId}
          members={members.data}
          onAdded={(added) => setDone(copy.added(added))}
        />
      )}
      {members.isPending && <Loading />}
      {members.isError && (
        <ErrorNotice
          error={members.error}
          retrying={members.isFetching}
          onRetry={() => void members.refetch()}
        />
      )}
      {members.data?.length === 0 && <p className="text-sm text-muted-foreground">{copy.none}</p>}
      {members.data !== undefined && members.data.length > 0 && (
        <ul className="grid gap-2">
          {members.data.map((member) => (
            <MemberRow
              key={member.user.id}
              teamspaceId={teamspaceId}
              teamspaceName={name}
              member={member}
              canManage={canManage}
              fixed={fixed}
              onDone={setDone}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

type MemberRowProps = {
  teamspaceId: string;
  teamspaceName: string;
  member: TeamspaceMember;
  canManage: boolean;
  fixed: boolean;
  onDone: (message: string) => void;
};

function MemberRow({
  teamspaceId,
  teamspaceName,
  member,
  canManage,
  fixed,
  onDone,
}: MemberRowProps) {
  const me = useMe();
  const workspace = useWorkspace();
  const change = useChangeTeamspaceRole(workspace.id, teamspaceId);
  const remove = useRemoveTeamspaceMember(workspace.id, teamspaceId);
  const [removing, setRemoving] = useState(false);
  const { user } = member;
  const self = user.id === me.id;
  const failure = change.error ?? remove.error;
  const busy = change.isPending || remove.isPending;

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-1.5">
        <p className="flex min-w-0 items-center gap-2 font-medium">
          <HueDot hue={user.hue} />
          <span className="break-words">{user.display_name}</span>
        </p>
        <p className="text-sm break-all text-muted-foreground">{user.email}</p>
        <ChipList>
          <Chip tone={member.role === "owner" ? "strong" : "default"}>
            {strings.teamspaceRoles[member.role]}
          </Chip>
          {self && <Chip>{strings.workspaces.members.you}</Chip>}
          <Chip>{strings.workspaces.members.joined(formatDate(member.joined_at))}</Chip>
        </ChipList>
        {failure !== null &&
          (hasCode(failure, "last_teamspace_owner") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.lastOwner}</p>
            </Alert>
          ) : (
            <ErrorNotice error={failure} />
          ))}
      </div>
      {canManage && (
        <ItemActions>
          <span className="w-36">
            <NativeSelect
              aria-label={copy.roleOf(user.display_name)}
              value={member.role}
              disabled={busy}
              onChange={(event) => {
                const role = TEAMSPACE_ROLES.find((option) => option === event.target.value);
                if (role !== undefined && role !== member.role) {
                  remove.reset();
                  change.mutate({ userId: user.id, role });
                }
              }}
            >
              {TEAMSPACE_ROLES.map((role) => (
                <option key={role} value={role}>
                  {strings.teamspaceRoles[role]}
                </option>
              ))}
            </NativeSelect>
          </span>
          {!fixed && !self && (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={copy.removeLabel(user.display_name)}
              onClick={() => setRemoving(true)}
            >
              {copy.remove}
            </Button>
          )}
        </ItemActions>
      )}
      <ConfirmDialog
        open={removing}
        title={copy.removeTitle(user.display_name, teamspaceName)}
        description={copy.removeLede}
        confirm={copy.remove}
        onCancel={() => setRemoving(false)}
        onConfirm={() => {
          setRemoving(false);
          change.reset();
          remove.mutate(user.id, {
            onSuccess: () => onDone(strings.workspaces.members.removed(user.display_name)),
          });
        }}
      />
    </ItemCard>
  );
}

type AddMemberFormProps = {
  teamspaceId: string;
  members: readonly TeamspaceMember[];
  onAdded: (name: string) => void;
};

/** Someone from the workspace who is not a guest and not in the teamspace yet. */
function AddMemberForm({ teamspaceId, members, onAdded }: AddMemberFormProps) {
  const workspace = useWorkspace();
  const people = useQuery(membersQuery(workspace.id));
  const add = useAddTeamspaceMember(workspace.id, teamspaceId);
  const [userId, setUserId] = useState("");
  const [role, setRole] = useState<TeamspaceRole>("member");
  const inside = new Set(members.map((member) => member.user.id));
  const candidates =
    people.data?.filter((person) => person.role !== "guest" && !inside.has(person.user.id)) ?? [];

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const person = candidates.find((candidate) => candidate.user.id === userId);
    if (person === undefined) {
      return;
    }
    add.mutate(
      { user_id: userId, role },
      {
        onSuccess: () => {
          setUserId("");
          setRole("member");
          onAdded(person.user.display_name);
        },
      },
    );
  };

  if (people.isPending) {
    return <Loading />;
  }
  if (people.isError) {
    return (
      <ErrorNotice
        error={people.error}
        retrying={people.isFetching}
        onRetry={() => void people.refetch()}
      />
    );
  }
  if (candidates.length === 0) {
    return <p className="text-sm text-muted-foreground">{copy.everyoneIn}</p>;
  }
  return (
    <form
      noValidate
      onSubmit={submit}
      aria-label={copy.add}
      className="grid gap-4 rounded-lg border border-border bg-card p-4 sm:grid-cols-[minmax(0,1fr)_10rem_auto] sm:items-end"
    >
      {add.isError && (
        <div className="sm:col-span-3">
          {hasCode(add.error, "forbidden") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.guestRefused}</p>
            </Alert>
          ) : (
            <ErrorNotice error={add.error} />
          )}
        </div>
      )}
      <Field label={copy.person}>
        {(control) => (
          <NativeSelect
            {...control}
            name="user_id"
            value={userId}
            onChange={(event) => setUserId(event.target.value)}
          >
            <option value="">{copy.choose}</option>
            {candidates.map((person) => (
              <option key={person.user.id} value={person.user.id}>
                {person.user.display_name} ({person.user.email})
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <Field label={copy.role}>
        {(control) => (
          <NativeSelect
            {...control}
            name="role"
            value={role}
            onChange={(event) => setRole(event.target.value === "owner" ? "owner" : "member")}
          >
            {TEAMSPACE_ROLES.map((option) => (
              <option key={option} value={option}>
                {strings.teamspaceRoles[option]}
              </option>
            ))}
          </NativeSelect>
        )}
      </Field>
      <Button type="submit" variant="outline" disabled={add.isPending || userId === ""}>
        {add.isPending ? copy.pending : copy.submit}
      </Button>
    </form>
  );
}
