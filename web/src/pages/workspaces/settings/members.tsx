import { useQuery } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { hasCode } from "@/api/errors";
import {
  membersQuery,
  sentInvitationsQuery,
  useChangeMemberRole,
  useInvite,
  useRemoveMember,
  useRevokeInvitation,
  useTransferOwnership,
  type Member,
  type SentInvitation,
  type WorkspaceRole,
} from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Field } from "@/components/forms/field";
import { Chip, ChipList } from "@/components/nodes/chip";
import { HueDot } from "@/components/nodes/hue-dot";
import { ItemActions, ItemCard, PageSection } from "@/components/settings/page-section";
import { useMe } from "@/components/shell/signed-in";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { formatDate, formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";
import { emailProblem } from "@/lib/validation";
import {
  assignableRoles,
  INVITATION_ROLES,
  manages,
  mayRemove,
  mayTransfer,
} from "@/lib/workspace-rules";
import { InviteLinks } from "./invite-links";

const copy = strings.workspaces.members;

/**
 * Who is in the workspace, and, for its owners and admins, how others get in:
 * invitations by address, invite links, and the invitations still waiting.
 */
export function MembersSettingsPage() {
  const workspace = useWorkspace();
  const managing = manages(workspace.role);
  return (
    <div className="grid gap-10">
      {managing && <InviteSection />}
      {managing && <InviteLinks />}
      {managing && <PendingSection />}
      <MemberSection />
    </div>
  );
}

/** Which refusals of an invitation are about the address typed, in words of this page. */
const ADDRESS_REFUSALS: Record<string, string> = {
  no_account: copy.noAccount,
  already_member: copy.alreadyMember,
  already_invited: copy.alreadyInvited,
};

function InviteSection() {
  const workspace = useWorkspace();
  const invite = useInvite(workspace.id);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<(typeof INVITATION_ROLES)[number]>("member");
  const [problem, setProblem] = useState<string>();
  const [invited, setInvited] = useState<string>();

  const error = invite.error;
  const refusal = hasCode(error, ...Object.keys(ADDRESS_REFUSALS))
    ? ADDRESS_REFUSALS[error.code]
    : undefined;
  const full = hasCode(error, "member_limit");

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    invite.reset();
    setInvited(undefined);
    const found = emailProblem(email);
    setProblem(found);
    if (found !== undefined) {
      event.currentTarget.querySelector("input")?.focus();
      return;
    }
    const address = email.trim();
    invite.mutate(
      { email: address, role },
      {
        onSuccess: () => {
          setEmail("");
          setInvited(address);
        },
      },
    );
  };

  return (
    <PageSection title={copy.invite} hint={copy.inviteHint}>
      <form
        noValidate
        onSubmit={submit}
        aria-label={copy.invite}
        className="grid max-w-xl gap-4 rounded-lg border border-border bg-card p-4"
      >
        {full && (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{copy.memberLimit}</p>
          </Alert>
        )}
        {invite.isError && refusal === undefined && !full && <ErrorNotice error={error} />}
        <Field label={copy.email} error={problem ?? refusal}>
          {(control) => (
            <Input
              {...control}
              name="email"
              type="email"
              autoComplete="off"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
            />
          )}
        </Field>
        <Field label={copy.role} hint={copy.roleHints[role]}>
          {(control) => (
            <NativeSelect
              {...control}
              name="role"
              value={role}
              onChange={(event) =>
                setRole(
                  INVITATION_ROLES.find((option) => option === event.target.value) ?? "member",
                )
              }
            >
              {INVITATION_ROLES.map((option) => (
                <option key={option} value={option}>
                  {strings.roles[option]}
                </option>
              ))}
            </NativeSelect>
          )}
        </Field>
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={invite.isPending}>
            {invite.isPending ? copy.pending : copy.submit}
          </Button>
          <output className="block text-sm text-muted-foreground">
            {invited === undefined ? "" : copy.invited(invited)}
          </output>
        </div>
      </form>
    </PageSection>
  );
}

function PendingSection() {
  const workspace = useWorkspace();
  const invitations = useQuery(sentInvitationsQuery(workspace.id));
  return (
    <PageSection title={copy.pendingTitle} hint={copy.pendingHint}>
      {invitations.isPending && <Loading />}
      {invitations.isError && (
        <ErrorNotice
          error={invitations.error}
          retrying={invitations.isFetching}
          onRetry={() => void invitations.refetch()}
        />
      )}
      {invitations.data?.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.noPending}</p>
      )}
      {invitations.data !== undefined && invitations.data.length > 0 && (
        <ul className="grid gap-3">
          {invitations.data.map((invitation) => (
            <PendingCard key={invitation.id} invitation={invitation} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

function PendingCard({ invitation }: { invitation: SentInvitation }) {
  const workspace = useWorkspace();
  const revoke = useRevokeInvitation(workspace.id);
  const [confirming, setConfirming] = useState(false);
  const { invitee } = invitation;
  return (
    <ItemCard>
      <div className="grid min-w-0 gap-1.5">
        <p className="flex min-w-0 items-center gap-2 font-medium">
          <HueDot hue={invitee.hue} />
          <span className="break-words">{invitee.display_name}</span>
        </p>
        <p className="text-sm break-all text-muted-foreground">{invitee.email}</p>
        <ChipList>
          <Chip>{copy.as(strings.roleNames[invitation.role])}</Chip>
        </ChipList>
        <p className="text-sm text-muted-foreground">
          {copy.sentBy(formatDateTime(invitation.created_at))} ·{" "}
          {copy.ends(formatDateTime(invitation.expires_at))}
        </p>
        {revoke.isError && <ErrorNotice error={revoke.error} />}
      </div>
      <ItemActions>
        <Button
          variant="outline"
          size="sm"
          disabled={revoke.isPending}
          aria-label={copy.revokeLabel(invitee.email)}
          onClick={() => setConfirming(true)}
        >
          {copy.revoke}
        </Button>
      </ItemActions>
      <ConfirmDialog
        open={confirming}
        title={copy.revokeTitle(invitee.email)}
        description={copy.revokeLede}
        confirm={copy.revoke}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          revoke.mutate(invitation.id);
        }}
      />
    </ItemCard>
  );
}

function MemberSection() {
  const workspace = useWorkspace();
  const members = useQuery(membersQuery(workspace.id));
  const [done, setDone] = useState<string>();
  return (
    <PageSection
      title={copy.list}
      hint={workspace.role === "guest" ? copy.guestHint : copy.listHint}
    >
      <output className="block text-sm text-muted-foreground empty:hidden">{done ?? ""}</output>
      {members.isPending && <Loading />}
      {members.isError && (
        <ErrorNotice
          error={members.error}
          retrying={members.isFetching}
          onRetry={() => void members.refetch()}
        />
      )}
      {members.data && (
        <ul className="grid gap-3">
          {members.data.map((member) => (
            <MemberCard key={member.user.id} member={member} onDone={setDone} />
          ))}
        </ul>
      )}
    </PageSection>
  );
}

/** A change to a member waiting for its confirmation. */
type Pending = { kind: "role"; role: WorkspaceRole } | { kind: "remove" } | { kind: "transfer" };

function MemberCard({ member, onDone }: { member: Member; onDone: (message: string) => void }) {
  const me = useMe();
  const workspace = useWorkspace();
  const changeRole = useChangeMemberRole(workspace.id);
  const remove = useRemoveMember(workspace.id);
  const transfer = useTransferOwnership(workspace.id);
  const [pending, setPending] = useState<Pending>();
  const { user } = member;
  const self = user.id === me.id;
  const roles = assignableRoles(workspace.role, member.role);
  const canRemove = mayRemove(workspace.role, member.role, self);
  const canTransfer = mayTransfer(workspace.role, member.role, self);
  const busy = changeRole.isPending || remove.isPending || transfer.isPending;
  const failure = changeRole.error ?? remove.error ?? transfer.error;
  const name = user.display_name;

  const reset = () => {
    changeRole.reset();
    remove.reset();
    transfer.reset();
  };
  const confirm = () => {
    if (pending === undefined) {
      return;
    }
    setPending(undefined);
    reset();
    if (pending.kind === "role") {
      const role = pending.role;
      changeRole.mutate(
        { userId: user.id, role },
        { onSuccess: () => onDone(copy.changed(name, strings.roleNames[role])) },
      );
    } else if (pending.kind === "remove") {
      remove.mutate(user.id, { onSuccess: () => onDone(copy.removed(name)) });
    } else {
      transfer.mutate(user.id, { onSuccess: () => onDone(copy.transferred(name)) });
    }
  };

  const dialog =
    pending === undefined
      ? undefined
      : pending.kind === "role"
        ? {
            title: copy.changeTitle(name, strings.roleNames[pending.role]),
            lede: self
              ? `${copy.changeLedes[pending.role]} ${copy.changeSelf}`
              : copy.changeLedes[pending.role],
            action: copy.change,
          }
        : pending.kind === "remove"
          ? { title: copy.removeTitle(name), lede: copy.removeLede, action: copy.remove }
          : { title: copy.transferTitle(name), lede: copy.transferLede, action: copy.transfer };

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-1.5">
        <p className="flex min-w-0 items-center gap-2 font-medium">
          <HueDot hue={user.hue} />
          <span className="break-words">{name}</span>
        </p>
        <p className="text-sm break-all text-muted-foreground">{user.email}</p>
        <ChipList>
          <Chip tone={member.role === "owner" ? "strong" : "default"}>
            {strings.roles[member.role]}
          </Chip>
          {self && <Chip>{copy.you}</Chip>}
          <Chip>{copy.joined(formatDate(member.joined_at))}</Chip>
        </ChipList>
        {failure !== null &&
          (hasCode(failure, "last_owner") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">{copy.lastOwner}</p>
            </Alert>
          ) : (
            <ErrorNotice error={failure} />
          ))}
      </div>
      {(roles.length > 0 || canRemove || canTransfer) && (
        <ItemActions>
          {roles.length > 0 && (
            <span className="w-36">
              <NativeSelect
                aria-label={copy.roleOf(name)}
                value={member.role}
                disabled={busy}
                onChange={(event) => {
                  const role = roles.find((option) => option === event.target.value);
                  if (role !== undefined && role !== member.role) {
                    setPending({ kind: "role", role });
                  }
                }}
              >
                {roles.map((role) => (
                  <option key={role} value={role}>
                    {strings.roles[role]}
                  </option>
                ))}
              </NativeSelect>
            </span>
          )}
          {canTransfer && (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={copy.transferLabel(name)}
              onClick={() => setPending({ kind: "transfer" })}
            >
              {copy.transfer}
            </Button>
          )}
          {canRemove && (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              aria-label={copy.removeLabel(name)}
              onClick={() => setPending({ kind: "remove" })}
            >
              {copy.remove}
            </Button>
          )}
        </ItemActions>
      )}
      <ConfirmDialog
        open={dialog !== undefined}
        title={dialog?.title}
        description={dialog?.lede}
        confirm={dialog?.action ?? ""}
        onCancel={() => setPending(undefined)}
        onConfirm={confirm}
      />
    </ItemCard>
  );
}
