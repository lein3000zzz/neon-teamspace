import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { hasCode } from "@/api/errors";
import {
  myInvitationsQuery,
  useAcceptInvitation,
  useDeclineInvitation,
  type ReceivedInvitation,
  type WorkspaceSummary,
} from "@/api/workspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { EntityMark } from "@/components/nodes/entity-mark";
import { ItemActions, ItemCard } from "@/components/settings/page-section";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/format";
import { strings } from "@/lib/strings";

const copy = strings.account.invitations;

/**
 * The invitations into workspaces that wait for the user, each with the
 * workspace's mark, the role it offers and who sent it, to accept or decline.
 */
export function InvitationList() {
  const invitations = useQuery(myInvitationsQuery);
  const [joined, setJoined] = useState<WorkspaceSummary>();
  const [declined, setDeclined] = useState<string>();

  return (
    <div className="grid gap-3">
      <output className="block text-sm empty:hidden">
        {joined !== undefined && (
          <span className="flex flex-wrap items-center gap-x-3">
            {copy.joined(joined.name)}
            <Link
              to="/w/$slug"
              params={{ slug: joined.slug }}
              className="inline-flex min-h-(--tap) items-center text-foreground underline underline-offset-4"
            >
              {copy.open(joined.name)}
            </Link>
          </span>
        )}
        {joined === undefined && declined !== undefined && (
          <span className="text-muted-foreground">{copy.declined(declined)}</span>
        )}
      </output>
      {invitations.isPending && <Loading />}
      {invitations.isError && (
        <ErrorNotice
          error={invitations.error}
          retrying={invitations.isFetching}
          onRetry={() => void invitations.refetch()}
        />
      )}
      {invitations.data?.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.none}</p>
      )}
      {invitations.data !== undefined && invitations.data.length > 0 && (
        <ul className="grid gap-3">
          {invitations.data.map((invitation) => (
            <InvitationCard
              key={invitation.id}
              invitation={invitation}
              onJoined={(workspace) => {
                setDeclined(undefined);
                setJoined(workspace);
              }}
              onDeclined={(name) => {
                setJoined(undefined);
                setDeclined(name);
              }}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

type InvitationCardProps = {
  invitation: ReceivedInvitation;
  onJoined: (workspace: WorkspaceSummary) => void;
  onDeclined: (name: string) => void;
};

function InvitationCard({ invitation, onJoined, onDeclined }: InvitationCardProps) {
  const accept = useAcceptInvitation();
  const decline = useDeclineInvitation();
  const [confirming, setConfirming] = useState(false);
  const { workspace } = invitation;
  const inviter = invitation.invited_by ?? undefined;
  const busy = accept.isPending || decline.isPending;

  return (
    <ItemCard>
      <div className="grid min-w-0 gap-2">
        <p className="flex min-w-0 items-center gap-2.5 font-medium">
          <EntityMark name={workspace.name} icon={workspace.icon} hue={workspace.hue} />
          <span className="break-words">{workspace.name}</span>
        </p>
        <ChipList>
          <Chip tone="strong" hue={workspace.hue}>
            {copy.as(strings.roleNames[invitation.role])}
          </Chip>
          <Chip>{inviter === undefined ? copy.fromGone : copy.from(inviter.display_name)}</Chip>
        </ChipList>
        <p className="text-sm text-muted-foreground">
          {copy.sent(formatDateTime(invitation.created_at))} ·{" "}
          {copy.ends(formatDateTime(invitation.expires_at))}
        </p>
        {accept.isError &&
          (hasCode(accept.error, "member_limit", "already_member") ? (
            <Alert tone="error" live="assertive">
              <p className="text-foreground">
                {hasCode(accept.error, "member_limit") ? copy.full : copy.already}
              </p>
            </Alert>
          ) : (
            <ErrorNotice error={accept.error} />
          ))}
        {decline.isError && <ErrorNotice error={decline.error} />}
      </div>
      <ItemActions>
        <Button
          size="sm"
          disabled={busy}
          aria-label={copy.actionLabel(copy.accept, workspace.name)}
          onClick={() => {
            decline.reset();
            accept.mutate(invitation.id, { onSuccess: onJoined });
          }}
        >
          {accept.isPending ? copy.accepting : copy.accept}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          aria-label={copy.actionLabel(copy.decline, workspace.name)}
          onClick={() => setConfirming(true)}
        >
          {copy.decline}
        </Button>
      </ItemActions>
      <ConfirmDialog
        open={confirming}
        title={copy.declineTitle(workspace.name)}
        description={copy.declineLede}
        confirm={copy.decline}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          accept.reset();
          decline.mutate(invitation.id, { onSuccess: () => onDeclined(workspace.name) });
        }}
      />
    </ItemCard>
  );
}
