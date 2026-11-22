/**
 * What the server's page rules let someone do, so pages show only the
 * controls that can work. The server decides in the end, and a refusal shows
 * like any other error.
 */
import type { PageAccess } from "@/api/pages";
import type { WorkspaceRole } from "@/api/workspaces";

/** Editing, trashing, adding pages under it and moving it all need edit access. */
export function edits(access: PageAccess): boolean {
  return access === "edit";
}

/** Those who edit a page share it, but not guests, who may not know who else is in the workspace. */
export function mayShare(role: WorkspaceRole, access: PageAccess): boolean {
  return role !== "guest" && edits(access);
}

/** Guests have no private pages. */
export function hasPrivatePages(role: WorkspaceRole): boolean {
  return role !== "guest";
}
