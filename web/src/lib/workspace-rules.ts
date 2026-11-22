/**
 * What a role lets someone do in a workspace, as the server's rules say, so
 * pages show only the controls that can work. The server decides in the end:
 * a control shown here that it refuses gets its answer, a 403, shown like any
 * other error.
 */
import type { Teamspace, TeamspaceAccess, TeamspaceRole } from "@/api/teamspaces";
import type { WorkspaceRole } from "@/api/workspaces";

/** Highest first, as the member list orders them. */
export const WORKSPACE_ROLES = [
  "owner",
  "admin",
  "member",
  "guest",
] as const satisfies readonly WorkspaceRole[];

/** What an invitation can make someone; owners are made by a transfer or a role change. */
export const INVITATION_ROLES = ["admin", "member", "guest"] as const satisfies WorkspaceRole[];

/** What an invite link can admit as. */
export const LINK_ROLES = ["member", "guest"] as const satisfies WorkspaceRole[];

/** What a new teamspace can be; the default one comes with the workspace. */
export const NEW_TEAMSPACE_ACCESS = [
  "open",
  "closed",
  "private",
] as const satisfies readonly TeamspaceAccess[];

export const TEAMSPACE_ROLES = ["owner", "member"] as const satisfies readonly TeamspaceRole[];

/** Owners and admins manage members, invitations, invite links, teamspaces and settings. */
export function manages(role: WorkspaceRole): boolean {
  return role === "owner" || role === "admin";
}

export function mayCreateTeamspace(role: WorkspaceRole, membersCanCreate: boolean): boolean {
  return manages(role) || (role === "member" && membersCanCreate);
}

/**
 * The roles `actor` may give a member whose role is `target`, their current
 * one included; none when they may not change it. Only owners touch owners or
 * make them; an admin may change any other role, their own included.
 */
export function assignableRoles(actor: WorkspaceRole, target: WorkspaceRole): WorkspaceRole[] {
  if (actor === "owner") {
    return [...WORKSPACE_ROLES];
  }
  if (actor === "admin" && target !== "owner") {
    return ["admin", "member", "guest"];
  }
  return [];
}

/** Nobody removes themselves: they leave. */
export function mayRemove(actor: WorkspaceRole, target: WorkspaceRole, self: boolean): boolean {
  if (self) {
    return false;
  }
  return actor === "owner" || (actor === "admin" && target !== "owner");
}

/** An owner hands ownership to someone else who is not an owner already. */
export function mayTransfer(actor: WorkspaceRole, target: WorkspaceRole, self: boolean): boolean {
  return actor === "owner" && !self && target !== "owner";
}

/** Guests are in no teamspace, and see none. */
export function seesTeamspaces(role: WorkspaceRole): boolean {
  return role !== "guest";
}

/**
 * Who changes a teamspace, its access and its members: its owners, and the
 * workspace's owners and admins.
 */
export function managesTeamspace(
  role: WorkspaceRole,
  membership: TeamspaceRole | null | undefined,
): boolean {
  return manages(role) || membership === "owner";
}

/**
 * How the user is or gets into a teamspace they see: in it already; free to
 * join; or asking one of its owners to add them, for a closed one.
 */
export type TeamspaceEntry = "in" | "join" | "ask";

export function teamspaceEntry(
  role: WorkspaceRole,
  teamspace: Pick<Teamspace, "access" | "role">,
): TeamspaceEntry {
  if (teamspace.role) {
    return "in";
  }
  if (teamspace.access === "open" || manages(role)) {
    return "join";
  }
  return "ask";
}

/** Anyone in a teamspace leaves it, but for the default one, which everyone stays in. */
export function mayLeaveTeamspace(teamspace: Pick<Teamspace, "access" | "role">): boolean {
  return teamspace.access !== "default" && !!teamspace.role;
}
