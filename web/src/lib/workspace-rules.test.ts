import { describe, expect, it } from "vitest";
import type { TeamspaceAccess, TeamspaceRole } from "@/api/teamspaces";
import type { WorkspaceRole } from "@/api/workspaces";
import {
  assignableRoles,
  manages,
  mayCreateTeamspace,
  mayLeaveTeamspace,
  mayRemove,
  mayTransfer,
  managesTeamspace,
  seesTeamspaces,
  teamspaceEntry,
  WORKSPACE_ROLES,
  type TeamspaceEntry,
} from "./workspace-rules";

/*
 * The tables of the permissions reference, every acting role against every
 * other, so the mirror cannot drift from the server's rules unseen.
 */

type ByRole<T> = Record<WorkspaceRole, T>;

const ALL: WorkspaceRole[] = ["owner", "admin", "member", "guest"];
const BUT_OWNER: WorkspaceRole[] = ["admin", "member", "guest"];
const NONE: WorkspaceRole[] = [];

/** Workspaces: change its settings, invite, list and revoke invitations and links. */
const MANAGES: ByRole<boolean> = { owner: true, admin: true, member: false, guest: false };

/** Create a teamspace, with members_can_create_teamspaces on and off. */
const CREATES: ByRole<[boolean, boolean]> = {
  owner: [true, true],
  admin: [true, true],
  member: [true, false],
  guest: [false, false],
};

/** Members: the roles the acting role (first key) gives a member whose role is the second key. */
const CHANGES: ByRole<ByRole<WorkspaceRole[]>> = {
  owner: { owner: ALL, admin: ALL, member: ALL, guest: ALL },
  admin: { owner: NONE, admin: BUT_OWNER, member: BUT_OWNER, guest: BUT_OWNER },
  member: { owner: NONE, admin: NONE, member: NONE, guest: NONE },
  guest: { owner: NONE, admin: NONE, member: NONE, guest: NONE },
};

/** Members: remove another member, by the acting role, then the member's. */
const REMOVES: ByRole<ByRole<boolean>> = {
  owner: { owner: true, admin: true, member: true, guest: true },
  admin: { owner: false, admin: true, member: true, guest: true },
  member: { owner: false, admin: false, member: false, guest: false },
  guest: { owner: false, admin: false, member: false, guest: false },
};

/** Members: transfer ownership to another member. */
const TRANSFERS: ByRole<ByRole<boolean>> = {
  owner: { owner: false, admin: true, member: true, guest: true },
  admin: { owner: false, admin: false, member: false, guest: false },
  member: { owner: false, admin: false, member: false, guest: false },
  guest: { owner: false, admin: false, member: false, guest: false },
};

describe.each(WORKSPACE_ROLES)("the rules for an %s", (actor) => {
  it("in the workspace", () => {
    expect(manages(actor)).toBe(MANAGES[actor]);
    expect(seesTeamspaces(actor)).toBe(actor !== "guest");
    expect([mayCreateTeamspace(actor, true), mayCreateTeamspace(actor, false)]).toEqual(
      CREATES[actor],
    );
  });

  it.each(WORKSPACE_ROLES)("toward a member who is %s", (target) => {
    expect(assignableRoles(actor, target)).toEqual(CHANGES[actor][target]);
    expect(mayRemove(actor, target, false)).toBe(REMOVES[actor][target]);
    expect(mayTransfer(actor, target, false)).toBe(TRANSFERS[actor][target]);
  });

  it("toward themselves", () => {
    // Change their own role: owners to any, admins to all but owner, others not at all.
    expect(assignableRoles(actor, actor)).toEqual(CHANGES[actor][actor]);
    // Nobody removes themselves, nor transfers ownership to themselves.
    expect(mayRemove(actor, actor, true)).toBe(false);
    expect(mayTransfer(actor, actor, true)).toBe(false);
  });
});

/** What the pages offer someone toward a teamspace they see. */
type Standing = { entry: TeamspaceEntry; manages: boolean; leaves: boolean };

/**
 * Teamspaces, for the roles that join them: someone in it is in, and leaves it
 * unless it is the default one; its owners and the workspace's owners and
 * admins manage it. Outside it, an open one is joined at will, a closed one by
 * the workspace's owners and admins, others asking its owners. Undefined where
 * the pages show no teamspace: a private one is hidden to everyone outside it,
 * but for the name owners and admins see, apart, in the settings; and nobody
 * who joins teamspaces is outside the default one.
 */
function standing(
  role: WorkspaceRole,
  access: TeamspaceAccess,
  membership: TeamspaceRole | null,
): Standing | undefined {
  const manager = role === "owner" || role === "admin";
  if (membership !== null) {
    return {
      entry: "in",
      manages: manager || membership === "owner",
      leaves: access !== "default",
    };
  }
  switch (access) {
    case "default":
    case "private":
      return undefined;
    case "open":
      return { entry: "join", manages: manager, leaves: false };
    case "closed":
      return { entry: manager ? "join" : "ask", manages: manager, leaves: false };
  }
}

const TEAMSPACE_ROLES_HELD: (TeamspaceRole | null)[] = ["owner", "member", null];
const ACCESSES: TeamspaceAccess[] = ["default", "open", "closed", "private"];

describe.each(["owner", "admin", "member"] as const)("a teamspace to a workspace %s", (role) => {
  it.each(ACCESSES)("that is %s", (access) => {
    for (const membership of TEAMSPACE_ROLES_HELD) {
      const expected = standing(role, access, membership);
      if (expected === undefined) {
        continue;
      }
      const teamspace = { access, role: membership };
      expect(
        {
          entry: teamspaceEntry(role, teamspace),
          manages: managesTeamspace(role, membership),
          leaves: mayLeaveTeamspace(teamspace),
        },
        `${role}, ${membership ?? "not in it"}`,
      ).toEqual(expected);
    }
  });
});
