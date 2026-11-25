import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { Download, Lock, LogOut, Plus, Settings, Table2, Upload } from "lucide-react";
import { useState } from "react";
import { hasCode } from "@/api/errors";
import { teamspacePagesQuery, useExportTeamspace } from "@/api/pages";
import { teamspaceQuery, useLeaveTeamspace, type Teamspace } from "@/api/teamspaces";
import { ConfirmDialog } from "@/components/feedback/confirm-dialog";
import { ErrorNotice, Loading } from "@/components/feedback/error-notice";
import { Chip, ChipList } from "@/components/nodes/chip";
import { EntityMark } from "@/components/nodes/entity-mark";
import { ImportDialog } from "@/components/pages/import-dialog";
import { PageCards } from "@/components/pages/page-cards";
import { useNewDatabase } from "@/components/pages/use-new-database";
import { useNewPage } from "@/components/pages/use-new-page";
import { PageSection } from "@/components/settings/page-section";
import { PageHeader } from "@/components/shell/page-header";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { TeamspaceEntry } from "@/components/workspaces/teamspace-entry";
import { TeamspaceMembers } from "@/components/workspaces/teamspace-members";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { forestOf } from "@/lib/page-tree";
import { strings } from "@/lib/strings";
import { managesTeamspace, mayLeaveTeamspace } from "@/lib/workspace-rules";

const copy = strings.teamspaces.page;

/**
 * One teamspace: what it is, its pages, who is in it, and the way in or out.
 * A private one the user is not in answers not found, as one that does not
 * exist would.
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

      <TeamspacePages teamspace={teamspace} />

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

/**
 * The pages at the teamspace's top, and, for its members, the ways to add a
 * page or a database and to import markdown; for everyone who sees them, the
 * way to export them.
 */
function TeamspacePages({ teamspace }: { teamspace: Teamspace }) {
  const workspace = useWorkspace();
  const pages = useQuery(teamspacePagesQuery(workspace.id, teamspace.id));
  const newPage = useNewPage(workspace);
  const newDatabase = useNewDatabase(workspace);
  const exporter = useExportTeamspace();
  const [importing, setImporting] = useState(false);
  const top = pages.data === undefined ? [] : forestOf(pages.data).roots;
  const transfer = strings.pages.transfer;
  return (
    <PageSection
      title={copy.pages}
      hint={copy.pagesHint}
      actions={
        <div className="flex flex-wrap gap-2">
          {teamspace.role && (
            <>
              <Button
                size="sm"
                disabled={newPage.pending}
                onClick={() => newPage.start({ teamspace_id: teamspace.id })}
              >
                <Plus aria-hidden />
                {newPage.pending ? strings.pages.creating : copy.newPage}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={newDatabase.pending}
                onClick={() => newDatabase.start({ teamspace_id: teamspace.id })}
              >
                <Table2 aria-hidden />
                {newDatabase.pending
                  ? strings.databases.create.creating
                  : strings.databases.create.newDatabase}
              </Button>
              <Button variant="outline" size="sm" onClick={() => setImporting(true)}>
                <Upload aria-hidden />
                {transfer.import}
              </Button>
            </>
          )}
          {top.length > 0 && (
            <Button
              variant="outline"
              size="sm"
              disabled={exporter.isPending}
              onClick={() => exporter.mutate(teamspace.id)}
            >
              <Download aria-hidden />
              {exporter.isPending ? transfer.exporting(teamspace.name) : transfer.export}
            </Button>
          )}
        </div>
      }
    >
      {importing && (
        <ImportDialog
          workspace={workspace}
          place={{ teamspace_id: teamspace.id }}
          placeName={teamspace.name}
          open
          onOpenChange={setImporting}
        />
      )}
      {exporter.isError && <ErrorNotice error={exporter.error} />}
      {pages.isPending && <Loading />}
      {pages.isError && (
        <ErrorNotice
          error={pages.error}
          retrying={pages.isFetching}
          onRetry={() => void pages.refetch()}
        />
      )}
      {pages.isSuccess && top.length === 0 && (
        <p className="text-sm text-muted-foreground">{copy.noPages}</p>
      )}
      {top.length > 0 && <PageCards workspace={workspace} pages={top} label={copy.pages} />}
    </PageSection>
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
