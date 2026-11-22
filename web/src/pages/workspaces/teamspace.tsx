import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { Lock, LogOut, Settings } from "lucide-react";
import { useState } from "react";
import { hasCode } from "@/api/errors";
import { teamspaceQuery, useLeaveTeamspace, type Teamspace } from "@/api/teamspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { EntityMark } from "@/components/nodes/entity-mark";
import { PageSection } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { TeamspaceEntry } from "@/components/workspaces/teamspace-entry";
import { TeamspaceMembers } from "@/components/workspaces/teamspace-members";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { managesTeamspace, mayLeaveTeamspace } from "@/lib/workspace-rules";

const copy = strings.teamspaces.page;

/**
 * One teamspace: what it is, who is in it, and the way in or out. Its pages
 * will live here. A private one the user is not in answers not found, as one
 * that does not exist would.
 */
export function TeamspacePage() {
  const { teamspaceId } = useParams({ from: "/_app/w/$slug/teamspaces/$teamspaceId" });
  const workspace = useWorkspace();
  const teamspace = useQuery(teamspaceQuery(workspace.id, teamspaceId));

  if (teamspace.isPending) {
    return <Loading />;
  }
  if (teamspace.isError) {
    if (hasCode(teamspace.error, "not_found")) {
      return <TeamspaceNotFound />;
    }
    return (
      <ErrorNotice
        error={teamspace.error}
        retrying={teamspace.isFetching}
        onRetry={() => void teamspace.refetch()}
      />
    );
  }
  return <TeamspaceView teamspace={teamspace.data} />;
}

function TeamspaceView({ teamspace }: { teamspace: Teamspace }) {
  const workspace = useWorkspace();
  const canManage = managesTeamspace(workspace.role, teamspace.role);
  return (
    <div className="grid gap-10">
      <PageHeader
        eyebrow={
          <span className="flex items-center gap-2">
            {copy.eyebrow} · {workspace.name}
          </span>
        }
        title={
          <span className="flex min-w-0 items-center gap-3">
            <EntityMark name={teamspace.name} icon={teamspace.icon} hue={teamspace.hue} size="lg" />
            <span className="break-words">{teamspace.name}</span>
            {teamspace.access === "private" && (
              <Lock aria-hidden className="size-5 shrink-0 text-muted-foreground" />
            )}
          </span>
        }
        lede={copy.lede}
      >
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
        <p className="max-w-[62ch] text-sm text-muted-foreground">
          {strings.accessHints[teamspace.access]}
        </p>
        <div className="flex flex-wrap items-start gap-2">
          <TeamspaceEntry teamspace={teamspace} />
          {mayLeaveTeamspace(teamspace) && <LeaveButton teamspace={teamspace} />}
          {canManage && (
            <Button asChild variant="outline" size="sm">
              <Link to="/w/$slug/settings/teamspaces" params={{ slug: workspace.slug }}>
                <Settings aria-hidden />
                {copy.settings}
              </Link>
            </Button>
          )}
        </div>
      </PageHeader>

      <PageSection title={copy.members} hint={copy.membersHint(teamspace.member_count)}>
        <TeamspaceMembers
          teamspaceId={teamspace.id}
          name={teamspace.name}
          access={teamspace.access}
          canManage={canManage}
        />
      </PageSection>
    </div>
  );
}

function LeaveButton({ teamspace }: { teamspace: Teamspace }) {
  const workspace = useWorkspace();
  const navigate = useNavigate();
  const leave = useLeaveTeamspace(workspace.id);
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="grid gap-2">
      {leave.isError &&
        (hasCode(leave.error, "last_teamspace_owner") ? (
          <Alert tone="error" live="assertive">
            <p className="text-foreground">{strings.teamspaces.members.lastOwner}</p>
          </Alert>
        ) : (
          <ErrorNotice error={leave.error} />
        ))}
      <Button
        variant="outline"
        size="sm"
        className="justify-self-start"
        disabled={leave.isPending}
        onClick={() => setConfirming(true)}
      >
        <LogOut aria-hidden />
        {copy.leave}
      </Button>
      <ConfirmDialog
        open={confirming}
        title={copy.leaveTitle(teamspace.name)}
        description={copy.leaveLede}
        confirm={copy.leave}
        onCancel={() => setConfirming(false)}
        onConfirm={() => {
          setConfirming(false);
          leave.mutate(teamspace.id, {
            // A private teamspace closes to whoever leaves it.
            onSuccess: () =>
              teamspace.access === "private"
                ? void navigate({ to: "/w/$slug/teamspaces", params: { slug: workspace.slug } })
                : undefined,
          });
        }}
      />
    </div>
  );
}

function TeamspaceNotFound() {
  const workspace = useWorkspace();
  return (
    <PageHeader eyebrow={strings.notFound.eyebrow} title={copy.notFound} lede={copy.notFoundLede}>
      <div className="mt-3">
        <Button asChild variant="outline">
          <Link to="/w/$slug/teamspaces" params={{ slug: workspace.slug }}>
            {copy.back}
          </Link>
        </Button>
      </div>
    </PageHeader>
  );
}
