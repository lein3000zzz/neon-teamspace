import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { agentsQuery } from "@/api/agents";
import { membersQuery } from "@/api/workspaces";
import { peopleById, type Someone } from "@/lib/people";

/**
 * Everyone a workspace's content may name, by id: its members, agents among
 * them, and the agents deleted since, so what they made still names them.
 * Empty until both lists come, and while `enabled` is false, for a guest, who
 * knows of no other member.
 */
export function useWorkspacePeople(
  workspaceId: string,
  enabled: boolean,
): ReadonlyMap<string, Someone> {
  const members = useQuery({ ...membersQuery(workspaceId), enabled });
  const agents = useQuery({ ...agentsQuery(workspaceId), enabled });
  return useMemo(
    () => peopleById(members.data ?? [], agents.data ?? []),
    [members.data, agents.data],
  );
}
