import { useQuery } from "@tanstack/react-query";
import { teamspaceMembersQuery, useJoinTeamspace, type Teamspace } from "@/api/teamspaces";
import { ErrorNotice } from "@/components/feedback/error-notice";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "@/components/workspaces/workspace-context";
import { strings } from "@/lib/strings";
import { teamspaceEntry } from "@/lib/workspace-rules";

const copy = strings.teamspaces.browse;

const either = new Intl.ListFormat("en", { type: "disjunction" });

/**
 * The way into a teamspace the user sees and is not in: a Join button where
 * they may join, else whom to ask, by the names of its owners.
 */
export function TeamspaceEntry({
  teamspace,
  onJoined,
}: {
  teamspace: Teamspace;
  onJoined?: ((name: string) => void) | undefined;
}) {
  const workspace = useWorkspace();
  const join = useJoinTeamspace(workspace.id);
  const entry = teamspaceEntry(workspace.role, teamspace);

  if (entry === "in") {
    return null;
  }
  if (entry === "ask") {
    return <AskOwners teamspace={teamspace} />;
  }
  return (
    <div className="grid gap-2">
      {join.isError && <ErrorNotice error={join.error} />}
      <Button
        size="sm"
        className="justify-self-start"
        disabled={join.isPending}
        aria-label={copy.joinLabel(teamspace.name)}
        onClick={() => join.mutate(teamspace.id, { onSuccess: () => onJoined?.(teamspace.name) })}
      >
        {join.isPending ? copy.joining : copy.join}
      </Button>
    </div>
  );
}

/** A closed teamspace: who to ask to be added, its owners by name once they are known. */
function AskOwners({ teamspace }: { teamspace: Teamspace }) {
  const workspace = useWorkspace();
  const members = useQuery(teamspaceMembersQuery(workspace.id, teamspace.id));
  const owners =
    members.data?.filter((member) => member.role === "owner").map((member) => member.user) ?? [];
  return (
    <p className="text-sm text-muted-foreground">
      {owners.length === 0
        ? copy.askAnyone
        : copy.ask(either.format(owners.map((owner) => owner.display_name)))}
    </p>
  );
}
