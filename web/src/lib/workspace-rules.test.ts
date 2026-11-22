import { describe, expect, it } from "vitest";
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
} from "./workspace-rules";

describe("the workspace rules", () => {
  it("lets owners and admins manage, and nobody else", () => {
    expect(manages("owner")).toBe(true);
    expect(manages("admin")).toBe(true);
    expect(manages("member")).toBe(false);
    expect(manages("guest")).toBe(false);
  });

  it("lets members create teamspaces only while the workspace allows it, and guests never", () => {
    expect(mayCreateTeamspace("member", true)).toBe(true);
    expect(mayCreateTeamspace("member", false)).toBe(false);
    expect(mayCreateTeamspace("admin", false)).toBe(true);
    expect(mayCreateTeamspace("guest", true)).toBe(false);
  });

  it("leaves owners to owners", () => {
    expect(assignableRoles("owner", "owner")).toEqual(["owner", "admin", "member", "guest"]);
    expect(assignableRoles("admin", "owner")).toEqual([]);
    expect(assignableRoles("admin", "member")).toEqual(["admin", "member", "guest"]);
    expect(assignableRoles("member", "guest")).toEqual([]);
  });

  it("removes nobody who is the actor, and owners only by owners", () => {
    expect(mayRemove("owner", "owner", false)).toBe(true);
    expect(mayRemove("owner", "member", true)).toBe(false);
    expect(mayRemove("admin", "owner", false)).toBe(false);
    expect(mayRemove("admin", "guest", false)).toBe(true);
    expect(mayRemove("member", "guest", false)).toBe(false);
  });

  it("transfers ownership from an owner to someone else who is not one", () => {
    expect(mayTransfer("owner", "admin", false)).toBe(true);
    expect(mayTransfer("owner", "owner", false)).toBe(false);
    expect(mayTransfer("owner", "member", true)).toBe(false);
    expect(mayTransfer("admin", "member", false)).toBe(false);
  });

  it("says how someone gets into a teamspace", () => {
    expect(teamspaceEntry("member", { access: "open" })).toBe("join");
    expect(teamspaceEntry("member", { access: "closed" })).toBe("ask");
    expect(teamspaceEntry("admin", { access: "closed" })).toBe("join");
    expect(teamspaceEntry("member", { access: "closed", role: "member" })).toBe("in");
    expect(seesTeamspaces("guest")).toBe(false);
  });

  it("keeps everyone in the default teamspace, and teamspace owners in charge", () => {
    expect(mayLeaveTeamspace({ access: "default", role: "member" })).toBe(false);
    expect(mayLeaveTeamspace({ access: "open", role: "member" })).toBe(true);
    expect(mayLeaveTeamspace({ access: "open", role: null })).toBe(false);
    expect(managesTeamspace("member", "owner")).toBe(true);
    expect(managesTeamspace("member", "member")).toBe(false);
    expect(managesTeamspace("admin", undefined)).toBe(true);
  });
});
